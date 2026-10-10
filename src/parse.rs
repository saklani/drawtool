//! Parser for the Mermaid-style flowchart subset.

use crate::model::{Direction, EdgeDef, Graph, Group, Mark, NodeDef, Shape, Style};
use crate::Error;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// `line` without a trailing `%% comment`. A line that starts with `%%` is
/// left alone: it is a whole-line comment or a `%% highlight:` directive.
pub(crate) fn strip_comment(line: &str) -> &str {
    let at = line.match_indices("%%").map(|(i, _)| i).find(|&i| i > 0 && line[..i].ends_with(char::is_whitespace));
    at.map_or(line, |i| line[..i].trim_end())
}

struct Patterns {
    id: Regex,
    connectors: Vec<(Regex, Style, bool)>,
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
        ignored: rx(r"^(?:classDef|class|style|linkStyle|click)\b"),
        subgraph: rx(r#"^subgraph\s+(?:(\w[\w.:$-]*)\s*\[(.*)\]|(.+))$"#),
        br: rx(r"(?i)<br\s*/?>"),
    }
});

/// Shape brackets, two-character openers first so `[(` wins over `[`.
const SHAPES: &[(&str, &str, Shape)] = &[
    ("([", "])", Shape::Round),
    ("[(", ")]", Shape::Database),
    ("((", "))", Shape::Round),
    ("[[", "]]", Shape::Box),
    ("{{", "}}", Shape::Decision),
    ("[", "]", Shape::Box),
    ("(", ")", Shape::Round),
    ("{", "}", Shape::Decision),
];

/// Strip quotes, expand `<br>` / `\n`, split into trimmed lines.
#[must_use]
pub fn clean_label(text: &str) -> Vec<String> {
    let mut t = text.trim();
    let mut cs = t.chars();
    if let (Some(first), Some(last)) = (cs.next(), cs.next_back()) {
        if first == last && (first == '"' || first == '\'') {
            t = &t[1..t.len() - 1];
        }
    }
    let t = PATTERNS.br.replace_all(t, "\n").replace("\\n", "\n");
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

/// A failed statement: the byte offset where it went wrong, and why.
type Fail = (usize, String);

struct Parser {
    nodes: Vec<NodeDef>,
    index: HashMap<String, usize>,
    edges: Vec<EdgeDef>,
    /// Open `subgraph` blocks (group, line), innermost last.
    open: Vec<(usize, usize)>,
}

impl Parser {
    fn node(&mut self, s: &str, pos: usize) -> Result<(usize, usize), Fail> {
        let m = PATTERNS.id.find(&s[pos..]).ok_or_else(|| (pos, format!("expected a node id at {:?}", &s[pos..])))?;
        let id = m.as_str().to_string();
        let pos = pos + m.end();
        let group = self.open.last().map(|o| o.0);
        let idx = if let Some(&i) = self.index.get(&id) {
            if self.nodes[i].group.is_none() {
                self.nodes[i].group = group;
            }
            i
        } else {
            self.nodes.push(NodeDef {
                id: id.clone(),
                lines: vec![id.clone()],
                shape: Shape::Box,
                mark: Mark::None,
                group,
                explicit: false,
            });
            self.index.insert(id.clone(), self.nodes.len() - 1);
            self.nodes.len() - 1
        };
        for &(opener, closer, shape) in SHAPES {
            if s[pos..].starts_with(opener) {
                let end = find_close(s, pos + opener.len(), closer)
                    .ok_or_else(|| (pos, format!("unclosed {opener:?} for node {id:?}")))?;
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

    fn group(&mut self, s: &str, mut pos: usize) -> Result<(Vec<usize>, usize), Fail> {
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

    fn statement(&mut self, s: &str) -> Result<(), Fail> {
        let (mut group, mut pos) = self.group(s, 0)?;
        loop {
            pos = skip_ws(s, pos);
            if pos >= s.len() || s[pos..].starts_with("%%") {
                return Ok(());
            }
            let (caps, style, arrow) = PATTERNS
                .connectors
                .iter()
                .find_map(|(rx, st, ar)| rx.captures(&s[pos..]).map(|c| (c, *st, *ar)))
                .ok_or_else(|| (pos, format!("expected an arrow like --> at {:?}", &s[pos..])))?;
            let mut label = caps.get(1).map(|m| m.as_str().to_string());
            pos += caps.get(0).unwrap().end();
            if let Some(pm) = PATTERNS.pipe_label.captures(&s[pos..]) {
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
///
/// # Errors
///
/// A line that isn't valid flowchart syntax, an unbalanced `subgraph`/`end`,
/// or a `%% highlight:` path that doesn't exist.
pub fn parse(text: &str) -> Result<Graph, Error> {
    let pats = &*PATTERNS;
    let mut direction = Direction::LR;
    let mut p = Parser { nodes: Vec::new(), index: HashMap::new(), edges: Vec::new(), open: Vec::new() };
    let mut groups: Vec<Group> = Vec::new();
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
            let title = c.get(2).or_else(|| c.get(1)).or_else(|| c.get(3)).map_or("", |m| m.as_str());
            groups.push(Group { title: clean_label(title).join(" ") });
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
            let d = c[1].to_uppercase();
            direction = if d == "LR" || d == "RL" { Direction::LR } else { Direction::TD };
            continue;
        }
        if pats.bare_header.is_match(line) || pats.ignored.is_match(line) {
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
    let mut graph = Graph { direction, nodes: p.nodes, edges: p.edges, groups };
    if let Some((lineno, ids)) = highlight {
        crate::marks::highlight(&mut graph, &ids).map_err(|e| Error::at(lineno, e.message))?;
    }
    Ok(graph)
}
