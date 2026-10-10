//! Render Mermaid-style flowcharts as box diagrams in the terminal.

pub mod fit;
pub mod layout;
pub mod marks;
pub mod parse;

pub use fit::{render_fit, wrap_labels, Fitted};
pub use layout::render;
pub use marks::{diff, highlight};
pub use parse::{parse, Direction, Graph, Mark};
