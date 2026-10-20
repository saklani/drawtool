//! Render Mermaid diagrams as box diagrams in the terminal.
//!
//! ```
//! let drawtool::Diagram::Flowchart(graph) = drawtool::parse("flowchart LR\na[build] --> b[ship]").unwrap() else {
//!     panic!()
//! };
//! let fit = drawtool::render_fit(&graph, graph.direction, 0);
//! assert_eq!(fit.text.lines().nth(1), Some("│ build ├─►│ ship │"));
//! ```
//!
//! [`parse`] reads any diagram drawtool draws; each kind has its own fitter
//! (`render_fit`, `render_sequence_fit`). The parsed data model is exported
//! too, for callers that want to inspect or modify a diagram before drawing it.

use std::fmt;

mod flowchart;
mod mermaid;
mod paint;
mod sequence;
mod text;

pub use flowchart::{highlight, render, render_fit, wrap_labels};
pub use flowchart::{Arrow, Direction, EdgeDef, Graph, Group, Mark, NodeDef, Shape, Style};
pub use sequence::{render_sequence, render_sequence_fit};
pub use sequence::{Head, Item, Message, Note, NoteSide, Participant, Sequence};
pub use text::Fit;

/// A parsed diagram, by how drawtool draws it.
#[derive(Debug, Clone)]
pub enum Diagram {
    /// A Mermaid flowchart or state diagram: boxes and arrows.
    Flowchart(Graph),
    /// A Mermaid sequence diagram.
    Sequence(Sequence),
}

/// Parse Mermaid source; Mermaid decides its kind.
///
/// # Errors
///
/// What Mermaid rejects, at the line and column it names; a kind of diagram
/// drawtool doesn't draw yet; or what drawtool can't draw within a kind.
pub fn parse(text: &str) -> Result<Diagram, Error> {
    match mermaid::parse(text)? {
        ("flowchart", m) => flowchart::flowchart(&m, text).map(Diagram::Flowchart),
        ("state", m) => flowchart::state(&m, text).map(Diagram::Flowchart),
        ("sequence", m) => Ok(Diagram::Sequence(sequence::sequence(&m))),
        (t, _) => Err(Error::new(format!("`{t}` diagrams aren't supported yet"))),
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
