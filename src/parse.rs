//! Parser for the Mermaid-style flowchart subset.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    TD,
    LR,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Box,
    Round,
    Decision,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Solid,
    Dotted,
    Thick,
}

#[derive(Clone, Debug)]
pub struct NodeDef {
    pub id: String,
    pub lines: Vec<String>,
    pub shape: Shape,
    explicit: bool,
}

impl NodeDef {
    pub fn new(id: &str, lines: Vec<String>, shape: Shape) -> Self {
        NodeDef { id: id.to_string(), lines, shape, explicit: true }
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

#[derive(Clone, Debug)]
pub struct Graph {
    pub direction: Direction,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

struct Patterns {
    id: Regex,
    connectors: Vec<(Regex, Style, bool)>,
    pipe_label: Regex,
    header: Regex,
    bare_header: Regex,
    ignored: Regex,
    br: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let rx = |s: &str| Regex::new(s).unwrap();
        Patterns {
            id: rx(r"^\w[\w.:$]*"),
            connectors: vec![
                (rx(r"^--\s+(.+?)\s+-->"), Style::Solid, true),
                (rx(r"^--\s+(.+?)\s+---"), Style::Solid, false),
                (rx(r"^-\.\s+(.+?)\s+\.->"), Style::Dotted, true),
                (rx(r"^==\s+(.+?)\s+==>"), Style::Thick, true),
                (rx(r"^-\.+->"), Style::Dotted, true),
                (rx(r"^-+>"), Style::Solid, true),
                (rx(r"^=+>"), Style::Thick, true),
                (rx(r"^-\.+-"), Style::Dotted, false),
                (rx(r"^-{3,}"), Style::Solid, false),
                (rx(r"^={3,}"), Style::Thick, false),
            ],
            pipe_label: rx(r"^\s*\|([^|]*)\|"),
            header: rx(r"(?i)^(?:(?:flowchart|graph)\s+|direction\s+)?(TD|TB|BT|LR|RL)$"),
            bare_header: rx(r"(?i)^(?:flowchart|graph)$"),
            ignored: rx(r"^(?:(?:subgraph|classDef|class|style|linkStyle|click)\b|end$)"),
            br: rx(r"(?i)<br\s*/?>"),
        }
    })
}

const SHAPES: &[(&str, &str, Shape)] = &[
    ("([", "])", Shape::Round),
    ("[(", ")]", Shape::Round),
    ("((", "))", Shape::Round),
    ("[[", "]]", Shape::Box),
    ("{{", "}}", Shape::Decision),
    ("[", "]", Shape::Box),
    ("(", ")", Shape::Round),
    ("{", "}", Shape::Decision),
];

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Strip quotes, expand `<br>` / `\n`, split into trimmed lines.
pub fn clean_label(text: &str) -> Vec<String> {
    let mut t = text.trim();
    let mut cs = t.chars();
    if let (Some(first), Some(last)) = (cs.next(), cs.next_back()) {
        if first == last && (first == '"' || first == '\'') {
            t = &t[1..t.len() - 1];
        }
    }
    let t = patterns().br.replace_all(t, "\n").replace("\\n", "\n");
    t.split('\n').map(|l| l.trim().to_string()).collect()
}

fn skip_ws(s: &str, mut pos: usize) -> usize {
    while let Some(c) = s[pos..].chars().next() {
        if !c.is_whitespace() {
            break;
        }
        pos += c.len_utf8();
    }
    pos
}

/// Find a closing delimiter that is followed by end-of-line, `&` or an edge.
fn find_close(s: &str, start: usize, close: &str) -> Option<usize> {
    let mut from = start;
    while let Some(off) = s[from..].find(close) {
        let i = from + off;
        let rest = s[i + close.len()..].trim_start();
        match rest.chars().next() {
            None => return Some(i),
            Some(c) if "-=&.%".contains(c) => return Some(i),
            _ => from = i + 1,
        }
    }
    None
}

struct Parser {
    nodes: Vec<NodeDef>,
    index: HashMap<String, usize>,
    edges: Vec<EdgeDef>,
}

impl Parser {
    fn node(&mut self, s: &str, pos: usize) -> Result<(usize, usize), String> {
        let m = patterns()
            .id
            .find(&s[pos..])
            .ok_or_else(|| format!("expected a node id at {}", quote(&s[pos..])))?;
        let id = m.as_str().to_string();
        let pos = pos + m.end();
        let idx = match self.index.get(&id) {
            Some(&i) => i,
            None => {
                self.nodes.push(NodeDef { id: id.clone(), lines: vec![id.clone()], shape: Shape::Box, explicit: false });
                self.index.insert(id.clone(), self.nodes.len() - 1);
                self.nodes.len() - 1
            }
        };
        for &(opener, closer, shape) in SHAPES {
            if s[pos..].starts_with(opener) {
                let end = find_close(s, pos + opener.len(), closer)
                    .ok_or_else(|| format!("unclosed {} for node {}", quote(opener), quote(&id)))?;
                let node = &mut self.nodes[idx];
                if !node.explicit {
                    node.lines = clean_label(&s[pos + opener.len()..end]);
                    node.shape = shape;
                    node.explicit = true;
                }
                return Ok((idx, end + closer.len()));
            }
        }
        Ok((idx, pos))
    }

