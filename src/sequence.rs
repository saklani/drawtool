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

impl Sequence {
    /// Check what [`parse_sequence`] guarantees, for a sequence built by hand.
    ///
    /// # Errors
    ///
    /// A message or note naming a participant that doesn't exist, a note whose
    /// `first` comes after its `last` (or that spans two participants without
    /// being `Over`), or a `Divider`/`End` outside a frame or a `Start` never
    /// closed.
    pub fn validate(&self) -> Result<(), Error> {
        let n = self.participants.len();
        let mut open = 0;
        for (i, item) in self.items.iter().enumerate() {
            let problem = match item {
                Item::Message(Message { from, to, .. }) | Item::Note(Note { first: from, last: to, .. })
                    if *from.max(to) >= n =>
                {
                    Some(format!("no participant {} (there are {n})", from.max(to)))
                }
                Item::Note(note) if note.first > note.last => Some("a note's `first` is after its `last`".into()),
                Item::Note(note) if note.side != NoteSide::Over && note.first != note.last => {
                    Some("only `Over` notes can span two participants".into())
                }
                Item::Start { .. } => {
                    open += 1;
                    None
                }
                Item::Divider { .. } | Item::End if open == 0 => Some("outside a frame".into()),
                Item::End => {
                    open -= 1;
                    None
                }
                _ => None,
            };
            if let Some(problem) = problem {
                return Err(Error::new(format!("item {i}: {problem}")));
            }
        }
        if open > 0 {
            return Err(Error::new("a frame `Start` is never closed with `End`"));
        }
        Ok(())
    }
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

/// Label text as lines; empty text means no label.
fn text_lines(text: Option<&str>) -> Vec<String> {
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
    // Open blocks: kind, and where they start (for "never closed").
    let mut open: Vec<(String, usize, usize)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let lineno = n + 1;
        let line = strip_comment(raw.trim()).trim_end_matches(';').trim();
        let opt = |m: Option<regex::Match>| m.map_or("", |m| m.as_str()).trim().to_string();
        // 1-based character column of byte offset `at` in `line`, in the original line.
        let indent = raw[..raw.len() - raw.trim_start().len()].chars().count();
        let col = |at: usize| indent + line[..at].chars().count() + 1;
        if line.is_empty() || line.starts_with("%%") || line.starts_with('#') || p.ignored.is_match(line) {
            continue;
        } else if let Some(c) = p.participant.captures(line) {
            let i = get(&c[2], &mut parts);
            parts[i].actor = c[1].eq_ignore_ascii_case("actor");
            if let Some(alias) = c.get(3) {
                parts[i].lines = text_lines(Some(alias.as_str()));
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
                let at = c.get(3).map_or(0, |m| m.start());
                return Err(Error::at_column(lineno, col(at), "only `Note over` can span two participants"));
            }
            let lines = text_lines(c.get(4).map(|m| m.as_str()));
            items.push(Item::Note(Note { side, first: a.min(z), last: a.max(z), lines }));
        } else if line == "end" {
            let (kind, _, _) =
                open.pop().ok_or_else(|| Error::at_column(lineno, col(0), "`end` without an open block"))?;
            if kind != "box" {
                items.push(Item::End);
            }
        } else if let Some(c) = p.start.captures(line) {
            open.push((c[1].to_string(), lineno, col(0)));
            // `box` only groups participant declarations (Mermaid colours them),
            // and a `rect`'s text is just its colour: neither needs a title.
            match &c[1] {
                "box" => {}
                "rect" => items.push(Item::Start { kind: c[1].to_string(), text: String::new() }),
                kind => items.push(Item::Start { kind: kind.to_string(), text: opt(c.get(2)) }),
            }
        } else if let Some(c) = p.divider.captures(line) {
            if open.is_empty() {
                return Err(Error::at_column(lineno, col(0), format!("`{}` outside a block", &c[1])));
            }
            items.push(Item::Divider { kind: c[1].to_string(), text: opt(c.get(2)) });
        } else if let Some(c) = p.message.captures(line) {
            let (from, to, arrow) = (get(&c[1], &mut parts), get(&c[3], &mut parts), &c[2]);
            let head = match arrow {
                a if a.ends_with('x') => Head::Cross,
                a if a.ends_with(">>") || a.ends_with(')') => Head::Arrow,
                _ => Head::Open,
            };
            let lines = text_lines(c.get(4).map(|m| m.as_str()));
            items.push(Item::Message(Message { from, to, lines, dotted: arrow.starts_with("--"), head }));
        } else {
            let message = format!("cannot parse {line:?} (expected a message like A->>B: text)");
            return Err(Error::at_column(lineno, col(0), message));
        }
    }
    if let Some((kind, lineno, column)) = open.pop() {
        return Err(Error::at_column(lineno, column, format!("`{kind}` is never closed with `end`")));
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

fn box_width(p: &Participant) -> i64 {
    lines_width(&p.lines) + 4
}

/// Columns a message label needs between two neighbouring lifelines.
fn label_room(m: &Message) -> i64 {
    (lines_width(&m.lines) + 4).max(6)
}

/// Left lifeline of the first gap in `lo..hi` wide enough for a label that
/// needs `room` columns (layout guarantees one exists; else the first gap).
fn label_gap(x: &[i64], lo: usize, hi: usize, room: i64) -> usize {
    (lo..hi).find(|&i| x[i + 1] - x[i] >= room).unwrap_or(lo)
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
    let grow = |e: &mut Extent, (lo, hi): (i64, i64)| *e = Some(e.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Start { kind, text } => {
                let title = block_title(kind, text);
                stack.push((out.len(), None, width(&title)));
                out.push(Frame { title, left: 0, right: 0, top: i, bottom: None, dividers: Vec::new() });
            }
            Item::Divider { kind, text } => {
                if let Some(open) = stack.last_mut() {
                    let title = block_title(kind, text);
                    open.2 = open.2.max(width(&title));
                    out[open.0].dividers.push((i, title));
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

/// What an item needs from the gaps between lifelines `lo` and `hi`.
enum Need {
    /// The gaps together are at least this wide.
    Span(usize, usize, i64),
    /// One of the gaps is this wide, so a label fits without covering a lifeline.
    Gap(usize, usize, i64),
}

/// The spacing each message, self-message and note asks for.
fn needs(seq: &Sequence, boxw: &[i64]) -> Vec<Need> {
    let n = seq.participants.len();
    let mut out = Vec::new();
    for item in &seq.items {
        match item {
            Item::Message(m) if m.from == m.to => {
                out.extend((m.from + 1 < n).then(|| Need::Span(m.from, m.from + 1, self_reach(m) + 3)));
            }
            Item::Message(m) => {
                let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
                if hi - lo == 1 {
                    out.push(Need::Span(lo, hi, label_room(m)));
                } else if !m.lines.is_empty() {
                    out.push(Need::Gap(lo, hi, label_room(m)));
                }
            }
            Item::Note(note) => {
                let (w, i, j) = (note_width(note), note.first, note.last);
                let (lh, rh) = halves(w);
                match note.side {
                    NoteSide::Right if i + 1 < n => out.push(Need::Span(i, i + 1, w + 3)),
                    NoteSide::Left if i > 0 => out.push(Need::Span(i - 1, i, w + 3)),
                    // Each half clears the note's border and the neighbour's box.
                    NoteSide::Over if i == j => {
                        out.extend((i > 0).then(|| Need::Span(i - 1, i, lh + 2 + halves(boxw[i - 1]).1.min(lh))));
                        out.extend((i + 1 < n).then(|| Need::Span(i, i + 1, rh + 2 + halves(boxw[i + 1]).0.min(rh))));
                    }
                    NoteSide::Over => out.push(Need::Span(i, j, w - 5)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Lifeline columns: wide enough for boxes, labels and notes, with every
/// frame border clear of every lifeline.
fn columns(seq: &Sequence, boxw: &[i64]) -> Vec<i64> {
    let n = seq.participants.len();
    let mut gap: Vec<i64> = (0..n.saturating_sub(1)).map(|i| halves(boxw[i]).1 + halves(boxw[i + 1]).0 + 3).collect();
    // Narrow spans first, so a wide one only adds what is still missing,
    // shared evenly; then give each long label one gap to sit in.
    let mut needs = needs(seq, boxw);
    needs.sort_by_key(|need| match *need {
        Need::Span(lo, hi, _) => (0, hi - lo),
        Need::Gap(..) => (1, 0),
    });
    for need in needs {
        match need {
            Need::Span(lo, hi, w) => {
                let (missing, k) = ((w - gap[lo..hi].iter().sum::<i64>()).max(0), (hi - lo) as i64);
                for (step, g) in gap[lo..hi].iter_mut().enumerate() {
                    *g += missing / k + i64::from((step as i64) < missing % k);
                }
            }
            // Widen the widest gap of the span: the least extra width.
            Need::Gap(lo, hi, w) if gap[lo..hi].iter().all(|&g| g < w) => {
                let widest = (lo..hi).rev().max_by_key(|&i| gap[i]).unwrap_or(lo);
                gap[widest] = w;
            }
            Need::Gap(..) => {}
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

/// The row each item starts on, and the row of the footer boxes.
fn rows(seq: &Sequence, header: i64) -> (Vec<i64>, i64) {
    let mut rows = vec![header + 1];
    for item in &seq.items {
        let height = match item {
            Item::Message(m) => m.lines.len() as i64 + 1 + i64::from(m.from == m.to),
            Item::Note(n) => n.lines.len() as i64 + 2,
            _ => 1,
        };
        rows.push(rows[rows.len() - 1] + height);
    }
    let footer = rows.pop().expect("starts with the first item's row") + 1;
    (rows, footer)
}

/// Block frames, on the under layer so lifelines and messages run through.
fn draw_frames(p: &mut Painter, rows: &[i64], frames: &[Frame]) {
    for f in frames {
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
}

/// A message's label and arrow, starting at `row`.
fn draw_message(p: &mut Painter, m: &Message, row: i64, x: &[i64]) {
    let (a, b, right) = (x[m.from], x[m.to], x[m.to] >= x[m.from]);
    let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
    let label_at = if m.from == m.to { a } else { x[label_gap(x, lo, hi, label_room(m))] };
    for (k, line) in m.lines.iter().enumerate() {
        p.text(label_at + 2, row + k as i64, line);
    }
    let r = row + m.lines.len() as i64;
    let step = if right { 1 } else { -1 };
    let (mut pts, tip) = if m.from == m.to {
        (vec![(a, r), (a + 3, r), (a + 3, r + 1)], (a + 1, r + 1))
    } else {
        (vec![(a, r)], (b - step, r))
    };
    pts.push(if m.head == Head::Open { (if m.from == m.to { a } else { b }, tip.1) } else { tip });
    let net = p.net();
    p.line(&pts, net, Stroke::light(if m.dotted { Style::Dotted } else { Style::Solid }), Layer::Main);
    match m.head {
        Head::Arrow => p.symbol(tip.0, tip.1, if right && m.from != m.to { '►' } else { '◄' }),
        Head::Cross => p.symbol(tip.0, tip.1, 'x'),
        Head::Open => {}
    }
}

/// Render a sequence diagram as text.
///
/// # Errors
///
/// A hand-built sequence that fails [`Sequence::validate`].
pub fn render_sequence(seq: &Sequence, ascii: bool) -> Result<String, Error> {
    seq.validate()?;
    Ok(draw(seq, ascii))
}

/// [`render_sequence`] for a sequence known to be valid.
fn draw(seq: &Sequence, ascii: bool) -> String {
    if seq.participants.is_empty() {
        return String::new();
    }
    let boxw: Vec<i64> = seq.participants.iter().map(box_width).collect();
    let x = columns(seq, &boxw);
    let mut p = Painter::new(ascii);
    let boxh: Vec<i64> = seq.participants.iter().map(|p| p.lines.len() as i64 + 2).collect();
    let (rows, footer) = rows(seq, *boxh.iter().max().unwrap_or(&2));
    for (i, &xi) in x.iter().enumerate() {
        let net = p.net();
        p.line(&[(xi, boxh[i] - 1), (xi, footer)], net, Stroke::LIGHT, Layer::Main);
    }
    draw_frames(&mut p, &rows, &frames(&seq.items, &x));
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
///
/// # Errors
///
/// A hand-built sequence that fails [`Sequence::validate`].
pub fn render_sequence_fit(seq: &Sequence, ascii: bool, max_width: usize) -> Result<Fit, Error> {
    seq.validate()?;
    let wrapped = [None, Some(24), Some(16), Some(12)].into_iter();
    Ok(first_fit(max_width, wrapped.map(|w| draw(&w.map_or_else(|| seq.clone(), |w| wrap_sequence(seq, w)), ascii))))
}
