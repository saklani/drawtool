//! Render Mermaid-style flowcharts as box diagrams in the terminal.

pub mod layout;
pub mod parse;

pub use layout::render;
pub use parse::{parse, Direction, Graph};
