use drawtool::{parse, render_fit, Direction};
use std::io::Read;
use std::process::ExitCode;

const HELP: &str = "\
Render Mermaid-style flowcharts in the terminal.

Usage: drawtool [OPTIONS] [FILE]

Arguments:
  [FILE]  diagram source file (default: stdin)

Options:
  -a, --ascii            plain ASCII output
  -d, --direction <DIR>  override layout direction: TD or LR
  -w, --max-width <N>    column budget (default: $COLUMNS, else the terminal's
                         width, else 120; 0 = unlimited).
                         Too-wide diagrams get their labels wrapped; LR ones are
                         then broken into bands joined by numbered connectors.
                         The direction never changes on its own.
  -h, --help             print help
  -V, --version          print version

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

Shapes: [box]  (rounded)  {decision}  ([stadium])  [(database)]  [[sub]]
";

fn fail(msg: &str) -> ExitCode {
    eprintln!("drawtool: {msg}");
    ExitCode::from(2)
}

/// `$COLUMNS`, else the width of whichever of stdout/stderr is a terminal
/// (so `drawtool | less` still fits the window), else 120.
fn default_width() -> usize {
    if let Some(n) = std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()) {
        return n;
    }
    let tty = terminal_size::terminal_size_of(std::io::stdout())
        .or_else(|| terminal_size::terminal_size_of(std::io::stderr()));
    match tty {
        Some((terminal_size::Width(w), _)) => w as usize,
        None => 120,
    }
}

fn main() -> ExitCode {
    let mut ascii = false;
    let mut direction = None;
    let mut file: Option<String> = None;
    let mut max_width = default_width();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("drawtool {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-a" | "--ascii" => ascii = true,
            "-d" | "--direction" => {
                let Some(v) = inline.or_else(|| args.next()) else {
                    return fail("--direction needs a value (TD or LR)");
                };
                direction = match v.to_uppercase().as_str() {
                    "TD" | "TB" => Some(Direction::TD),
                    "LR" => Some(Direction::LR),
                    _ => return fail(&format!("invalid direction {v:?} (use TD or LR)")),
                };
            }
            "-w" | "--max-width" => {
                let Some(v) = inline.or_else(|| args.next()) else {
                    return fail("--max-width needs a number");
                };
                let Ok(n) = v.parse() else {
                    return fail(&format!("invalid --max-width {v:?}"));
                };
                max_width = n;
            }
            "-" => file = None,
            f if f.starts_with('-') => return fail(&format!("unknown option {f} (see --help)")),
            _ => file = Some(arg),
        }
    }

    let text = match &file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return fail(&format!("{path}: {e}")),
        },
        None => {
            let mut t = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut t) {
                return fail(&format!("reading stdin: {e}"));
            }
            t
        }
    };
    match parse(&text) {
        Ok(graph) => {
            let wanted = direction.unwrap_or(graph.direction);
            let fitted = render_fit(&graph, wanted, ascii, max_width);
            if !fitted.fits {
                eprintln!("drawtool: note: {} columns wide, over the {max_width}-column budget", fitted.width);
            }
            println!("{}", fitted.text);
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}
