//! Render Mermaid-style flowcharts, sequence diagrams and UI wireframes in the terminal.

pub mod fit;
pub mod layout;
pub mod marks;
pub mod paint;
pub mod parse;
pub mod sequence;
pub mod wireframe;

pub use fit::{render_fit, wrap_labels, Fitted};
pub use layout::render;
pub use marks::{diff, highlight};
pub use parse::{parse, Direction, Graph, Mark};
pub use sequence::{is_sequence, parse_sequence, render_sequence, render_sequence_fit, Sequence};
pub use wireframe::{is_wireframe, parse_wireframe, render_wireframe, render_wireframe_fit, Wireframe};
