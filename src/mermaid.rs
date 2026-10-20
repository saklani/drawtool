//! Mermaid source to Ondine's parsed model: drawtool parses standard Mermaid
//! with Ondine's port of Mermaid's grammar and only converts what it gets.

use crate::Error;
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|\n").unwrap());
/// Mermaid hides `#name;` entity codes behind placeholder characters while parsing.
static NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(\d+);").unwrap());
static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new("\u{fb02}\u{b0}(\u{b0})?(\\w+)\u{b6}\u{df}").unwrap());

/// A parsed diagram, as Mermaid's own database holds it.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum Model {
    Flowchart(Flowchart),
    Sequence(Sequence),
    State(State),
}

#[derive(Deserialize)]
pub(crate) struct Flowchart {
    pub direction: Option<String>,
    pub nodes: IndexMap<String, FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub subgraphs: Vec<Subgraph>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlowNode {
    pub id: String,
    pub text: Option<String>,
    pub label_type: String,
    #[serde(rename = "type")]
    pub shape: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlowEdge {
    pub start: String,
    pub end: String,
    /// `arrow_point`, `double_arrow_point`, `arrow_open`, ...
    #[serde(rename = "type")]
    pub arrow: Option<String>,
    pub text: String,
    pub label_type: String,
    /// `normal`, `thick`, `dotted` or `invisible`.
    pub stroke: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Subgraph {
    pub id: String,
    pub nodes: Vec<String>,
    pub title: String,
}

#[derive(Deserialize)]
pub(crate) struct Sequence {
    pub actors: Vec<Actor>,
    pub messages: Vec<SeqMessage>,
}

#[derive(Deserialize)]
pub(crate) struct Actor {
    pub id: String,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Deserialize)]
pub(crate) struct SeqMessage {
    pub from: Option<String>,
    pub to: Option<String>,
    /// The text, or an `autonumber` object.
    pub message: serde_json::Value,
    /// Mermaid's `LINETYPE`.
    #[serde(rename = "type")]
    pub kind: Option<i32>,
    pub placement: Option<i32>,
}

#[derive(Deserialize)]
pub(crate) struct State {
    pub direction: String,
    pub graph: StateGraph,
}

#[derive(Deserialize)]
pub(crate) struct StateGraph {
    pub nodes: Vec<StateNode>,
    pub edges: Vec<StateEdge>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StateNode {
    pub id: String,
    pub label: Option<String>,
    pub description: Vec<String>,
    pub is_group: bool,
    pub parent_id: Option<String>,
    pub shape: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct StateEdge {
    pub start: String,
    pub end: String,
    pub label: String,
    /// `dashed` on the line from a note to its state.
    pub pattern: Option<String>,
}

/// Parse `text` as Mermaid.
///
/// # Errors
///
/// What Mermaid itself rejects, at the line and column it names.
pub(crate) fn parse(text: &str) -> Result<Model, Error> {
    let m = compiler::model(text, "", 0.0).map_err(|e| error(text, e))?;
    match m.r#type {
        "flowchart" | "state" | "sequence" => {
            Ok(serde_json::from_str(&m.json).expect("Ondine's model JSON matches Mermaid's"))
        }
        t => Err(Error::new(format!("`{t}` diagrams aren't supported yet"))),
    }
}

fn error(text: &str, e: compiler::Error) -> Error {
    match (e.code, e.r#type) {
        (compiler::Code::UnsupportedDiagram, Some(t)) => Error::new(format!("`{t}` diagrams aren't supported yet")),
        (compiler::Code::UnsupportedDiagram, None) => Error::new(
            "no diagram: the first line names it: `flowchart LR`, `stateDiagram-v2`, `sequenceDiagram` or `wireframe`",
        ),
        _ if e.line == 0 => Error::new(e.message),
        _ => {
            // The message repeats the line and its snippet, which drawtool prints itself.
            let reason = e.message.lines().last().unwrap_or_default();
            let reason = if e.message.starts_with("Lexical error") { "unrecognized text" } else { reason };
            let eof = reason.ends_with("got 'EOF'") || reason.ends_with("got '$end'");
            if eof {
                if let Some((line, word)) = unclosed(text) {
                    let close = if word == "state" { "}" } else { "end" };
                    return Error::at(line, format!("`{word}` is never closed with `{close}`"));
                }
            }
            // A long list of what the grammar expected says less than what it got.
            let reason = match reason.rsplit_once(", got ") {
                Some((expected, _)) if eof && expected.matches(',').count() >= 3 => "unexpected end of input".into(),
                Some((_, "'end'")) => "`end` without an open block".into(),
                Some((expected, got)) if expected.matches(',').count() >= 3 => format!("unexpected {got}"),
                _ => reason.to_string(),
            };
            // Mermaid points past the end: put it on the last line of text.
            let line = if eof { text.trim_end().lines().count().max(1) } else { e.line as usize };
            let column = if eof { text.trim_end().lines().last().unwrap_or_default().chars().count() + 1 } else { e.column as usize };
            Error::at_column(line, column, reason)
        }
    }
}

/// The innermost block still open at the end of `text`, as (line, keyword):
/// Mermaid only says it ran out of input.
fn unclosed(text: &str) -> Option<(usize, String)> {
    const OPENERS: &[&str] = &["loop", "alt", "opt", "par", "critical", "break", "rect", "box", "subgraph"];
    let mut open = Vec::new();
    for (i, line) in text.lines().map(str::trim).enumerate() {
        let word = line.split_whitespace().next().unwrap_or_default();
        if line == "end" || line == "}" {
            open.pop();
        } else if line.contains(['-', '=', '~']) {
            // A line with an arrow is an edge or message, even from a node called `loop`.
        } else if OPENERS.contains(&word) || (word == "state" && line.ends_with('{')) {
            open.push((i + 1, word.to_string()));
        }
    }
    open.pop()
}

/// A label as drawtool draws it: HTML entities decoded, markdown emphasis
/// dropped, and split into lines at `<br>` and newlines.
pub(crate) fn label(text: &str, markdown: bool) -> Vec<String> {
    let t = ENTITY.replace_all(text, |c: &regex::Captures| if c.get(1).is_some() { format!("&#{};", &c[2]) } else { format!("&{};", &c[2]) });
    let mut t = t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    t = NUMERIC.replace_all(&t, |c: &regex::Captures| c[1].parse().ok().and_then(char::from_u32).map(String::from).unwrap_or_default()).into_owned();
    t = t.replace("&amp;", "&");
    if markdown {
        t = t.replace("**", "").replace("__", "");
    }
    BR.split(&t).map(|l| l.trim().to_string()).collect()
}
