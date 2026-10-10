//! Sequence diagrams: a subset of Mermaid's `sequenceDiagram`.
//!
//! Participants sit in a row with lifelines running down; each message is a
//! label row above a horizontal arrow. Columns are spaced so every label fits
//! its span, then `loop`/`alt`/`opt`/`par` frames are fitted around their
//! contents, widening gaps until no frame border lands on a lifeline.

use crate::parse::clean_label;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthChar;

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
    Start { kind: String, text: String },
    /// `else` / `and` inside a frame.
    Divider { kind: String, text: String },
    End,
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub participants: Vec<Participant>,
    pub items: Vec<Item>,
}

/// A rendered sequence diagram and whether it met the column budget.
pub struct SeqFitted {
    pub text: String,
    pub width: usize,
    pub fits: bool,
}

struct Patterns {
    participant: Regex,
    message: Regex,
    note: Regex,
    start: Regex,
    divider: Regex,
    ignored: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let rx = |s: &str| Regex::new(s).unwrap();
        Patterns {
            participant: rx(r"(?i)^(participant|actor)\s+([^\s]+?)(?:\s+as\s+(.+))?$"),
            message: rx(r"^([^\s:+\-][^\s:]*?)\s*(-->>|->>|--x|-x|--\)|-\)|-->|->)\s*[+\-]?\s*([^\s:]+)\s*(?::(.*))?$"),
            note: rx(r"(?i)^note\s+(left of|right of|over)\s+([^\s,:]+)(?:\s*,\s*([^\s,:]+))?\s*:(.*)$"),
            start: rx(r"^(loop|alt|opt|par|critical|break|rect|box)(?:\s+(.*))?$"),
            divider: rx(r"^(else|and|option)(?:\s+(.*))?$"),
            ignored: rx(r"(?i)^(sequenceDiagram$|autonumber\b|activate\b|deactivate\b|title\b|acc(Title|Descr)\b)"),
        }
    })
}

/// True if the first meaningful line of `text` is `sequenceDiagram`.
pub fn is_sequence(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with('#'))
        .is_some_and(|l| l.eq_ignore_ascii_case("sequenceDiagram"))
}

fn label(text: Option<&str>) -> Vec<String> {
    let lines = clean_label(text.unwrap_or(""));
    if lines.iter().all(|l| l.is_empty()) {
        Vec::new()
    } else {
        lines
    }
}

struct Builder {
    participants: Vec<Participant>,
    index: HashMap<String, usize>,
}

impl Builder {
    fn get(&mut self, id: &str) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        self.participants.push(Participant { id: id.to_string(), lines: vec![id.to_string()], actor: false });
        self.index.insert(id.to_string(), self.participants.len() - 1);
        self.participants.len() - 1
    }
}

/// Parse sequence diagram source.
pub fn parse_sequence(text: &str) -> Result<Sequence, String> {
    let p = patterns();
    let mut b = Builder { participants: Vec::new(), index: HashMap::new() };
    let mut items = Vec::new();
    let mut open: Vec<(String, usize)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let lineno = n + 1;
        let line = raw.trim().trim_end_matches(';').trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with('#') || p.ignored.is_match(line) {
            continue;
        }
        if let Some(c) = p.participant.captures(line) {
            let i = b.get(&c[2]);
            let part = &mut b.participants[i];
            part.actor = c[1].eq_ignore_ascii_case("actor");
            if let Some(alias) = c.get(3) {
                part.lines = label(Some(alias.as_str()));
            }
        } else if let Some(c) = p.note.captures(line) {
            let side = match c[1].to_lowercase().as_str() {
                "left of" => NoteSide::Left,
                "right of" => NoteSide::Right,
                _ => NoteSide::Over,
            };
            let a = b.get(&c[2]);
            let z = c.get(3).map_or(a, |m| b.get(m.as_str()));
            if side != NoteSide::Over && a != z {
                return Err(format!("line {lineno}: only `Note over` can span two participants"));
            }
            let lines = label(c.get(4).map(|m| m.as_str()));
            items.push(Item::Note(Note { side, first: a.min(z), last: a.max(z), lines }));
        } else if line == "end" {
            if open.pop().is_none() {
                return Err(format!("line {lineno}: `end` without an open block"));
            }
            items.push(Item::End);
        } else if let Some(c) = p.start.captures(line) {
            open.push((c[1].to_string(), lineno));
            items.push(Item::Start { kind: c[1].to_string(), text: c.get(2).map_or("", |m| m.as_str()).trim().to_string() });
        } else if let Some(c) = p.divider.captures(line) {
            if open.is_empty() {
                return Err(format!("line {lineno}: `{}` outside a block", &c[1]));
            }
            items.push(Item::Divider { kind: c[1].to_string(), text: c.get(2).map_or("", |m| m.as_str()).trim().to_string() });
        } else if let Some(c) = p.message.captures(line) {
            let from = b.get(&c[1]);
            let to = b.get(&c[3]);
            let arrow = &c[2];
            let head = if arrow.ends_with('x') {
                Head::Cross
            } else if arrow.ends_with(">>") || arrow.ends_with(')') {
                Head::Arrow
            } else {
                Head::Open
            };
            let dotted = arrow.starts_with("--");
            items.push(Item::Message(Message { from, to, lines: label(c.get(4).map(|m| m.as_str())), dotted, head }));
        } else {
            return Err(format!("line {lineno}: cannot parse {:?} (expected a message like A->>B: text)", line));
        }
    }
    if let Some((kind, lineno)) = open.pop() {
        return Err(format!("line {lineno}: `{kind}` is never closed with `end`"));
    }
    Ok(Sequence { participants: b.participants, items })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Placeholder for the right half of a double-width character.
