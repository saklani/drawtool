use drawtool::{parse, render, Direction};
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
  -h, --help             print help
  -V, --version          print version

Syntax (a subset of Mermaid flowcharts):
  flowchart TD              # TD = top-down (default), LR = left-to-right
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

fn main() -> ExitCode {
    let mut ascii = false;
    let mut direction = None;
    let mut file: Option<String> = None;
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
            println!("{}", render(&graph, direction, ascii));
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}
