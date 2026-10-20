//! Sequence diagrams: Mermaid's `sequenceDiagram`, parsed by Ondine.
//!
//! Participants sit in a row with lifelines running down; each message is a
//! label row above a horizontal arrow. Columns are spaced so every label fits
//! its span, then `loop`/`alt`/`opt`/`par` frames are fitted around their
//! contents, widening gaps until no frame border lands on a lifeline.

use crate::paint::{Corners, Layer, Painter, Stroke};
use crate::text::{first_fit, width, wrap_all, Fit, WRAP_WIDTHS};
use crate::Style;
use crate::mermaid::{get, list};
use serde_json::Value;

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
    /// `loop`, `alt`, `opt`, `par`, ... opening a frame, with its title (`loop every 5m`).
    Start(String),
    /// `else` / `and` inside a frame, with its title.
    Divider(String),
    End,
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub participants: Vec<Participant>,
    pub items: Vec<Item>,
}

/// Mermaid's message codes (`SequenceDB.LINETYPE`): frame starts with their
/// keyword, dividers, and ends.
const FRAMES: &[(i64, &str)] = &[(10, "loop"), (12, "alt"), (15, "opt"), (19, "par"), (22, "rect"), (27, "critical"), (30, "break")];
const DIVIDERS: &[(i64, &str)] = &[(13, "else"), (20, "and"), (28, "option")];
const ENDS: &[i64] = &[11, 14, 16, 21, 23, 29, 31];

/// Label text as lines; empty text means no label.
fn text_lines(text: &str) -> Vec<String> {
    let lines = crate::mermaid::label(text, false);
    if lines.iter().all(String::is_empty) {
        Vec::new()
    } else {
        lines
    }
}

/// A Mermaid `sequenceDiagram` as a sequence.
pub(crate) fn sequence(m: &Value) -> Sequence {
    let participants: Vec<Participant> = (list(&m["actors"]).iter())
        .map(|a| Participant { id: get(a, "id").into(), lines: text_lines(get(a, "description")), actor: get(a, "type") == "actor" })
        .collect();
    let at = |id: &str| participants.iter().position(|a| a.id == id);
    let titled = |keyword: &str, text: &str| if text.trim().is_empty() { keyword.to_string() } else { format!("{keyword} {}", text.trim()) };
    let mut items = Vec::new();
    for msg in list(&m["messages"]) {
        let (t, text) = (msg["type"].as_i64().unwrap_or(-1), get(msg, "message"));
        let item = if let Some(&(_, keyword)) = FRAMES.iter().find(|f| f.0 == t) {
            // A `rect`'s text is just its colour.
            Item::Start(if keyword == "rect" { keyword.to_string() } else { titled(keyword, text) })
        } else if let Some(&(_, keyword)) = DIVIDERS.iter().find(|d| d.0 == t) {
            Item::Divider(titled(keyword, text))
        } else if ENDS.contains(&t) {
            Item::End
        } else if let (Some(a), Some(z)) = (at(get(msg, "from")), at(get(msg, "to"))) {
            let lines = text_lines(text);
            match t {
                2 => {
                    let side = [NoteSide::Left, NoteSide::Right].get(msg["placement"].as_u64().unwrap_or(2) as usize).copied();
                    Item::Note(Note { side: side.unwrap_or(NoteSide::Over), first: a.min(z), last: a.max(z), lines })
                }
                0 | 1 | 3..=6 | 24 | 25 => {
                    let head = match t {
                        3 | 4 => Head::Cross,
                        5 | 6 => Head::Open,
                        _ => Head::Arrow,
                    };
                    Item::Message(Message { from: a, to: z, lines, dotted: matches!(t, 1 | 4 | 6 | 25), head })
                }
                _ => continue,
            }
        } else {
            continue;
        };
        items.push(item);
    }
    Sequence { participants, items }
}

// ---- Layout -----------------------------------------------------------------

fn lines_width(lines: &[String]) -> i64 {
    lines.iter().map(|l| width(l)).max().unwrap_or(0)
}

/// Left/right half-widths of a box centred on a column.
fn halves(width: i64) -> (i64, i64) {
    let left = (width - 1) / 2;
    (left, width - 1 - left)
}

