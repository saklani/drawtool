//! Parser for the Mermaid-style flowchart subset.

use super::model::{Arrow, Direction, EdgeDef, Graph, Group, NodeDef, Shape, Style};
use crate::text::strip_comment;
use crate::Error;
use petgraph::graph::{DiGraph, NodeIndex};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

struct Patterns {
    id: Regex,
    /// One per style, capturing `<`?, the label, the closing run and `>`?;
    /// with no `>`, the closing run must be at least this long (`---`).
    connectors: [(Regex, Style, usize); 3],
    class_suffix: Regex,
    pipe_label: Regex,
    header: Regex,
    direction: Regex,
    ignored: Regex,
    subgraph: Regex,
    br: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let rx = |s: &str| Regex::new(s).unwrap();
    Patterns {
        id: rx(r"^\w[\w.:$]*"),
        // A label ends at the first closing arrow, so `a -- x --- b -- y --> c` is two edges.
        connectors: [
            (rx(r"^(<?)(?:--\s*([^\s>-].*?)\s*)?(-{2,})(>?)"), Style::Solid, 3),
            (rx(r"^(<?)-\.+(?:\s*([^\s.>-].*?)\s*\.+)?(-)(>?)"), Style::Dotted, 1),
            (rx(r"^(<?)(?:==\s*([^\s=>].*?)\s*)?(={2,})(>?)"), Style::Thick, 3),
        ],
        class_suffix: rx(r"^:::\w[\w-]*"),
        pipe_label: rx(r"^\s*\|([^|]*)\|"),
        header: rx(r"(?i)^(?:flowchart|graph)(?:\s+(\S+))?$"),
        direction: rx(r"(?i)^direction\s+(\S+)$"),
        ignored: rx(r"^(?:classDef|class|style|linkStyle|click)\b"),
        // `subgraph id [title]`, `subgraph id`, or `subgraph any title`.
        subgraph: rx(r#"^subgraph\s+(?:(\w[\w.:$-]*)\s*\[(.*)\]|(\w[\w.:$-]*)|(.+))$"#),
        br: rx(r"(?i)<br\s*/?>"),
    }
});

/// Shape brackets, longest openers first so `[(` wins over `[`. Shapes
/// drawtool has no glyphs for (parallelograms, trapezoids, `>flag]`, double
/// circles) keep their label and are drawn as the nearest shape it has.
const SHAPES: &[(&str, &str, Shape)] = &[
    ("(((", ")))", Shape::Round),
    ("([", "])", Shape::Round),
    ("[(", ")]", Shape::Database),
    ("((", "))", Shape::Round),
    ("[[", "]]", Shape::Box),
    ("{{", "}}", Shape::Decision),
    ("[/", "/]", Shape::Box),
    ("[/", "\\]", Shape::Box),
    ("[\\", "\\]", Shape::Box),
    ("[\\", "/]", Shape::Box),
    (">", "]", Shape::Box),
    ("[", "]", Shape::Box),
    ("(", ")", Shape::Round),
    ("{", "}", Shape::Decision),
];

/// A flowchart label as Mermaid reads it: only `"…"` quotes are stripped,
/// and only `<br>` breaks a line.
fn label(text: &str) -> Vec<String> {
    let t = text.trim();
    let t = t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(t);
    PATTERNS.br.split(t).map(|l| l.trim().to_string()).collect()
}

/// The first position at or after `pos` that isn't whitespace.
fn skip_ws(s: &str, pos: usize) -> usize {
    s.len() - s[pos..].trim_start().len()
}

/// Find a closing delimiter that is followed by end-of-line, `&`, `;`, a
/// `:::class` or an edge.
fn find_close(s: &str, start: usize, close: &str) -> Option<usize> {
    let mut from = start;
    while let Some(off) = s[from..].find(close) {
        let i = from + off;
        if s[i + close.len()..].trim_start().chars().next().is_none_or(|c| "-=&.%;:<".contains(c)) {
            return Some(i);
        }
        from = i + 1;
    }
    None
}

/// A failed statement: the byte offset where it went wrong, and why.
type Fail = (usize, String);

struct Parser {
    g: DiGraph<NodeDef, EdgeDef>,
    index: HashMap<String, NodeIndex>,
    /// Open `subgraph` blocks (group, line), innermost last.
    open: Vec<(usize, usize)>,
}

impl Parser {
    /// The node called `id`, added if it's new.
    fn node_id(&mut self, id: &str) -> NodeIndex {
        *self
            .index
            .entry(id.to_string())
            .or_insert_with(|| self.g.add_node(NodeDef::new(id, vec![id.to_string()], Shape::Box)))
    }

