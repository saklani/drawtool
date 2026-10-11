//! Render Mermaid-style flowcharts, sequence diagrams and UI wireframes as
//! box diagrams in the terminal.
//!
//! ```
//! let fit = drawtool::render_source("flowchart LR\na[build] --> b[ship]", &drawtool::Options::default()).unwrap();
//! assert_eq!(fit.text.lines().nth(1), Some("│ build ├─►│ ship │"));
//! ```
//!
//! [`render_source`] detects the kind of diagram. The per-kind functions
//! (`parse`/`render_fit`, `parse_sequence`/`render_sequence_fit`, ...) and
//! the parsed data model are exported too, for callers that want to inspect
//! or modify a diagram before drawing it.

use std::fmt;

mod fit;
mod layout;
mod marks;
mod model;
mod paint;
mod parse;
mod sequence;
mod text;
mod wireframe;

pub use fit::{render_fit, wrap_labels};
pub use layout::render;
pub use marks::highlight;
pub use model::{Arrow, Direction, EdgeDef, Graph, Group, Mark, NodeDef, Shape, Style};
pub use parse::parse;
pub use sequence::{is_sequence, parse_sequence, render_sequence, render_sequence_fit};
pub use sequence::{Head, Item, Message, Note, NoteSide, Participant, Sequence};
pub use text::Fit;
pub use wireframe::{is_wireframe, parse_wireframe, render_wireframe, render_wireframe_fit};
pub use wireframe::{Align, El, Screen, Wireframe, DEFAULT_WIDTH as WIREFRAME_WIDTH, MIN_WIDTH as WIREFRAME_MIN_WIDTH};

/// How to render diagram source with [`render_source`].
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Column budget; 0 means unlimited.
    pub max_width: usize,
    /// Override a flowchart's own direction.
    pub direction: Option<Direction>,
    /// Flowcharts: draw this path of node ids thick.
    pub highlight: Vec<String>,
}

/// Render a flowchart, `sequenceDiagram` or `wireframe`, whichever `text` is.
///
/// # Errors
///
/// Source that doesn't parse, a highlight path that doesn't exist, or
/// highlighting a diagram that isn't a flowchart.
pub fn render_source(text: &str, opts: &Options) -> Result<Fit, Error> {
    let (wireframe, sequence) = (is_wireframe(text), is_sequence(text));
    if !opts.highlight.is_empty() && (wireframe || sequence) {
        return Err(Error::new("highlighting only works on flowcharts"));
    }
    if wireframe {
        return Ok(render_wireframe_fit(&parse_wireframe(text)?, opts.max_width));
    }
    if sequence {
        return render_sequence_fit(&parse_sequence(text)?, opts.max_width);
    }
    let mut graph = parse(text)?;
    if !opts.highlight.is_empty() {
        highlight(&mut graph, &opts.highlight)?;
    }
    Ok(render_fit(&graph, opts.direction.unwrap_or(graph.direction), opts.max_width))
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
