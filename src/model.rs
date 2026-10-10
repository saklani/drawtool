//! The parsed flowchart: what every stage after parsing works on.

use petgraph::graph::{DiGraph, NodeIndex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    TD,
    LR,
}

impl std::str::FromStr for Direction {
    type Err = String;

    /// `TD`/`TB` or `LR`, in any case.
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_uppercase().as_str() {
            "TD" | "TB" => Ok(Direction::TD),
            "LR" => Ok(Direction::LR),
            _ => Err(format!("unknown direction {s:?} (use TD or LR)")),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Box,
    /// `[(name)]`: drawn with double caps, `╒═╕`.
    Database,
    Round,
    Decision,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Solid,
    Dotted,
    Thick,
}

/// How a node is emphasised: by `--highlight`, or by `--diff`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mark {
    #[default]
    None,
    Highlight,
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug)]
pub struct NodeDef {
    pub id: String,
    pub lines: Vec<String>,
    pub shape: Shape,
    pub mark: Mark,
    /// Index into `Graph::groups` of the subgraph the node belongs to: of
    /// those that mention it, the first to close (so the innermost).
    pub group: Option<usize>,
}

impl NodeDef {
    #[must_use]
    pub fn new(id: &str, lines: Vec<String>, shape: Shape) -> Self {
        NodeDef { id: id.to_string(), lines, shape, mark: Mark::None, group: None }
    }
}

/// An arrow's own data; its ends are the graph's.
#[derive(Clone, Debug)]
pub struct EdgeDef {
    pub label: Option<Vec<String>>,
    pub style: Style,
    pub arrow: bool,
}

/// A `subgraph` block, drawn as a dashed frame around its nodes.
#[derive(Clone, Debug)]
pub struct Group {
    pub title: String,
}

/// A flowchart: boxes are the nodes of `g` and arrows its edges, each in the
/// order they were first written. Nothing is ever removed, so indices stay
/// in that order too.
#[derive(Clone, Debug)]
pub struct Graph {
    pub direction: Direction,
    pub g: DiGraph<NodeDef, EdgeDef>,
    pub groups: Vec<Group>,
    /// `%% stack:` lines: boxes drawn in one column, directly under each
    /// other (beside each other in TD), in this order.
    pub stacks: Vec<Vec<NodeIndex>>,
}
