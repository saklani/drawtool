use clap::Parser;
use drawtool::{render_source, Direction, Options};
use std::io::Read;
use std::process::ExitCode;

const SYNTAX: &str = "\
Syntax (a subset of Mermaid flowcharts):
  flowchart LR              # LR = left-to-right (default), TD = top-down
  req[HTTP request] --> parse(parse_body)
  parse --> ok{valid?}
  ok -->|yes| save[(db.insert)]
  ok -- no --> err[400 Bad Request]
  save -.-> cache[cache.invalidate]     %% dotted edge
  save ==> resp[201 Created]            %% thick edge
  a --> b & c                           %% fan-out (also: a & b --> c)
  x --- y                               %% edge without arrowhead
  n[two\\nlines]                         %% multi-line label (\\n or <br>)
  subgraph api [API server]             %% dashed group frame
    mw --> route
  end

Shapes: [box]  (rounded)  {decision}  ([stadium])  [(database)]  [[sub]]

Sequence diagrams (start the input with `sequenceDiagram`; -d is ignored):
  sequenceDiagram
  participant api as API server         %% or: actor user
  user->>api: POST /orders              %% ->> solid, -->> dotted reply
  api-->>user: 201 Created              %% -> / --> no head, -x cross end
  api->>api: validate                   %% self-message
  Note over user,api: text              %% also: Note left of / right of X
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
    /// Diagram source file (default: stdin)
    file: Option<String>,
    /// Plain ASCII output
    #[arg(short, long)]
    ascii: bool,
    /// Override layout direction: TD or LR
    #[arg(short, long, value_parser = direction)]
    direction: Option<Direction>,
    /// Column budget (default: $COLUMNS, else the terminal's width, else 120; 0 = unlimited).
    /// Too-wide diagrams get their labels wrapped; LR ones are then broken into
    /// bands. The direction never changes on its own.
    #[arg(short = 'w', long, value_name = "N")]
    max_width: Option<usize>,
    /// Draw a path thick, e.g. --highlight req,auth,db (in a file: `%% highlight: req, auth, db`)
    #[arg(long, value_name = "IDS")]
    highlight: Option<String>,
    /// Show how BEFORE became FILE: + added (thick), - removed (dashed), ~ relabelled
    #[arg(long, value_name = "BEFORE")]
    diff: Option<String>,
}

fn direction(s: &str) -> Result<Direction, String> {
    match s.to_uppercase().as_str() {
        "TD" | "TB" => Ok(Direction::TD),
        "LR" => Ok(Direction::LR),
        _ => Err("use TD or LR".to_string()),
    }
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

/// Read a diagram from `path`, or stdin when `None` or `-`.
fn read_source(path: Option<&str>) -> Result<String, String> {
    if let Some(path) = path.filter(|p| *p != "-") {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
    } else {
        let mut t = String::new();
        std::io::stdin().read_to_string(&mut t).map_err(|e| format!("reading stdin: {e}"))?;
        Ok(t)
    }
}

/// Read the input (and the `--diff` file) and render it.
fn run(args: &Args, max_width: usize) -> Result<drawtool::Fit, Box<dyn std::error::Error>> {
    let text = read_source(args.file.as_deref())?;
    let before = args.diff.as_deref().map(|path| read_source(Some(path))).transpose()?;
    let highlight = args.highlight.iter().flat_map(|ids| ids.split([',', ' '])).filter(|id| !id.is_empty());
    let opts = Options {
        ascii: args.ascii,
        max_width,
        direction: args.direction,
        highlight: highlight.map(String::from).collect(),
        before,
    };
    Ok(render_source(&text, &opts)?)
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
        Err(e) => {
            eprintln!("drawtool: {e}");
            ExitCode::from(2)
        }
    }
}
