//! Mermaid source to Ondine's parsed model: drawtool parses standard Mermaid
//! with Ondine's port of Mermaid's grammar and only converts what it gets.

use crate::Error;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|\n").unwrap());
/// Mermaid hides `#name;` entity codes behind placeholder characters while parsing.
static NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(\d+);").unwrap());
static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new("\u{fb02}\u{b0}(\u{b0})?(\\w+)\u{b6}\u{df}").unwrap());

/// An array of Ondine's model, or none.
pub(crate) fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

/// A field of Ondine's model as text, or empty.
pub(crate) fn get<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// Parse `text` as Mermaid.
///
/// # Errors
///
/// What Mermaid itself rejects, at the line and column it names.
/// The diagram's type and model: Mermaid's own database, as JSON.
pub(crate) fn parse(text: &str) -> Result<(&'static str, Value), Error> {
    let m = compiler::model(text, "", 0.0).map_err(|e| error(text, e))?;
    Ok((m.r#type, serde_json::from_str(&m.json).expect("Ondine's model is JSON")))
}

fn error(text: &str, e: compiler::Error) -> Error {
    match (e.code, e.r#type) {
        (compiler::Code::UnsupportedDiagram, Some(t)) => Error::new(format!("`{t}` diagrams aren't supported yet")),
        (compiler::Code::UnsupportedDiagram, None) => Error::new(
            "no diagram: the first line names it: `flowchart LR`, `stateDiagram-v2` or `sequenceDiagram`",
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
