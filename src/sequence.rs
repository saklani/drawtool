//! Sequence diagrams: a subset of Mermaid's `sequenceDiagram`.
//!
//! Participants sit in a row with lifelines running down; each message is a
//! label row above a horizontal arrow. Columns are spaced so every label fits
//! its span, then `loop`/`alt`/`opt`/`par` frames are fitted around their
//! contents, widening gaps until no frame border lands on a lifeline.

use crate::model::Style;
use crate::paint::{Corners, Layer, Painter, Stroke};
use crate::parse::{clean_label, strip_comment};
use crate::text::{first_fit, width, wrap, Fit};
use crate::Error;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub struct Participant {
    pub id: String,
    pub lines: Vec<String>,
    pub actor: bool,
}

/// How a message ends at its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    Arrow,
    Open,
    Cross,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub from: usize,
    pub to: usize,
    pub lines: Vec<String>,
    pub dotted: bool,
    pub head: Head,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSide {
    Left,
    Right,
    Over,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub side: NoteSide,
    pub first: usize,
    pub last: usize,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Message(Message),
    Note(Note),
    /// `loop`, `alt`, `opt`, `par`, ... opening a frame.
    Start {
        kind: String,
        text: String,
    },
    /// `else` / `and` inside a frame.
    Divider {
        kind: String,
        text: String,
    },
    End,
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub participants: Vec<Participant>,
    pub items: Vec<Item>,
}

struct Patterns {
    participant: Regex,
    message: Regex,
    note: Regex,
    start: Regex,
    divider: Regex,
    ignored: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let rx = |s: &str| Regex::new(s).unwrap();
    Patterns {
        participant: rx(r"(?i)^(participant|actor)\s+([^\s]+?)(?:\s+as\s+(.+))?$"),
        message: rx(r"^([^\s:+\-][^\s:]*?)\s*(-->>|->>|--x|-x|--\)|-\)|-->|->)\s*[+\-]?\s*([^\s:]+)\s*(?::(.*))?$"),
        note: rx(r"(?i)^note\s+(left of|right of|over)\s+([^\s,:]+)(?:\s*,\s*([^\s,:]+))?\s*:(.*)$"),
        start: rx(r"^(loop|alt|opt|par|critical|break|rect|box)(?:\s+(.*))?$"),
        divider: rx(r"^(else|and|option)(?:\s+(.*))?$"),
        ignored: rx(r"(?i)^(sequenceDiagram$|autonumber\b|activate\b|deactivate\b|title\b|acc(Title|Descr)\b)"),
    }
});

/// True if the first meaningful line of `text` is `sequenceDiagram`.
pub fn is_sequence(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with('#'))
        .is_some_and(|l| l.eq_ignore_ascii_case("sequenceDiagram"))
}

fn label(text: Option<&str>) -> Vec<String> {
    let lines = clean_label(text.unwrap_or(""));
    if lines.iter().all(String::is_empty) {
        Vec::new()
    } else {
        lines
    }
}

/// Parse sequence diagram source.
///
/// # Errors
///
/// A line that isn't a participant, message, note or block keyword, or an
/// unbalanced block.
pub fn parse_sequence(text: &str) -> Result<Sequence, Error> {
    let p = &*PATTERNS;
    let (mut parts, mut index): (Vec<Participant>, HashMap<String, usize>) = Default::default();
    let mut get = |id: &str, parts: &mut Vec<Participant>| {
        *index.entry(id.to_string()).or_insert_with(|| {
            parts.push(Participant { id: id.to_string(), lines: vec![id.to_string()], actor: false });
            parts.len() - 1
        })
    };
    let mut items = Vec::new();
    let mut open: Vec<(String, usize)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let lineno = n + 1;
        let line = strip_comment(raw.trim()).trim_end_matches(';').trim();
        let opt = |m: Option<regex::Match>| m.map_or("", |m| m.as_str()).trim().to_string();
        if line.is_empty() || line.starts_with("%%") || line.starts_with('#') || p.ignored.is_match(line) {
            continue;
        } else if let Some(c) = p.participant.captures(line) {
            let i = get(&c[2], &mut parts);
            parts[i].actor = c[1].eq_ignore_ascii_case("actor");
            if let Some(alias) = c.get(3) {
                parts[i].lines = label(Some(alias.as_str()));
            }
        } else if let Some(c) = p.note.captures(line) {
            let side = match c[1].to_lowercase().as_str() {
                "left of" => NoteSide::Left,
                "right of" => NoteSide::Right,
                _ => NoteSide::Over,
            };
            let a = get(&c[2], &mut parts);
            let z = c.get(3).map_or(a, |m| get(m.as_str(), &mut parts));
            if side != NoteSide::Over && a != z {
                return Err(Error::at(lineno, "only `Note over` can span two participants"));
            }
            items.push(Item::Note(Note {
                side,
                first: a.min(z),
                last: a.max(z),
                lines: label(c.get(4).map(|m| m.as_str())),
            }));
        } else if line == "end" {
            open.pop().ok_or_else(|| Error::at(lineno, "`end` without an open block"))?;
            items.push(Item::End);
        } else if let Some(c) = p.start.captures(line) {
            open.push((c[1].to_string(), lineno));
            items.push(Item::Start { kind: c[1].to_string(), text: opt(c.get(2)) });
        } else if let Some(c) = p.divider.captures(line) {
            if open.is_empty() {
                return Err(Error::at(lineno, format!("`{}` outside a block", &c[1])));
            }
            items.push(Item::Divider { kind: c[1].to_string(), text: opt(c.get(2)) });
        } else if let Some(c) = p.message.captures(line) {
            let (from, to, arrow) = (get(&c[1], &mut parts), get(&c[3], &mut parts), &c[2]);
            let head = match arrow {
                a if a.ends_with('x') => Head::Cross,
                a if a.ends_with(">>") || a.ends_with(')') => Head::Arrow,
                _ => Head::Open,
            };
            let lines = label(c.get(4).map(|m| m.as_str()));
            items.push(Item::Message(Message { from, to, lines, dotted: arrow.starts_with("--"), head }));
        } else {
            return Err(Error::at(lineno, format!("cannot parse {line:?} (expected a message like A->>B: text)")));
        }
    }
    if let Some((kind, lineno)) = open.pop() {
        return Err(Error::at(lineno, format!("`{kind}` is never closed with `end`")));
    }
    Ok(Sequence { participants: parts, items })
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