    /// `id`, an optional shape with its label (the last one given wins, as
    /// in Mermaid), and an optional `:::class`, which is ignored.
    fn node(&mut self, s: &str, pos: usize) -> Result<(NodeIndex, usize), Fail> {
        let m = PATTERNS.id.find(&s[pos..]).ok_or_else(|| (pos, format!("expected a node id at {:?}", &s[pos..])))?;
        let id = m.as_str().split(":::").next().unwrap_or_default();
        let idx = self.node_id(id);
        // As in Mermaid, the first subgraph to close claims a node: one opened
        // inside the node's group (still open) closes first, a sibling later.
        if let Some(&(inner, _)) = self.open.last() {
            let (open, group) = (&self.open, &mut self.g[idx].group);
            if group.is_none_or(|g| open.iter().any(|o| o.0 == g)) {
                *group = Some(inner);
            }
        }
        let mut pos = pos + id.len();
        let mut unclosed = None;
        for &(opener, closer, shape) in SHAPES.iter().filter(|sh| s[pos..].starts_with(sh.0)) {
            if let Some(end) = find_close(s, pos + opener.len(), closer) {
                let node = &mut self.g[idx];
                node.lines = label(&s[pos + opener.len()..end]);
                node.shape = shape;
                pos = end + closer.len();
                unclosed = None;
                break;
            }
            unclosed = unclosed.or(Some(opener));
        }
        if let Some(opener) = unclosed {
            return Err((pos, format!("unclosed {opener:?} for node {id:?}")));
        }
        Ok((idx, pos + PATTERNS.class_suffix.find(&s[pos..]).map_or(0, |m| m.end())))
    }

    /// Nodes joined by `&`.
    fn group(&mut self, s: &str, mut pos: usize) -> Result<(Vec<NodeIndex>, usize), Fail> {
        let mut ids = Vec::new();
        loop {
            let (id, p) = self.node(s, skip_ws(s, pos))?;
            ids.push(id);
            pos = skip_ws(s, p);
            if !s[pos..].starts_with('&') {
                return Ok((ids, p));
            }
            pos += 1;
        }
    }

    /// Groups joined by arrows; `;` starts another statement on the same line.
    fn statement(&mut self, s: &str) -> Result<(), Fail> {
        let (mut group, mut pos) = self.group(s, 0)?;
        loop {
            pos = skip_ws(s, pos);
            if let Some(rest) = s[pos..].strip_prefix(';') {
                if rest.trim().is_empty() {
                    return Ok(());
                }
                (group, pos) = self.group(s, pos + 1)?;
                continue;
            }
            if pos >= s.len() {
                return Ok(());
            }
            let (caps, style) = PATTERNS
                .connectors
                .iter()
                .find_map(|(rx, st, min)| rx.captures(&s[pos..]).map(|c| (c, *st, *min)))
                .filter(|(c, _, min)| if c[4].is_empty() { c[1].is_empty() && c[3].len() >= *min } else { true })
                .map(|(c, st, _)| (c, st))
                .ok_or_else(|| (pos, format!("expected an arrow like --> at {:?}", &s[pos..])))?;
            let arrow = match (&caps[1], &caps[4]) {
                ("<", _) => Arrow::Both,
                (_, ">") => Arrow::End,
                _ => Arrow::None,
            };
            let mut text = caps.get(2).map(|m| m.as_str().to_string());
            pos += caps[0].len();
            if let Some(pm) = PATTERNS.pipe_label.captures(&s[pos..]) {
                text = Some(pm[1].to_string());
                pos += pm[0].len();
            }
            let text = text.filter(|l| !l.trim().is_empty()).map(|l| label(&l));
            let (nxt, p) = self.group(s, pos)?;
            pos = p;
            for &a in &group {
                for &b in &nxt {
                    self.g.add_edge(a, b, EdgeDef { label: text.clone(), style, arrow });
                }
            }
            group = nxt;
        }
    }
}

/// How a flowchart must start; said whenever its header is missing.
const START: &str = "a flowchart starts with `flowchart LR` or `flowchart TD`";

/// `LR` or `TD`/`TB`, in any case.
fn parse_direction(d: &str, lineno: usize) -> Result<Direction, Error> {
    match d.to_uppercase().as_str() {
        "LR" => Ok(Direction::LR),
        "TD" | "TB" => Ok(Direction::TD),
        _ => Err(Error::at(lineno, format!("`{d}` isn't supported: use LR or TD"))),
    }
}

