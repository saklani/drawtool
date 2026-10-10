use drawtool::{
    diff, highlight, is_sequence, is_wireframe, marks::split_ids, parse, parse_sequence, parse_wireframe, render_fit,
    render_sequence_fit, render_wireframe_fit, Direction,
};
use std::io::Read;
use std::process::ExitCode;

const HELP: &str = "\
Render Mermaid-style flowcharts, sequence diagrams and UI wireframes in the terminal.

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
      --highlight <IDS>  draw a path thick, e.g. --highlight req,auth,db
                         (in a file: a line `%% highlight: req, auth, db`)
      --diff <BEFORE>    show how BEFORE became FILE: + added (thick),
                         - removed (dashed), ~ relabelled
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

/// Read a diagram from `path`, or stdin when `None`.
fn read_source(path: Option<&str>) -> Result<String, String> {
    match path {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}")),
        None => {
            let mut t = String::new();
            std::io::stdin().read_to_string(&mut t).map_err(|e| format!("reading stdin: {e}"))?;
            Ok(t)
        }
    }
}

fn main() -> ExitCode {
    let mut ascii = false;
    let mut direction = None;
    let mut file: Option<String> = None;
    let mut max_width = default_width();
    let mut path: Option<Vec<String>> = None;
    let mut before: Option<String> = None;
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
            "--highlight" => {
                let Some(v) = inline.or_else(|| args.next()) else {
                    return fail("--highlight needs node ids, e.g. --highlight a,b,c");
                };
                path = Some(split_ids(&v));
            }
            "--diff" => {
                let Some(v) = inline.or_else(|| args.next()) else {
                    return fail("--diff needs the BEFORE file");
                };
                before = Some(v);
            }
            "-" => file = None,
            f if f.starts_with('-') => return fail(&format!("unknown option {f} (see --help)")),
            _ => file = Some(arg),
        }
    }

    let text = match read_source(file.as_deref()) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    if is_wireframe(&text) {
        if before.is_some() || path.is_some() {
            return fail("--diff and --highlight only work on flowcharts");
        }
        return match parse_wireframe(&text) {
            Ok(wf) => {
                let fitted = render_wireframe_fit(&wf, ascii, max_width);
                if !fitted.fits {
                    eprintln!("drawtool: note: {} columns wide, over the {max_width}-column budget", fitted.width);
                }
                println!("{}", fitted.text);
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }
    if is_sequence(&text) {
        if before.is_some() || path.is_some() {
            return fail("--diff and --highlight only work on flowcharts");
        }
        return match parse_sequence(&text) {
            Ok(seq) => {
                let fitted = render_sequence_fit(&seq, ascii, max_width);
                if !fitted.fits {
                    eprintln!("drawtool: note: {} columns wide, over the {max_width}-column budget", fitted.width);
                }
                println!("{}", fitted.text);
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }
    let mut graph = match parse(&text) {
        Ok(g) => g,
        Err(e) => return fail(&e),
    };
    if let Some(before) = before {
        let old = match read_source(Some(&before)).and_then(|t| parse(&t).map_err(|e| format!("{before}: {e}"))) {
            Ok(g) => g,
            Err(e) => return fail(&e),
        };
        graph = diff(&old, &graph);
    }
    if let Some(path) = path {
        if let Err(e) = highlight(&mut graph, &path) {
            return fail(&e);
        }
    }
    let wanted = direction.unwrap_or(graph.direction);
    let fitted = render_fit(&graph, wanted, ascii, max_width);
    if !fitted.fits {
        eprintln!("drawtool: note: {} columns wide, over the {max_width}-column budget", fitted.width);
    }
    println!("{}", fitted.text);
    ExitCode::SUCCESS
}