const WIDE: char = '\0';

struct Glyphs {
    life: char,
    h: char,
    dotted: char,
    tee_right: char,
    tee_left: char,
    head_right: char,
    head_left: char,
    cross: char,
    box_chars: [char; 6],
    actor_chars: [char; 6],
    tee_down: char,
    tee_up: char,
    frame: [char; 6], // tl tr bl br horizontal vertical
    frame_tee: [char; 2],
    loop_chars: [char; 2], // top-right, bottom-right corners of a self-message
}

impl Glyphs {
    fn new(ascii: bool) -> Self {
        if ascii {
            Glyphs {
                life: '|',
                h: '-',
                dotted: '.',
                tee_right: '+',
                tee_left: '+',
                head_right: '>',
                head_left: '<',
                cross: 'x',
                box_chars: ['+', '+', '+', '+', '-', '|'],
                actor_chars: ['.', '.', '\'', '\'', '-', '|'],
                tee_down: '+',
                tee_up: '+',
                frame: ['+', '+', '+', '+', '.', ':'],
                frame_tee: ['+', '+'],
                loop_chars: ['+', '+'],
            }
        } else {
            Glyphs {
                life: '│',
                h: '─',
                dotted: '┄',
                tee_right: '├',
                tee_left: '┤',
                head_right: '►',
                head_left: '◄',
                cross: 'x',
                box_chars: ['┌', '┐', '└', '┘', '─', '│'],
                actor_chars: ['╭', '╮', '╰', '╯', '─', '│'],
                tee_down: '┬',
                tee_up: '┴',
                frame: ['┌', '┐', '└', '┘', '┄', '┆'],
                frame_tee: ['├', '┤'],
                loop_chars: ['┐', '┘'],
            }
        }
    }
}

fn char_width(c: char) -> i64 {
    c.width().unwrap_or(1).max(1) as i64
}

fn text_width(s: &str) -> i64 {
    s.chars().map(char_width).sum()
}

fn lines_width(lines: &[String]) -> i64 {
    lines.iter().map(|l| text_width(l)).max().unwrap_or(0)
}

struct Canvas {
    cells: HashMap<(i64, i64), char>,
}

impl Canvas {
    fn put(&mut self, x: i64, y: i64, ch: char) {
        self.cells.insert((x, y), ch);
    }

    fn text(&mut self, mut x: i64, y: i64, s: &str) {
        for ch in s.chars() {
            self.put(x, y, ch);
            if char_width(ch) == 2 {
                self.put(x + 1, y, WIDE);
            }
            x += char_width(ch);
        }
    }

    fn get(&self, x: i64, y: i64) -> Option<char> {
        self.cells.get(&(x, y)).copied()
    }

    fn boxed(&mut self, left: i64, top: i64, width: i64, lines: &[String], chars: [char; 6]) {
        let [tl, tr, bl, br, h, v] = chars;
        let bar: String = std::iter::repeat(h).take((width - 2).max(0) as usize).collect();
        self.text(left, top, &format!("{tl}{bar}{tr}"));
        for (i, line) in lines.iter().enumerate() {
            let pad = (width - 2 - text_width(line)).div_euclid(2);
            let fill = (width - 2 - pad - text_width(line)).max(0) as usize;
            self.text(left, top + 1 + i as i64, &format!("{v}{}{line}{}{v}", " ".repeat(pad.max(0) as usize), " ".repeat(fill)));
        }
        self.text(left, top + 1 + lines.len() as i64, &format!("{bl}{bar}{br}"));
    }