/// Parse diagram source into a graph. The first line that isn't blank or a
/// `%%` comment must be the header: `flowchart`/`graph`, then `LR` or `TD`.
///
/// # Errors
///
/// A missing or second header, a line that isn't valid flowchart syntax, an unbalanced `subgraph`/`end`,
/// a reversed direction (`RL`, `BT`), an edge to a subgraph, or a
/// `%% highlight:` path that doesn't exist.
pub fn parse(text: &str) -> Result<Graph, Error> {
    let pats = &*PATTERNS;
    let mut direction = None;
    let mut p = Parser { g: DiGraph::new(), index: HashMap::new(), open: Vec::new() };
    let mut groups: Vec<Group> = Vec::new();
    // Subgraph ids (id, line): a node may not reuse one.
    let mut group_ids: Vec<(String, usize)> = Vec::new();
    let mut highlight: Option<(usize, Vec<String>)> = None;
    let mut stacks: Vec<(usize, Vec<String>)> = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = strip_comment(raw.trim()).trim_end_matches(';').trim();
        let directive = |name| line.strip_prefix("%%").map(str::trim).and_then(|c| c.strip_prefix(name));
        if let Some(ids) = directive("highlight:") {
            let ids = super::marks::highlight_ids(ids).map_err(|e| Error::at(lineno + 1, e.message))?;
            highlight = Some((lineno + 1, ids));
        } else if let Some(ids) = directive("stack:") {
            stacks.push((lineno + 1, super::marks::split_ids(ids)));
        } else if line.is_empty() || line.starts_with("%%") {
        } else if let Some(c) = pats.header.captures(line) {
            if direction.is_some() {
                return Err(Error::at(lineno + 1, "a second header: the header goes on the first line only"));
            }
            // A bare `flowchart` means TD, as in Mermaid.
            direction = Some(parse_direction(c.get(1).map_or("TD", |m| m.as_str()), lineno + 1)?);
        } else if direction.is_none() {
            return Err(Error::at(lineno + 1, format!("expected a header, found `{line}`; {START}")));
        } else if let Some(c) = pats.subgraph.captures(line) {
            if let Some(id) = c.get(1).or_else(|| c.get(3)) {
                group_ids.push((id.as_str().to_string(), lineno + 1));
            }
            let title = [2, 1, 3, 4].into_iter().find_map(|i| c.get(i)).map_or("", |m| m.as_str());
            groups.push(Group { title: label(title).join(" ") });
            p.open.push((groups.len() - 1, lineno + 1));
        } else if line == "end" {
            if p.open.pop().is_none() {
                return Err(Error::at(lineno + 1, "`end` without a matching `subgraph`"));
            }
        } else if let Some(c) = pats.direction.captures(line) {
            // A subgraph's own direction isn't supported.
            let d = parse_direction(&c[1], lineno + 1)?;
            if p.open.is_empty() {
                direction = Some(d);
            }
        } else if let Some(id) = line.strip_prefix("style ").and_then(|r| pats.id.find(r.trim_start())) {
            // `style n1 …` declares n1, as in Mermaid; other styling is ignored.
            p.node_id(id.as_str());
        } else if !pats.ignored.is_match(line) {
            p.statement(line).map_err(|(at, message)| {
                // 1-based column in the original line, counting characters.
                let indent = raw.len() - raw.trim_start().len();
                let column = raw[..indent].chars().count() + line[..at].chars().count() + 1;
                Error::at_column(lineno + 1, column, message)
            })?;
        }
    }
    if let Some(&(g, line)) = p.open.first() {
        return Err(Error::at(line, format!("`subgraph {}` is never closed with `end`", groups[g].title)));
    }
    if let Some((id, line)) = group_ids.iter().find(|(id, _)| p.index.contains_key(id)) {
        let message = format!("`{id}` is both a subgraph and a node (edges to subgraphs aren't supported)");
        return Err(Error::at(*line, message));
    }
    let direction = direction.ok_or_else(|| Error::new(format!("no diagram: {START}")))?;
    let stacks = resolve_stacks(&p.g, &p.index, stacks)?;
    let mut graph = Graph { direction, g: p.g, groups, stacks };
    if let Some((lineno, ids)) = highlight {
        super::marks::highlight(&mut graph, &ids).map_err(|e| Error::at(lineno, e.message))?;
    }
    Ok(graph)
}

/// The boxes of each `%% stack:` line (line, ids), checked: every id is a
/// node, a stack has two or more, no box is stacked twice, and a stack's
/// boxes are all in the same subgraph (or none).
fn resolve_stacks(
    g: &DiGraph<NodeDef, EdgeDef>,
    index: &HashMap<String, NodeIndex>,
    lines: Vec<(usize, Vec<String>)>,
) -> Result<Vec<Vec<NodeIndex>>, Error> {
    let mut seen = HashSet::new();
    let mut stacks = Vec::new();
    for (line, ids) in lines {
        if ids.len() < 2 {
            return Err(Error::at(line, "`%% stack:` needs two or more boxes"));
        }
        let mut stack = Vec::new();
        for id in &ids {
            let &v = index.get(id).ok_or_else(|| Error::at(line, format!("stack: no node {id:?}")))?;
            if !seen.insert(v) {
                return Err(Error::at(line, format!("stack: {id:?} is already in a stack")));
            }
            stack.push(v);
        }
        if stack.iter().any(|&v| g[v].group != g[stack[0]].group) {
            return Err(Error::at(line, "stack: its boxes must all be in the same subgraph"));
        }
        stacks.push(stack);
    }
    Ok(stacks)
}
