//! Render Mermaid-style flowcharts and sequence diagrams as box diagrams in the terminal.

pub mod fit;
pub mod layout;
pub mod parse;
pub mod sequence;

pub use fit::{render_fit, wrap_labels, Fitted};
pub use layout::render;
pub use parse::{parse, Direction, Graph};
pub use sequence::{is_sequence, parse_sequence, render_sequence, render_sequence_fit, Sequence};
