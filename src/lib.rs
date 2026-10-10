//! Render Mermaid-style flowcharts, sequence diagrams and UI wireframes in the terminal.

use std::fmt;

pub mod fit;
pub mod layout;
pub mod marks;
pub mod paint;
pub mod parse;
pub mod sequence;
pub mod wireframe;

pub use fit::{render_fit, wrap_labels};
pub use layout::render;
pub use marks::{diff, highlight};
pub use paint::Fit;
pub use parse::{parse, Direction, Graph, Mark};
pub use sequence::{is_sequence, parse_sequence, render_sequence, render_sequence_fit, Sequence};
pub use wireframe::{is_wireframe, parse_wireframe, render_wireframe, render_wireframe_fit, Wireframe};

/// A problem in diagram source, with its 1-based line number when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub line: Option<usize>,
    pub message: String,
}

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Error { line: None, message: message.into() }
    }

    pub(crate) fn at(line: usize, message: impl Into<String>) -> Self {
        Error { line: Some(line), message: message.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}
