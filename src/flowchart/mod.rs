//! Flowcharts: parse Mermaid-style source into a graph, mark a highlighted
//! path, lay it out, and fit it into a column budget.

mod fit;
mod layout;
mod marks;
mod model;
mod parse;

pub use fit::{render_fit, wrap_labels};
pub use layout::render;
pub use marks::highlight;
pub use model::{Arrow, Direction, EdgeDef, Graph, Group, Mark, NodeDef, Shape, Style};
pub use parse::parse;