    fn group(&mut self, s: &str, mut pos: usize) -> Result<(Vec<usize>, usize), String> {
        let mut ids = Vec::new();
        loop {
            let (id, p) = self.node(s, skip_ws(s, pos))?;
            ids.push(id);
            pos = p;
            let nxt = skip_ws(s, pos);
            if s[nxt..].starts_with('&') {
                pos = nxt + 1;
            } else {
                return Ok((ids, pos));
            }
        }
    }

    fn statement(&mut self, s: &str) -> Result<(), String> {
        let (mut group, mut pos) = self.group(s, 0)?;
        loop {
            pos = skip_ws(s, pos);
            if pos >= s.len() || s[pos..].starts_with("%%") {
                return Ok(());
            }
            let (caps, style, arrow) = patterns()
                .connectors
                .iter()
                .find_map(|(rx, st, ar)| rx.captures(&s[pos..]).map(|c| (c, *st, *ar)))
                .ok_or_else(|| format!("expected an arrow like --> at {}", quote(&s[pos..])))?;
            let mut label = caps.get(1).map(|m| m.as_str().to_string());
            pos += caps.get(0).unwrap().end();
            if let Some(pm) = patterns().pipe_label.captures(&s[pos..]) {
                label = Some(pm[1].to_string());
                pos += pm.get(0).unwrap().end();
            }
            let label = label.filter(|l| !l.trim().is_empty()).map(|l| clean_label(&l));
            let (nxt, p) = self.group(s, pos)?;
            pos = p;
            for &a in &group {
                for &b in &nxt {
                    self.edges.push(EdgeDef { src: a, dst: b, label: label.clone(), style, arrow });
                }
            }
            group = nxt;
        }
    }
}

/// Parse diagram source into a graph.
pub fn parse(text: &str) -> Result<Graph, String> {
    let pats = patterns();
    let mut direction = Direction::LR;
    let mut p = Parser { nodes: Vec::new(), index: HashMap::new(), edges: Vec::new() };
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_end_matches(';').trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with('#') {
            continue;
        }
        if let Some(c) = pats.header.captures(line) {
            let d = c[1].to_uppercase();
            direction = if d == "LR" || d == "RL" { Direction::LR } else { Direction::TD };
            continue;
        }
        if pats.bare_header.is_match(line) || pats.ignored.is_match(line) {
            continue;
        }
        p.statement(line).map_err(|e| format!("line {}: {}", lineno + 1, e))?;
    }
    Ok(Graph { direction, nodes: p.nodes, edges: p.edges })
}
