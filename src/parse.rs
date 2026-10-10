//! Parser for the Mermaid-style flowchart subset.

use crate::model::{Arrow, Direction, EdgeDef, Graph, Group, NodeDef, Shape, Style};
use crate::Error;
use petgraph::graph::{DiGraph, NodeIndex};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// `line` without a trailing `%% comment`. A line that starts with `%%` is
/// left alone: it is a whole-line comment or a `%% highlight:` directive.
pub(crate) fn strip_comment(line: &str) -> &str {
    let at = line.match_indices("%%").map(|(i, _)| i).find(|&i| i > 0 && line[..i].ends_with(char::is_whitespace));
    at.map_or(line, |i| line[..i].trim_end())
}

/// `s` trimmed, without one pair of matching surrounding quotes (`"…"` or `'…'`).
pub(crate) fn unquote(s: &str) -> String {
    let s = s.trim();
    let quoted = ['"', '\''].iter().any(|&q| s.len() >= 2 && s.starts_with(q) && s.ends_with(q));
    if quoted { &s[1..s.len() - 1] } else { s }.to_string()
}

struct Patterns {
    id: Regex,
    connectors: Vec<(Regex, Style)>,
    class_suffix: Regex,
    pipe_label: Regex,
    header: Regex,
    bare_header: Regex,
    ignored: Regex,
    subgraph: Regex,
    br: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let rx = |s: &str| Regex::new(s).unwrap();
    Patterns {
        id: rx(r"^\w[\w.:$]*"),
        // Two-headed forms (`<-->`) first, then labelled ones; a label ends at
        // the first closing arrow, so `a -- x --- b -- y --> c` is two edges.
        connectors: vec![
            (rx(r"^<--\s*([^\s>-].*?)\s*-{2,}>"), Style::Solid),
            (rx(r"^<-\.\s*([^\s.>-].*?)\s*\.+->"), Style::Dotted),
            (rx(r"^<==\s*([^\s=>].*?)\s*={2,}>"), Style::Thick),
            (rx(r"^<-{2,}>"), Style::Solid),
            (rx(r"^<-\.+->"), Style::Dotted),
            (rx(r"^<={2,}>"), Style::Thick),
            (rx(r"^--\s*([^\s>-].*?)\s*(?:-{2,}>|-{3,})"), Style::Solid),
            (rx(r"^-\.\s*([^\s.>-].*?)\s*\.-+>?"), Style::Dotted),
            (rx(r"^==\s*([^\s=>].*?)\s*(?:=+>|={3,})"), Style::Thick),
            (rx(r"^-\.+->"), Style::Dotted),
            (rx(r"^-{2,}>"), Style::Solid),
            (rx(r"^={2,}>"), Style::Thick),
            (rx(r"^-\.+-"), Style::Dotted),
            (rx(r"^-{3,}"), Style::Solid),
            (rx(r"^={3,}"), Style::Thick),
        ],
        class_suffix: rx(r"^:::\w[\w-]*"),
        pipe_label: rx(r"^\s*\|([^|]*)\|"),
        header: rx(r"(?i)^(?:(?:flowchart|graph)\s+|direction\s+)?(TD|TB|BT|LR|RL)$"),
        bare_header: rx(r"(?i)^(?:flowchart|graph)$"),
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

/// [`unquote`], then expand `<br>` / `\n` and split into trimmed lines.
#[must_use]
pub fn clean_label(text: &str) -> Vec<String> {
    let t = PATTERNS.br.replace_all(&unquote(text), "\n").replace("\\n", "\n");
    t.split('\n').map(|l| l.trim().to_string()).collect()
}

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

    fn group(&mut self, s: &str, mut pos: usize) -> Result<(Vec<NodeIndex>, usize), Fail> {
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
                .find_map(|(rx, st)| rx.captures(&s[pos..]).map(|c| (c, *st)))
                .ok_or_else(|| (pos, format!("expected an arrow like --> at {:?}", &s[pos..])))?;
            let arrow = match (caps[0].starts_with('<'), caps[0].ends_with('>')) {
                (true, _) => Arrow::Both,
                (_, true) => Arrow::End,
                _ => Arrow::None,
            };
            let mut text = caps.get(1).map(|m| m.as_str().to_string());
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

/// Parse diagram source into a graph.
///
/// # Errors
///
/// A line that isn't valid flowchart syntax, an unbalanced `subgraph`/`end`,
/// a reversed direction (`RL`, `BT`), an edge to a subgraph, or a
/// `%% highlight:` path that doesn't exist.
pub fn parse(text: &str) -> Result<Graph, Error> {
    let pats = &*PATTERNS;
    let mut direction = Direction::LR;
    let mut p = Parser { g: DiGraph::new(), index: HashMap::new(), open: Vec::new() };
    let mut groups: Vec<Group> = Vec::new();
    // Subgraph ids (id, line): a node may not reuse one.
    let mut group_ids: Vec<(String, usize)> = Vec::new();
    let mut highlight: Option<(usize, Vec<String>)> = None;
    for (lineno, raw) in text.lines().enumerate() {
        let line = strip_comment(raw.trim()).trim_end_matches(';').trim();
        if let Some(ids) = line.strip_prefix("%%").map(str::trim).and_then(|c| c.strip_prefix("highlight:")) {
            highlight = Some((lineno + 1, crate::marks::split_ids(ids)));
            continue;
        }
        if line.is_empty() || line.starts_with("%%") || line.starts_with('#') {
            continue;
        }
        if let Some(c) = pats.subgraph.captures(line) {
            if let Some(id) = c.get(1).or_else(|| c.get(3)) {
                group_ids.push((id.as_str().to_string(), lineno + 1));
            }
            let title = [2, 1, 3, 4].into_iter().find_map(|i| c.get(i)).map_or("", |m| m.as_str());
            groups.push(Group { title: label(title).join(" ") });
            p.open.push((groups.len() - 1, lineno + 1));
            continue;
        }
        if line == "end" {
            if p.open.pop().is_none() {
                return Err(Error::at(lineno + 1, "`end` without a matching `subgraph`"));
            }
            continue;
        }
        if let Some(c) = pats.header.captures(line) {
            if !p.open.is_empty() {
                continue; // a subgraph's own direction isn't supported
            }
            direction = match c[1].to_uppercase().as_str() {
                "LR" => Direction::LR,
                "TD" | "TB" => Direction::TD,
                d => return Err(Error::at(lineno + 1, format!("`{d}` isn't supported: use LR or TD"))),
            };
            continue;
        }
        if pats.bare_header.is_match(line) {
            direction = Direction::TD; // Mermaid's default once there is a header
            continue;
        }
        // `style n1 …` declares n1, as in Mermaid; other styling is ignored.
        if let Some(id) = line.strip_prefix("style ").and_then(|r| pats.id.find(r.trim_start())) {
            p.node_id(id.as_str());
            continue;
        }
        if pats.ignored.is_match(line) {
            continue;
        }
        p.statement(line).map_err(|(at, message)| {
            // 1-based column in the original line, counting characters.
            let indent = raw.len() - raw.trim_start().len();
            Error::at_column(lineno + 1, raw[..indent].chars().count() + line[..at].chars().count() + 1, message)
        })?;
    }
    if let Some(&(g, line)) = p.open.first() {
        return Err(Error::at(line, format!("`subgraph {}` is never closed with `end`", groups[g].title)));
    }
    if let Some((id, line)) = group_ids.iter().find(|(id, _)| p.index.contains_key(id)) {
        return Err(Error::at(
            *line,
            format!("`{id}` is both a subgraph and a node (edges to subgraphs aren't supported)"),
        ));
    }
    let mut graph = Graph { direction, g: p.g, groups };
    if let Some((lineno, ids)) = highlight {
        crate::marks::highlight(&mut graph, &ids).map_err(|e| Error::at(lineno, e.message))?;
    }
    Ok(graph)
}