fn lines_width(lines: &[String]) -> i64 {
    lines.iter().map(|l| width(l)).max().unwrap_or(0)
}

/// Left/right half-widths of a box centred on a column.
fn halves(width: i64) -> (i64, i64) {
    let left = (width - 1) / 2;
    (left, width - 1 - left)
}

fn note_width(n: &Note) -> i64 {
    (lines_width(&n.lines) + 4).max(5)
}

fn self_reach(m: &Message) -> i64 {
    (lines_width(&m.lines) + 1).max(3)
}

/// Horizontal extent `[lo, hi]` an item occupies, given lifeline columns.
fn item_extent(item: &Item, x: &[i64]) -> Option<(i64, i64)> {
    Some(match item {
        Item::Message(m) if m.from == m.to => (x[m.from], x[m.from] + self_reach(m)),
        Item::Message(m) => (x[m.from].min(x[m.to]), x[m.from].max(x[m.to])),
        Item::Note(n) => match (n.side, note_width(n), x[n.first]) {
            (NoteSide::Right, w, xf) => (xf + 2, xf + 1 + w),
            (NoteSide::Left, w, xf) => (xf - 1 - w, xf - 2),
            (_, w, xf) if n.first == n.last => (xf - halves(w).0, xf + halves(w).1),
            (_, w, xf) => (xf - 2, xf - 3 + (x[n.last] - xf + 5).max(w)),
        },
        _ => return None,
    })
}

fn block_title(kind: &str, text: &str) -> String {
    if text.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {text}")
    }
}

/// A frame opened by `loop`/`alt`/..., spanning columns `left..=right`.
struct Frame {
    title: String,
    left: i64,
    right: i64,
}

/// Columns `lo..=hi` covered so far, if anything.
type Extent = Option<(i64, i64)>;

/// Frames in order of their `Start`, fitted around their contents.
fn frames(items: &[Item], x: &[i64]) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::new();
    // Open frames: index, content extent so far, widest title or divider.
    let mut stack: Vec<(usize, Extent, i64)> = Vec::new();
    let grow = |e: &mut Extent, (lo, hi): (i64, i64)| *e = Some(e.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    for item in items {
        match item {
            Item::Start { kind, text } => {
                out.push(Frame { title: block_title(kind, text), left: 0, right: 0 });
                stack.push((out.len() - 1, None, width(&block_title(kind, text))));
            }
            Item::Divider { kind, text } => {
                if let Some(top) = stack.last_mut() {
                    top.2 = top.2.max(width(&block_title(kind, text)));
                }
            }
            Item::End => {
                let (f, extent, title) = stack.pop().expect("parser balances blocks");
                let (lo, hi) = extent.unwrap_or((x[0], x[0]));
                let (left, right) = (lo - 2, (hi + 2).max(lo - 2 + title + 3));
                (out[f].left, out[f].right) = (left, right);
                if let Some(parent) = stack.last_mut() {
                    grow(&mut parent.1, (left, right));
                }
            }
            _ => {
                if let (Some(parent), Some(e)) = (stack.last_mut(), item_extent(item, x)) {
                    grow(&mut parent.1, e);
                }
            }
        }
    }
    out
}

