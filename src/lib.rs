//! Render Mermaid-style flowcharts, sequence diagrams and UI wireframes as
//! box diagrams in the terminal.
//!
//! ```
//! let graph = drawtool::parse("flowchart LR\na[build] --> b[ship]").unwrap();
//! let fit = drawtool::render_fit(&graph, graph.direction, 0);
//! assert_eq!(fit.text.lines().nth(1), Some("│ build ├─►│ ship │"));
//! ```
//!
//! [`kind`] says which kind of diagram some source is; each kind has its own
//! parser and fitter (`parse`/`render_fit`, `parse_sequence`/
//! `render_sequence_fit`, `parse_wireframe`/`render_wireframe_fit`). The
//! parsed data model is exported too, for callers that want to inspect or
//! modify a diagram before drawing it.

use std::fmt;

mod flowchart;
mod paint;
mod sequence;
mod text;
mod wireframe;

pub use flowchart::{highlight, parse, render, render_fit, wrap_labels};
pub use flowchart::{Arrow, Direction, EdgeDef, Graph, Group, Mark, NodeDef, Shape, Style};
pub use sequence::{parse_sequence, render_sequence, render_sequence_fit};
pub use sequence::{Head, Item, Message, Note, NoteSide, Participant, Sequence};
pub use text::Fit;
pub use wireframe::{parse_wireframe, render_wireframe, render_wireframe_fit};
pub use wireframe::{Align, El, Screen, Wireframe, DEFAULT_WIDTH as WIREFRAME_WIDTH, MIN_WIDTH as WIREFRAME_MIN_WIDTH};

/// The kinds of diagram drawtool draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flowchart,
    Sequence,
    Wireframe,
}

/// Which kind of diagram `text` is, from the first word of its first line
/// that isn't blank or a `%%` comment: `flowchart`/`graph`, `sequenceDiagram`
/// or `wireframe` (in any case).
///
/// # Errors
///
/// No such line, or one that starts with anything else.
pub fn kind(text: &str) -> Result<Kind, Error> {
    const START: &str = "start with `flowchart LR`, `flowchart TD`, `sequenceDiagram` or `wireframe`";
    let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l.trim()));
    let (lineno, line) = lines
        .find(|(_, l)| !l.is_empty() && !l.starts_with("%%"))
        .ok_or_else(|| Error::new(format!("no diagram: {START}")))?;
    let word = line.split_whitespace().next().unwrap_or_default().to_lowercase();
    match word.as_str() {
        "flowchart" | "graph" => Ok(Kind::Flowchart),
        "sequencediagram" => Ok(Kind::Sequence),
        "wireframe" => Ok(Kind::Wireframe),
        _ => Err(Error::at(lineno, format!("expected a header, found `{line}`; {START}"))),
    }
}

/// A problem in diagram source, with its 1-based line number when known.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Error {
    pub line: Option<usize>,
    /// 1-based character column on that line, when known.
    pub column: Option<usize>,
    pub message: String,
}

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Error { line: None, column: None, message: message.into() }
    }

    pub(crate) fn at(line: usize, message: impl Into<String>) -> Self {
        Error { line: Some(line), ..Error::new(message) }
    }

    pub(crate) fn at_column(line: usize, column: usize, message: impl Into<String>) -> Self {
        Error { column: Some(column), ..Error::at(line, message) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "line {line}, column {column}: {}", self.message),
            (Some(line), None) => write!(f, "line {line}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}