    fn finish(self) -> String {
        if self.cells.is_empty() {
            return String::new();
        }
        let min_x = self.cells.keys().map(|c| c.0).min().unwrap();
        let max_x = self.cells.keys().map(|c| c.0).max().unwrap();
        let max_y = self.cells.keys().map(|c| c.1).max().unwrap();
        let mut rows = vec![vec![' '; (max_x - min_x + 1) as usize]; (max_y + 1) as usize];
        for (&(x, y), &ch) in &self.cells {
            rows[y as usize][(x - min_x) as usize] = ch;
        }
        rows.iter()
            .map(|r| r.iter().filter(|&&c| c != WIDE).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A frame opened by `loop`/`alt`/... and its horizontal extent.
struct Frame {
    title: String,
    dividers: Vec<String>,
    left: i64,
    right: i64,
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
    match item {
        Item::Message(m) if m.from == m.to => Some((x[m.from], x[m.from] + self_reach(m))),
        Item::Message(m) => Some((x[m.from].min(x[m.to]), x[m.from].max(x[m.to]))),
        Item::Note(n) => {
            let w = note_width(n);
            Some(match n.side {
                NoteSide::Right => (x[n.first] + 2, x[n.first] + 1 + w),
                NoteSide::Left => (x[n.first] - 1 - w, x[n.first] - 2),
                NoteSide::Over if n.first == n.last => {
                    let (lh, rh) = halves(w);
                    (x[n.first] - lh, x[n.first] + rh)
                }
                NoteSide::Over => {
                    let bw = (x[n.last] - x[n.first] + 5).max(w);
                    (x[n.first] - 2, x[n.first] - 3 + bw)
                }
            })
        }
        _ => None,
    }
}

fn block_title(kind: &str, text: &str) -> String {
    if text.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {text}")
    }
}

/// Frames in order of their `Start`, with extents fitted around contents.
fn frames(items: &[Item], x: &[i64]) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::new();
    let mut stack: Vec<(usize, i64, i64)> = Vec::new();
    for item in items {
        match item {
            Item::Start { kind, text } => {
                out.push(Frame { title: block_title(kind, text), dividers: Vec::new(), left: 0, right: 0 });
                stack.push((out.len() - 1, i64::MAX, i64::MIN));
            }
            Item::Divider { kind, text } => {
                if let Some(&(f, _, _)) = stack.last() {
                    out[f].dividers.push(block_title(kind, text));
                }
            }
            Item::End => {
                let (f, mut lo, mut hi) = stack.pop().expect("parser balances blocks");
                if lo > hi {
                    lo = x[0];
                    hi = x[0];
                }
                let left = lo - 2;
                let titles = std::iter::once(&out[f].title).chain(&out[f].dividers);
                let need = titles.map(|t| text_width(t)).max().unwrap_or(0) + 3;
                let right = (hi + 2).max(left + need);
                out[f].left = left;
                out[f].right = right;
                if let Some(parent) = stack.last_mut() {
                    parent.1 = parent.1.min(left);
                    parent.2 = parent.2.max(right);
                }
            }
            _ => {
                if let (Some(parent), Some((lo, hi))) = (stack.last_mut(), item_extent(item, x)) {
                    parent.1 = parent.1.min(lo);
                    parent.2 = parent.2.max(hi);
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
    let mut gap: Vec<i64> = (0..n.saturating_sub(1))
        .map(|i| halves(boxw[i]).1 + halves(boxw[i + 1]).0 + 3)
        .collect();
    let mut spans: Vec<(usize, usize, i64)> = Vec::new();
    let mut need = |gap: &mut Vec<i64>, lo: usize, hi: usize, d: i64| {
        if hi == lo + 1 {
            gap[lo] = gap[lo].max(d);
        } else if hi > lo + 1 {
            spans.push((lo, hi, d));
        }
    };
    for item in &seq.items {
        match item {
            Item::Message(m) if m.from == m.to => {
                if m.from + 1 < n {
                    need(&mut gap, m.from, m.from + 1, self_reach(m) + 3);
                }
            }
            Item::Message(m) => {
                let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
                need(&mut gap, lo, hi, (lines_width(&m.lines) + 4).max(6));
            }
            Item::Note(note) => {
                let w = note_width(note);
                let (i, j) = (note.first, note.last);
                match note.side {
                    NoteSide::Right if i + 1 < n => need(&mut gap, i, i + 1, w + 3),
                    NoteSide::Left if i > 0 => need(&mut gap, i - 1, i, w + 3),
                    NoteSide::Over if i == j => {
                        let (lh, rh) = halves(w);
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
        let have: i64 = gap[lo..hi].iter().sum();
        if have < d {
            let k = (hi - lo) as i64;
            for (step, g) in gap[lo..hi].iter_mut().enumerate() {
                *g += (d - have) / k + i64::from((step as i64) < (d - have) % k);
            }
        }
    }
    let place = |gap: &[i64]| -> Vec<i64> {
        let mut x = vec![0i64; n];
        for i in 1..n {
            x[i] = x[i - 1] + gap[i - 1];
        }
        x
    };
    // Push lifelines apart until no frame border sits on (or next to) one.
    for _ in 0..200 {
        let x = place(&gap);
        let mut bumped = false;
        for f in frames(&seq.items, &x) {
            for (m, &xm) in x.iter().enumerate() {
                if (f.left - xm).abs() <= 1 && m + 1 < n {
                    gap[m] += 1;
                    bumped = true;
                } else if (f.right - xm).abs() <= 1 && m > 0 {
                    gap[m - 1] += 1;
                    bumped = true;
                }
            }
            if bumped {
                break;
            }
        }
        if !bumped {
            break;
        }
    }
    place(&gap)
}

/// Render a sequence diagram as text.
pub fn render_sequence(seq: &Sequence, ascii: bool) -> String {
    if seq.participants.is_empty() {
        return String::new();
    }
    let g = Glyphs::new(ascii);
    let x = columns(seq);
    let frame_list = frames(&seq.items, &x);
    let mut cv = Canvas { cells: HashMap::new() };

    let boxw: Vec<i64> = seq.participants.iter().map(|p| lines_width(&p.lines) + 4).collect();
    let boxh: Vec<i64> = seq.participants.iter().map(|p| p.lines.len() as i64 + 2).collect();
    let header = *boxh.iter().max().unwrap();

    // First pass: assign a row to every item.
    let mut y = header + 1;
    let mut rows = Vec::with_capacity(seq.items.len());
    for item in &seq.items {
        rows.push(y);
        y += match item {
            Item::Message(m) if m.from == m.to => m.lines.len() as i64 + 2,
            Item::Message(m) => m.lines.len() as i64 + 1,
            Item::Note(n) => n.lines.len() as i64 + 2,
            _ => 1,
        };
    }
    let footer = y + 1;

    for (i, &xi) in x.iter().enumerate() {
        for row in boxh[i]..footer {
            cv.put(xi, row, g.life);
        }
    }

    // Frames: borders give way to lifelines, which run straight through.
    let mut frame_rows: Vec<(usize, i64, char, char, String)> = Vec::new();
    let mut stack: Vec<(usize, i64)> = Vec::new();
    let mut next = 0;
    for (item, &row) in seq.items.iter().zip(&rows) {
        match item {
            Item::Start { .. } => {
                let f = &frame_list[next];
                frame_rows.push((next, row, g.frame[0], g.frame[1], f.title.clone()));
                stack.push((next, row));
                next += 1;
            }
            Item::Divider { kind, text } => {
                if let Some(&(f, _)) = stack.last() {
                    frame_rows.push((f, row, g.frame_tee[0], g.frame_tee[1], block_title(kind, text)));
                }
            }
            Item::End => {
                let (f, top) = stack.pop().expect("parser balances blocks");
                frame_rows.push((f, row, g.frame[2], g.frame[3], String::new()));
                let fr = &frame_list[f];
                for r in top + 1..row {
                    for col in [fr.left, fr.right] {
                        if cv.get(col, r).is_none() {
                            cv.put(col, r, g.frame[5]);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for (f, row, l, r, title) in &frame_rows {
        let fr = &frame_list[*f];
        for col in fr.left + 1..fr.right {
            if !x.contains(&col) {
                cv.put(col, *row, g.frame[4]);
            }
        }
        cv.put(fr.left, *row, *l);
        cv.put(fr.right, *row, *r);
        if !title.is_empty() {
            let at = fr.left + 2;
            cv.put(at - 1, *row, ' ');
            cv.text(at, *row, title);
            cv.put(at + text_width(title), *row, ' ');
        }
    }

    for (item, &row) in seq.items.iter().zip(&rows) {
        match item {
            Item::Message(m) if m.from == m.to => {
                let xi = x[m.from];
                for (k, line) in m.lines.iter().enumerate() {
                    cv.text(xi + 2, row + k as i64, line);
                }
                let r = row + m.lines.len() as i64;
                let h = if m.dotted { g.dotted } else { g.h };
                cv.put(xi, r, g.tee_right);
                cv.put(xi + 1, r, h);
                cv.put(xi + 2, r, h);
                cv.put(xi + 3, r, g.loop_chars[0]);
                cv.put(xi + 1, r + 1, match m.head {
                    Head::Arrow => g.head_left,
                    Head::Cross => g.cross,
                    Head::Open => h,
                });
                cv.put(xi + 2, r + 1, h);
                cv.put(xi + 3, r + 1, g.loop_chars[1]);
                if m.head == Head::Open {
                    cv.put(xi, r + 1, g.tee_right);
                }
            }
            Item::Message(m) => {
                let (a, b) = (x[m.from], x[m.to]);
                let (lo, hi) = (a.min(b), a.max(b));
                for (k, line) in m.lines.iter().enumerate() {
                    cv.text(lo + 2, row + k as i64, line);
                }
                let r = row + m.lines.len() as i64;
                let h = if m.dotted { g.dotted } else { g.h };
                for col in lo + 1..hi {
                    if !x.contains(&col) {
                        cv.put(col, r, h);
                    }
                }
                let right = b > a;
                cv.put(a, r, if right { g.tee_right } else { g.tee_left });
                let tip = if right { b - 1 } else { b + 1 };
                match m.head {
                    Head::Arrow => cv.put(tip, r, if right { g.head_right } else { g.head_left }),
                    Head::Cross => cv.put(tip, r, g.cross),
                    Head::Open => cv.put(b, r, if right { g.tee_left } else { g.tee_right }),
                }
            }
            Item::Note(n) => {
                let (lo, hi) = item_extent(item, &x).expect("notes have an extent");
                cv.boxed(lo, row, hi - lo + 1, &n.lines, g.box_chars);
            }
            _ => {}
        }
    }

    for (i, p) in seq.participants.iter().enumerate() {
        let (lh, _) = halves(boxw[i]);
        let chars = if p.actor { g.actor_chars } else { g.box_chars };
        let left = x[i] - lh;
        cv.boxed(left, 0, boxw[i], &p.lines, chars);
        cv.put(x[i], boxh[i] - 1, g.tee_down);
        cv.boxed(left, footer, boxw[i], &p.lines, chars);
        cv.put(x[i], footer, g.tee_up);
    }
    cv.finish()
}

fn wrap_line(line: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        if !cur.is_empty() && (text_width(&cur) + 1 + text_width(word)) as usize > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn wrap_lines(lines: &mut Vec<String>, max: usize) {
    *lines = lines.iter().flat_map(|l| wrap_line(l, max)).collect();
}

/// Copy of `seq` with participant, message and note text wrapped at `max`.
pub fn wrap_sequence(seq: &Sequence, max: usize) -> Sequence {
    let mut s = seq.clone();
    for p in &mut s.participants {
        wrap_lines(&mut p.lines, max);
    }
    for item in &mut s.items {
        match item {
            Item::Message(m) => wrap_lines(&mut m.lines, max),
            Item::Note(n) => wrap_lines(&mut n.lines, max),
            _ => {}
        }
    }
    s
}

/// Render within `max_width` columns (0 = unlimited), wrapping text at 24,
/// then 16, then 12 columns if needed. Returns the narrowest attempt if
/// nothing fits.
pub fn render_sequence_fit(seq: &Sequence, ascii: bool, max_width: usize) -> SeqFitted {
    let mut best: Option<SeqFitted> = None;
    for wrap in [None, Some(24), Some(16), Some(12)] {
        let s = match wrap {
            None => seq.clone(),
            Some(w) => wrap_sequence(seq, w),
        };
        let text = render_sequence(&s, ascii);
        let width = text.lines().map(unicode_width::UnicodeWidthStr::width).max().unwrap_or(0);
        if max_width == 0 || width <= max_width {
            return SeqFitted { text, width, fits: true };
        }
        if best.as_ref().map_or(true, |b| width < b.width) {
            best = Some(SeqFitted { text, width, fits: false });
        }
    }
    best.expect("at least one render attempt")
}