/// Columns a message label needs between two neighbouring lifelines.
fn label_room(m: &Message) -> i64 {
    (lines_width(&m.lines) + 4).max(6)
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

/// A frame opened by `loop`/`alt`/..., spanning columns `left..=right` from
/// item `top` to item `bottom` (`None` if never closed), with its dividers.
struct Frame {
    title: String,
    left: i64,
    right: i64,
    top: usize,
    bottom: Option<usize>,
    dividers: Vec<(usize, String)>,
}

/// Columns `lo..=hi` covered so far, if anything.
type Extent = Option<(i64, i64)>;

/// Frames in order of their `Start`, fitted around their contents.
fn frames(items: &[Item], x: &[i64]) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::new();
    // Open frames: index, content extent so far, widest title or divider.
    let mut stack: Vec<(usize, Extent, i64)> = Vec::new();
    let grow = |e: &mut Extent, (lo, hi)| *e = Some(e.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Start(title) => {
                stack.push((out.len(), None, width(title)));
                out.push(Frame { title: title.clone(), left: 0, right: 0, top: i, bottom: None, dividers: Vec::new() });
            }
            Item::Divider(title) => {
                if let Some(open) = stack.last_mut() {
                    open.2 = open.2.max(width(title));
                    out[open.0].dividers.push((i, title.clone()));
                }
            }
            Item::End => {
                let Some((f, extent, title)) = stack.pop() else { continue };
                let (lo, hi) = extent.unwrap_or((x[0], x[0]));
                let (left, right) = (lo - 2, (hi + 2).max(lo - 2 + title + 3));
                (out[f].left, out[f].right, out[f].bottom) = (left, right, Some(i));
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
fn columns(seq: &Sequence, boxw: &[i64]) -> Vec<i64> {
    let n = seq.participants.len();
    let mut gap: Vec<i64> = (0..n.saturating_sub(1)).map(|i| halves(boxw[i]).1 + halves(boxw[i + 1]).0 + 3).collect();
    // What each item needs: the gaps between lifelines lo and hi together at
    // least `w` wide (span), or one of them `w` wide so a label fits without
    // covering a lifeline (gap).
    let (mut spans, mut gaps) = (Vec::new(), Vec::new());
    for item in &seq.items {
        match item {
            Item::Message(m) if m.from == m.to && m.from + 1 < n => spans.push((m.from, m.from + 1, self_reach(m) + 3)),
            Item::Message(m) if m.from != m.to => {
                let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
                if hi - lo == 1 {
                    spans.push((lo, hi, label_room(m)));
                } else if !m.lines.is_empty() {
                    gaps.push((lo, hi, label_room(m)));
                }
            }
            Item::Note(note) => {
                let (w, i, j) = (note_width(note), note.first, note.last);
                let (lh, rh) = halves(w);
                match note.side {
                    NoteSide::Right if i + 1 < n => spans.push((i, i + 1, w + 3)),
                    NoteSide::Left if i > 0 => spans.push((i - 1, i, w + 3)),
                    // Each half clears the note's border and the neighbour's box.
                    NoteSide::Over if i == j => {
                        if i > 0 {
                            spans.push((i - 1, i, lh + 2 + halves(boxw[i - 1]).1.min(lh)));
                        }
                        if i + 1 < n {
                            spans.push((i, i + 1, rh + 2 + halves(boxw[i + 1]).0.min(rh)));
                        }
                    }
                    NoteSide::Over => spans.push((i, j, w - 5)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // Narrow spans first, so a wide one only adds what is still missing,
    // shared evenly; then give each long label the widest gap of its span.
    spans.sort_by_key(|&(lo, hi, _)| hi - lo);
    for (lo, hi, w) in spans {
        let (missing, k) = ((w - gap[lo..hi].iter().sum::<i64>()).max(0), (hi - lo) as i64);
        for (step, g) in gap[lo..hi].iter_mut().enumerate() {
            *g += missing / k + i64::from((step as i64) < missing % k);
        }
    }
    for (lo, hi, w) in gaps {
        if gap[lo..hi].iter().all(|&g| g < w) {
            let widest = (lo..hi).rev().max_by_key(|&i| gap[i]).unwrap_or(lo);
            gap[widest] = w;
        }
    }
    // Push lifelines apart until no frame border sits on (or next to) one:
    // a left border widens the gap after that lifeline, a right border the gap before.
    let place = |gap: &[i64]| -> Vec<i64> { (0..n).map(|i| gap[..i].iter().sum()).collect() };
    for _ in 0..200 {
        let x = place(&gap);
        let near = |border: i64, m: usize| (border - x[m]).abs() <= 1;
        let bump = |f: &Frame, m: usize| {
            let left = near(f.left, m) && m + 1 < n;
            (left || near(f.right, m) && m > 0).then(|| if left { m } else { m - 1 })
        };
        let touching = |f: &Frame| -> Vec<usize> { (0..n).filter_map(|m| bump(f, m)).collect() };
        let Some(bumps) = frames(&seq.items, &x).iter().map(touching).find(|b| !b.is_empty()) else { break };
        for m in bumps {
            gap[m] += 1;
        }
    }
    place(&gap)
}

/// A box `w` wide with centred lines.
fn boxed(p: &mut Painter, (left, top): (i64, i64), w: i64, lines: &[String], corners: Corners) {
    p.rect((left, top), (w, lines.len() as i64 + 2), Stroke::LIGHT, corners, Layer::Main);
    for (i, line) in lines.iter().enumerate() {
        p.text(left + 1 + (w - 2 - width(line)).div_euclid(2), top + 1 + i as i64, line);
    }
}

/// A message's label and arrow, starting at `row`.
fn draw_message(p: &mut Painter, m: &Message, row: i64, x: &[i64]) {
    let (a, b, right, own) = (x[m.from], x[m.to], x[m.to] >= x[m.from], m.from == m.to);
    let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
    // The label sits in the first gap of its span wide enough for it.
    let gap = (lo..hi).find(|&i| x[i + 1] - x[i] >= label_room(m)).unwrap_or(lo);
    let label_at = if own { a } else { x[gap] };
    for (k, line) in m.lines.iter().enumerate() {
        p.text(label_at + 2, row + k as i64, line);
    }
    let r = row + m.lines.len() as i64;
    let step = if right { 1 } else { -1 };
    let (mut pts, tip) =
        if own { (vec![(a, r), (a + 3, r), (a + 3, r + 1)], (a + 1, r + 1)) } else { (vec![(a, r)], (b - step, r)) };
    pts.push(if m.head == Head::Open { (if own { a } else { b }, tip.1) } else { tip });
    let net = p.net();
    p.line(&pts, net, Stroke::light(if m.dotted { Style::Dotted } else { Style::Solid }), Layer::Main);
    match m.head {
        Head::Arrow => p.symbol(tip.0, tip.1, if right && !own { '►' } else { '◄' }),
        Head::Cross => p.symbol(tip.0, tip.1, 'x'),
        Head::Open => {}
    }
}

/// Render a sequence diagram as text.
#[must_use]
pub fn render_sequence(seq: &Sequence) -> String {
    if seq.participants.is_empty() {
        return String::new();
    }
    let boxw: Vec<i64> = seq.participants.iter().map(|p| lines_width(&p.lines) + 4).collect();
    let boxh: Vec<i64> = seq.participants.iter().map(|p| p.lines.len() as i64 + 2).collect();
    let x = columns(seq, &boxw);
    // The row each item starts on, then the footer boxes' row.
    let mut rows = vec![boxh.iter().max().unwrap_or(&2) + 1];
    for item in &seq.items {
        let height = match item {
            Item::Message(m) => m.lines.len() as i64 + 1 + i64::from(m.from == m.to),
            Item::Note(n) => n.lines.len() as i64 + 2,
            _ => 1,
        };
        rows.push(rows[rows.len() - 1] + height);
    }
    let footer = rows.pop().expect("starts with the first item's row") + 1;
    let mut p = Painter::new();
    for (i, &xi) in x.iter().enumerate() {
        let net = p.net();
        p.line(&[(xi, boxh[i] - 1), (xi, footer)], net, Stroke::LIGHT, Layer::Main);
    }
    // Block frames, on the under layer so lifelines and messages run through.
    for f in frames(&seq.items, &x) {
        let (net, l, r) = (p.net(), f.left, f.right);
        for (i, title) in &f.dividers {
            p.line(&[(l, rows[*i]), (r, rows[*i])], net, Stroke::DOTTED, Layer::Under);
            p.text(l + 1, rows[*i], &format!(" {title} "));
        }
        if let Some(bottom) = f.bottom {
            let (top, bottom) = (rows[f.top], rows[bottom]);
            p.line(&[(l, top), (r, top), (r, bottom), (l, bottom), (l, top)], net, Stroke::DOTTED, Layer::Under);
            p.text(l + 1, top, &format!(" {} ", f.title));
        }
    }
    for (item, &row) in seq.items.iter().zip(&rows) {
        match item {
            Item::Message(m) => draw_message(&mut p, m, row, &x),
            Item::Note(n) => {
                // Notes are opaque: lifelines and frames stop at their border.
                let (lo, hi) = item_extent(item, &x).expect("notes have an extent");
                p.clear((lo, row), (hi, row + n.lines.len() as i64 + 1));
                boxed(&mut p, (lo, row), hi - lo + 1, &n.lines, Corners::Square);
            }
            _ => {}
        }
    }
    for (i, part) in seq.participants.iter().enumerate() {
        let corners = if part.actor { Corners::Round } else { Corners::Square };
        let left = x[i] - halves(boxw[i]).0;
        boxed(&mut p, (left, 0), boxw[i], &part.lines, corners);
        boxed(&mut p, (left, footer), boxw[i], &part.lines, corners);
    }
    p.finish()
}

/// Copy of `seq` with participant, message and note text wrapped at `max`.
#[must_use]
pub fn wrap_sequence(seq: &Sequence, max: usize) -> Sequence {
    let mut s = seq.clone();
    s.participants.iter_mut().for_each(|p| wrap_all(&mut p.lines, max));
    for item in &mut s.items {
        match item {
            Item::Message(Message { lines, .. }) | Item::Note(Note { lines, .. }) => wrap_all(lines, max),
            _ => {}
        }
    }
    s
}

/// Render within `max_width` columns (0 = unlimited), wrapping text ever
/// narrower ([`WRAP_WIDTHS`]) if needed.
#[must_use]
pub fn render_sequence_fit(seq: &Sequence, max_width: usize) -> Fit {
    let wrapped = |w: Option<usize>| w.map_or_else(|| seq.clone(), |w| wrap_sequence(seq, w));
    first_fit(max_width, WRAP_WIDTHS.into_iter().map(|w| render_sequence(&wrapped(w))))
}
