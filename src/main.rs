use clap::Parser;
use drawtool::{highlight, parse, Diagram, Direction, Fit};
use drawtool::{render_fit, render_sequence_fit, render_wireframe_fit};
use std::io::{IsTerminal, Read};
use std::process::ExitCode;

const SYNTAX: &str = "\
The diagram comes on stdin, usually a quoted heredoc: drawtool <<'EOF' ... EOF

Input is standard Mermaid, read by Mermaid's own grammar. drawtool draws:
  flowchart / graph    LR (left-to-right) or TD
  stateDiagram-v2      as a flowchart; add `direction LR` for left-to-right
  sequenceDiagram      (-d is ignored)
RL and BT are drawn as LR and TD. Not drawn yet: other kinds of diagram and edges
to a subgraph. Shapes are
drawn as the nearest of box, rounded, decision and database. A %% comment goes
on its own line.

Two comment lines drawtool reads (Mermaid ignores them):
  flowchart LR
  req[request] --> auth{token ok?}
  auth -->|yes| save[(db.insert)]
  auth -->|no| err[401]
  save -.-> cache[cache.invalidate]
  %% highlight: req, auth, save
  %% stack: save cache
`%% highlight: a, b, c` draws that path thick (or pass --highlight a,b,c).
`%% stack: a b c` draws those boxes in one column, in that order (in TD, one row),
joined by short straight arrows: side effects under the step that causes them, or
a pair that talks both ways.

Wireframes are drawtool's own, not Mermaid (start with `wireframe [title] [width=N]`,
indent children):
  header: ← Back | Title | ?    input: Email = you@x.com   button: Continue
  card Title:  row:  col:       tabs: *A | B   nav: *Home | Pay
  - item | $4.50                toggle: Wi-Fi = on   checkbox: x = on   radio: *a | b
";

/// Render Mermaid flowcharts, state and sequence diagrams, and UI wireframes in the terminal.
#[derive(Parser)]
#[command(version, after_help = SYNTAX)]
struct Args {
    /// Override layout direction: TD or LR
    #[arg(short, long)]
    direction: Option<Direction>,
    /// Column budget (default: $COLUMNS, else the terminal's width, else 120; 0 = unlimited).
    /// Too-wide diagrams get their labels wrapped; LR ones are then broken into
    /// rows. The direction never changes on its own.
    #[arg(short = 'w', long, value_name = "N")]
    max_width: Option<usize>,
    /// Draw a path thick, e.g. --highlight req,auth,db (in a file: `%% highlight: req, auth, db`)
    #[arg(long, value_name = "IDS", value_delimiter = ',')]
    highlight: Vec<String>,
}

/// `$COLUMNS`, else the width of whichever of stdout/stderr is a terminal
/// (so `drawtool | less` still fits the window), else 120.
fn default_width() -> usize {
    let tty = || {
        terminal_size::terminal_size_of(std::io::stdout())
            .or_else(|| terminal_size::terminal_size_of(std::io::stderr()))
    };
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .or_else(|| tty().map(|(w, _)| w.0 as usize))
        .unwrap_or(120)
}

/// The diagram source, from stdin (a heredoc).
fn read_stdin() -> Result<String, String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err("no input: pipe the diagram in on stdin (see --help)".to_string());
    }
    let mut t = String::new();
    stdin.read_to_string(&mut t).map_err(|e| format!("reading stdin: {e}"))?;
    if t.trim().is_empty() {
        return Err("no diagram on stdin".to_string());
    }
    Ok(t)
}

/// Read the diagram from stdin, parse it, and fit it to `max_width`.
fn run(args: &Args, max_width: usize) -> Result<Fit, Box<dyn std::error::Error>> {
    let diagram = parse(&read_stdin()?)?;
    if !args.highlight.is_empty() && !matches!(diagram, Diagram::Flowchart(_)) {
        return Err("highlighting only works on flowcharts and state diagrams".into());
    }
    Ok(match diagram {
        Diagram::Wireframe(w) => render_wireframe_fit(&w, max_width),
        Diagram::Sequence(s) => render_sequence_fit(&s, max_width)?,
        Diagram::Flowchart(mut graph) => {
            highlight(&mut graph, &args.highlight)?;
            render_fit(&graph, args.direction.unwrap_or(graph.direction), max_width)
        }
    })
}

fn main() -> ExitCode {
    let args = Args::parse();
    let budget = args.max_width.unwrap_or_else(default_width);
    match run(&args, budget) {
        Ok(fit) => {
            if !fit.fits {
                eprintln!("drawtool: note: {} columns wide, over the {budget}-column budget", fit.width);
            }
            println!("{}", fit.text);
            ExitCode::SUCCESS
        }
        // clap exits with 2 for usage errors; failures at run time get 1.
        Err(e) => {
            eprintln!("drawtool: {e}");
            ExitCode::FAILURE
        }
    }
}
