//! The parsed flowchart: what every stage after parsing works on.

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
    /// Index into `Graph::groups` of the innermost `subgraph` the node
    /// first appeared in.
    pub group: Option<usize>,
    /// Whether a label/shape was given (the first one given wins).
    pub(crate) explicit: bool,
}

impl NodeDef {
    #[must_use]
    pub fn new(id: &str, lines: Vec<String>, shape: Shape) -> Self {
        NodeDef { id: id.to_string(), lines, shape, mark: Mark::None, group: None, explicit: true }
    }
}

#[derive(Clone, Debug)]
pub struct EdgeDef {
    pub src: usize,
    pub dst: usize,
    pub label: Option<Vec<String>>,
    pub style: Style,
    pub arrow: bool,
}

/// A `subgraph` block, drawn as a dashed frame around its nodes.
#[derive(Clone, Debug)]
pub struct Group {
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub direction: Direction,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
    pub groups: Vec<Group>,
}