/// Lifeline columns: wide enough for boxes, labels and notes, with every
/// frame border clear of every lifeline.
fn columns(seq: &Sequence) -> Vec<i64> {
    let n = seq.participants.len();
    let boxw: Vec<i64> = seq.participants.iter().map(|p| lines_width(&p.lines) + 4).collect();
    let mut gap: Vec<i64> = (0..n.saturating_sub(1)).map(|i| halves(boxw[i]).1 + halves(boxw[i + 1]).0 + 3).collect();
    let mut spans: Vec<(usize, usize, i64)> = Vec::new();
    let mut need = |gap: &mut Vec<i64>, lo: usize, hi: usize, d: i64| match hi - lo {
        1 => gap[lo] = gap[lo].max(d),
        k if k > 1 => spans.push((lo, hi, d)),
        _ => {}
    };
    for item in &seq.items {
        match item {
            Item::Message(m) if m.from == m.to && m.from + 1 < n => {
                need(&mut gap, m.from, m.from + 1, self_reach(m) + 3)
            }
            Item::Message(m) if m.from != m.to => {
                need(&mut gap, m.from.min(m.to), m.from.max(m.to), (lines_width(&m.lines) + 4).max(6));
            }
            Item::Note(note) => {
                let (w, i, j) = (note_width(note), note.first, note.last);
                let (lh, rh) = halves(w);
                match note.side {
                    NoteSide::Right if i + 1 < n => need(&mut gap, i, i + 1, w + 3),
                    NoteSide::Left if i > 0 => need(&mut gap, i - 1, i, w + 3),
                    NoteSide::Over if i == j => {
                        if i > 0 {
                            need(&mut gap, i - 1, i, lh + 2 + halves(boxw[i - 1]).1.min(lh));
                        }
                        if i + 1 < n {
                            need(&mut gap, i, i + 1, rh + 2 + halves(boxw[i + 1]).0.min(rh));
                        }
                    }
                    NoteSide::Over => need(&mut gap, i, j, w - 5),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    spans.sort_by_key(|&(lo, hi, _)| hi - lo);
    for (lo, hi, d) in spans {
        let (have, k) = (gap[lo..hi].iter().sum::<i64>(), (hi - lo) as i64);
        if have < d {
            for (step, g) in gap[lo..hi].iter_mut().enumerate() {
                *g += (d - have) / k + i64::from((step as i64) < (d - have) % k);
            }
        }
    }
    let place = |gap: &[i64]| -> Vec<i64> { (0..n).map(|i| gap[..i].iter().sum()).collect() };
    // Push lifelines apart until no frame border sits on (or next to) one.
    for _ in 0..200 {
        let x = place(&gap);
        let touching = |&Frame { left, right, .. }: &Frame| -> Vec<usize> {
            let near = |a: i64, b: i64| (a - b).abs() <= 1;
            (0..n)
                .filter_map(|m| {
                    if near(left, x[m]) && m + 1 < n {
                        Some(m)
                    } else if near(right, x[m]) && m > 0 {
                        Some(m - 1)
                    } else {
                        None
                    }
                })
                .collect()
        };
        let Some(bumps) = frames(&seq.items, &x).iter().map(touching).find(|b| !b.is_empty()) else { break };
        for m in bumps {
            gap[m] += 1;
        }
    }
    place(&gap)
}

/// A box `w` wide with centred lines; an `opaque` one hides lines under it.
fn boxed(p: &mut Painter, (left, top): (i64, i64), w: i64, lines: &[String], round: bool, opaque: bool) {
    let h = lines.len() as i64 + 2;
    if opaque {
        p.clear((left, top), (left + w - 1, top + h - 1));
    }
    p.rect((left, top), (w, h), Stroke::LIGHT, if round { Corners::Round } else { Corners::Square }, Layer::Main);
    for (i, line) in lines.iter().enumerate() {
        p.text(left + 1 + (w - 2 - width(line)).div_euclid(2), top + 1 + i as i64, line);
    }
}

/// Render a sequence diagram as text.
#[must_use]
pub fn render_sequence(seq: &Sequence, ascii: bool) -> String {
    if seq.participants.is_empty() {
        return String::new();
    }
    let x = columns(seq);
    let frame_list = frames(&seq.items, &x);
    let mut p = Painter::new(ascii);
    let boxw: Vec<i64> = seq.participants.iter().map(|p| lines_width(&p.lines) + 4).collect();
    let boxh: Vec<i64> = seq.participants.iter().map(|p| p.lines.len() as i64 + 2).collect();

    let mut rows = Vec::with_capacity(seq.items.len());
    let mut y = boxh.iter().max().unwrap() + 1;
    for item in &seq.items {
        rows.push(y);
        y += match item {
            Item::Message(m) => m.lines.len() as i64 + 1 + i64::from(m.from == m.to),
            Item::Note(n) => n.lines.len() as i64 + 2,
            _ => 1,
        };
    }
    let footer = y + 1;
    for (i, &xi) in x.iter().enumerate() {
        let net = p.net();
        p.line(&[(xi, boxh[i] - 1), (xi, footer)], net, Stroke::LIGHT, Layer::Main);
    }

    // Frames sit under everything: lifelines and messages run through them.
    let (mut stack, mut next): (Vec<(usize, i64, usize)>, usize) = (Vec::new(), 0);
    for (item, &row) in seq.items.iter().zip(&rows) {
        let dotted = |p: &mut Painter, net, pts: &[(i64, i64)]| p.line(pts, net, Stroke::DOTTED, Layer::Under);
        match item {
            Item::Start { .. } => {
                stack.push((next, row, p.net()));
                next += 1;
            }
            Item::Divider { kind, text } => {
                if let Some(&(f, _, net)) = stack.last() {
                    let (l, r) = (frame_list[f].left, frame_list[f].right);
                    dotted(&mut p, net, &[(l, row), (r, row)]);
                    p.text(l + 1, row, &format!(" {} ", block_title(kind, text)));
                }
            }
            Item::End => {
                let (f, top, net) = stack.pop().expect("parser balances blocks");
                let Frame { title, left: l, right: r } = &frame_list[f];
                dotted(&mut p, net, &[(*l, top), (*r, top), (*r, row), (*l, row), (*l, top)]);
                p.text(l + 1, top, &format!(" {title} "));
            }
            _ => {}
        }
    }

    for (item, &row) in seq.items.iter().zip(&rows) {
        match item {
            Item::Message(m) => {
                let (a, b, right) = (x[m.from], x[m.to], x[m.to] >= x[m.from]);
                let lo = if m.from == m.to { a } else { a.min(b) };
                for (k, line) in m.lines.iter().enumerate() {
                    p.text(lo + 2, row + k as i64, line);
                }
                let r = row + m.lines.len() as i64;
                let (head, style) = (m.head, if m.dotted { Style::Dotted } else { Style::Solid });
                let step = if right { 1 } else { -1 };
                let (mut pts, tip) = if m.from == m.to {
                    (vec![(a, r), (a + 3, r), (a + 3, r + 1)], (a + 1, r + 1))
                } else {
                    (vec![(a, r)], (b - step, r))
                };
                pts.push(if head == Head::Open { (if m.from == m.to { a } else { b }, tip.1) } else { tip });
                let net = p.net();
                p.line(&pts, net, Stroke::light(style), Layer::Main);
                match head {
                    Head::Arrow => p.symbol(tip.0, tip.1, if right && m.from != m.to { '►' } else { '◄' }),
                    Head::Cross => p.symbol(tip.0, tip.1, 'x'),
                    Head::Open => {}
                }
            }
            Item::Note(n) => {
                let (lo, hi) = item_extent(item, &x).expect("notes have an extent");
                boxed(&mut p, (lo, row), hi - lo + 1, &n.lines, false, true);
            }
            _ => {}
        }
    }

    for (i, part) in seq.participants.iter().enumerate() {
        let left = x[i] - halves(boxw[i]).0;
        boxed(&mut p, (left, 0), boxw[i], &part.lines, part.actor, false);
        boxed(&mut p, (left, footer), boxw[i], &part.lines, part.actor, false);
    }
    p.finish()
}

/// Copy of `seq` with participant, message and note text wrapped at `max`.
#[must_use]
pub fn wrap_sequence(seq: &Sequence, max: usize) -> Sequence {
    let mut s = seq.clone();
    let wrap_all = |lines: &mut Vec<String>| *lines = lines.iter().flat_map(|l| wrap(l, max, false)).collect();
    s.participants.iter_mut().for_each(|p| wrap_all(&mut p.lines));
    for item in &mut s.items {
        match item {
            Item::Message(Message { lines, .. }) | Item::Note(Note { lines, .. }) => wrap_all(lines),
            _ => {}
        }
    }
    s
}

/// Render within `max_width` columns (0 = unlimited), wrapping text at 24,
/// then 16, then 12 columns if needed.
#[must_use]
pub fn render_sequence_fit(seq: &Sequence, ascii: bool, max_width: usize) -> Fit {
    let wrapped = [None, Some(24), Some(16), Some(12)].into_iter();
    first_fit(
        max_width,
        wrapped.map(|w| render_sequence(&w.map_or_else(|| seq.clone(), |w| wrap_sequence(seq, w)), ascii)),
    )
}
