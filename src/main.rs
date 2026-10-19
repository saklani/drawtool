use clap::Parser;
use drawtool::{highlight, parse, Diagram, Direction, Fit};
use drawtool::{render_fit, render_sequence_fit, render_wireframe_fit};
use std::io::{IsTerminal, Read};
use std::process::ExitCode;

const SYNTAX: &str = "\
The diagram comes on stdin, usually a quoted heredoc: drawtool <<'EOF' ... EOF

Syntax (a subset of Mermaid flowcharts):
  flowchart LR              %% LR = left-to-right (default), TD = top-down
  req[HTTP request] --> parse(parse_body)
  parse --> ok{valid?}
  ok -->|yes| save[(db.insert)]
  ok -- no --> err[400 Bad Request]
  save -.-> cache[cache.invalidate]     %% dotted edge
  save ==> resp[201 Created]            %% thick edge
  a --> b & c                           %% fan-out (also: a & b --> c)
  x --- y                               %% edge without arrowhead
  x <--> y                              %% arrowheads at both ends (also <-.-> <==>)
  n[two<br>lines]                        %% multi-line label
  a[x] --> b; b --> c                   %% `;` separates statements
  subgraph api [API server]             %% dashed group frame
    mw --> route
  end
  %% stack: save cache

`%% stack: a b c` draws those boxes in one column, in that order (in TD, one row),
joined by short straight arrows: side effects under the step that causes them, or a
pair that talks both ways. Mermaid reads it as a comment.

Shapes: [box]  (rounded)  {decision}  ([stadium])  [(database)]  [[sub]]
Other Mermaid shapes keep their label and are drawn as the nearest of these.
Any valid input here is valid Mermaid and means the same; RL/BT, --o, --x,
~~~ and edges to a subgraph are rejected. The header is required.

Sequence diagrams (start the input with `sequenceDiagram`; -d is ignored):
  sequenceDiagram
  %% or: actor user (a %% comment goes on its own line here)
  participant api as API server
  %% ->> solid, -->> dotted reply, -> / --> no head, -x cross end
  user->>api: POST /orders
  api-->>user: 201 Created
  api->>api: validate
  %% also: Note left of / right of X
  Note over user,api: text
  loop every 5m / alt x / else y / opt / par / and ... end

Wireframes (start with `wireframe [title] [width=N]`, indent children):
  header: ← Back | Title | ?    input: Email = you@x.com   button: Continue
  card Title:  row:  col:       tabs: *A | B   nav: *Home | Pay
  - item | $4.50                toggle: Wi-Fi = on   checkbox: x = on   radio: *a | b
";

/// Render Mermaid-style flowcharts, sequence diagrams and UI wireframes in the terminal.
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
