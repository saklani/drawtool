//! Mermaid source to merman-core's typed model: drawtool parses standard
//! Mermaid with Mermaid's own grammar and only converts what it gets.

use crate::Error;
use merman_core::{Engine, ParseDiagnosticSpanKind, ParseOptions, RenderSemanticModel};
use regex::Regex;
use std::sync::LazyLock;

static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|\n").unwrap());

/// Parse `text` as Mermaid.
///
/// # Errors
///
/// What Mermaid itself rejects, at the line and column it names.
pub(crate) fn parse(text: &str) -> Result<RenderSemanticModel, Error> {
    let parsed = Engine::new().parse_diagram_for_render_model_sync(text, ParseOptions::strict());
    match parsed {
        Ok(Some(p)) => Ok(p.into_parts().1),
        Ok(None) | Err(merman_core::Error::DetectType(_)) => Err(Error::new(
            "no diagram: the first line names it: `flowchart LR`, `stateDiagram-v2`, `sequenceDiagram` or `wireframe`",
        )),
        Err(merman_core::Error::UnsupportedDiagram { diagram_type }) => {
            Err(Error::new(format!("drawtool doesn't draw `{diagram_type}` diagrams yet")))
        }
        Err(merman_core::Error::DiagramParse { diagnostic: d, .. }) => {
            // "Expected …" at the very end points past trailing blank lines: put it after the last text.
            let insert = d.span_kind() == ParseDiagnosticSpanKind::InsertionPoint;
            let offset = d.span().map(|s| if insert { text[..s.start.min(text.len())].trim_end().len() } else { s.start });
            Err(at(text, offset, d.message().to_string()))
        }
        Err(e) => Err(Error::new(e.to_string())),
    }
}

/// An error at byte `offset` of `text`, as a 1-based line and character column.
fn at(text: &str, offset: Option<usize>, message: String) -> Error {
    let Some(offset) = offset.map(|o| o.min(text.len())) else { return Error::new(message) };
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
    Error::at_column(line, column, message)
}

/// A label as drawtool draws it: HTML entities decoded, markdown emphasis
/// dropped, and split into lines at `<br>` and newlines.
pub(crate) fn label(text: &str, markdown: bool) -> Vec<String> {
    let mut t = text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    t = t.replace("&amp;", "&");
    if markdown {
        t = t.replace("**", "").replace("__", "");
    }
    BR.split(&t).map(|l| l.trim().to_string()).collect()
}
