//! The one place that turns geometry into characters. Renderers draw text,
//! symbols and lines (box borders are lines too); the painter derives every
//! corner, tee and junction from which directions meet in a cell.

use crate::parse::Style;
use std::collections::{HashMap, HashSet};
use unicode_width::UnicodeWidthChar;

const U: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const R: u8 = 8;
const VERT: u8 = U | D;
const HORZ: u8 = L | R;
/// Right half of a double-width character.
const WIDE: char = '\0';

pub type Pt = (i64, i64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Weight {
    Light,
    Heavy,
    Double,
}

/// One net's line directions in a cell. Lines of the same net join; lines of
/// different nets that pass straight through each other cross.
#[derive(Clone, Copy)]
struct Arm {
    net: usize,
    bits: u8,
    weight: Weight,
    style: Style,
}

pub fn char_width(c: char) -> i64 {
    c.width().unwrap_or(1).max(1) as i64
}

pub fn width(s: &str) -> i64 {
    s.chars().map(char_width).sum()
}

/// Greedy word wrap at `max` columns. Words longer than `max` are split only
/// if `split` is set.
pub fn wrap(s: &str, max: usize, split: bool) -> Vec<String> {
    let opts = textwrap::Options::new(max.max(1))
        .break_words(split)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);
    let out: Vec<String> = textwrap::wrap(&s.split_whitespace().collect::<Vec<_>>().join(" "), opts)
        .into_iter()
        .map(String::from)
        .collect();
    if out.is_empty() {
        vec![String::new()]
    } else {
        out
    }
}

/// A rendering and whether it met the column budget.
pub struct Fit {
    pub text: String,
    pub width: usize,
    pub fits: bool,
}

/// The first rendering no wider than `max` (0 = unlimited), else the narrowest.
pub fn first_fit(max: usize, attempts: impl IntoIterator<Item = String>) -> Fit {
    let mut best: Option<Fit> = None;
    for text in attempts {
        let width = text.lines().map(width).max().unwrap_or(0) as usize;
        if max == 0 || width <= max {
            return Fit { text, width, fits: true };
        }
        if best.as_ref().map_or(true, |b| width < b.width) {
            best = Some(Fit { text, width, fits: false });
        }
    }
    best.expect("at least one attempt")
}

#[derive(Default)]
pub struct Painter {
    pub ascii: bool,
    /// Text and symbols; the flag marks symbols, which ASCII mode translates.
    glyphs: HashMap<Pt, (char, bool)>,
    /// Main lines, then the under layer (shown only in otherwise empty cells).
    lines: [HashMap<Pt, Vec<Arm>>; 2],
    round: HashSet<Pt>,
    nets: usize,
}

impl Painter {
    pub fn new(ascii: bool) -> Self {
        Painter { ascii, ..Default::default() }
    }

    /// A fresh net id.
    pub fn net(&mut self) -> usize {
        self.nets += 1;
        self.nets
    }

    pub fn text(&mut self, mut x: i64, y: i64, s: &str) {
        for ch in s.chars() {
            self.glyphs.insert((x, y), (ch, false));
            if char_width(ch) == 2 {
                self.glyphs.insert((x + 1, y), (WIDE, false));
            }
            x += char_width(ch);
        }
    }

    pub fn symbol(&mut self, x: i64, y: i64, ch: char) {
        self.glyphs.insert((x, y), (ch, true));
    }

    /// An axis-aligned polyline through `pts`.
    pub fn line(&mut self, pts: &[Pt], net: usize, weight: Weight, style: Style, under: bool) {
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            let (lo, hi) = if x0 == x1 { (y0.min(y1), y0.max(y1)) } else { (x0.min(x1), x0.max(x1)) };
            for t in lo..=hi {
                let (back, fwd) = if x0 == x1 { (U, D) } else { (L, R) };
                let bits = if t > lo { back } else { 0 } | if t < hi { fwd } else { 0 };
                let cell = if x0 == x1 { (x0, t) } else { (t, y0) };
                let arms = self.lines[usize::from(under)].entry(cell).or_default();
                match arms.iter_mut().find(|a| a.net == net) {
                    Some(a) => {
                        a.bits |= bits;
                        // A trunk shared by edges of different styles belongs to none of them.
                        if a.style != style {
                            a.style = Style::Solid;
                        }
                    }
                    None => arms.push(Arm { net, bits, weight, style }),
                }
            }
        }
    }

    /// A rectangle `w`×`h` with its top-left at (`x`, `y`).
    pub fn rect(&mut self, (x, y): Pt, (w, h): Pt, weight: Weight, style: Style, round: bool, under: bool) {
        let (x1, y1) = (x + w - 1, y + h - 1);
        let net = self.net();
        self.line(&[(x, y), (x1, y), (x1, y1), (x, y1), (x, y)], net, weight, style, under);
        if round {
            self.round.extend([(x, y), (x1, y), (x, y1), (x1, y1)]);
        }
    }

    /// Erase every line in a rectangle (for boxes drawn over other lines).
    pub fn clear(&mut self, (x0, y0): Pt, (x1, y1): Pt) {
        for layer in &mut self.lines {
            layer.retain(|&(x, y), _| x < x0 || x > x1 || y < y0 || y > y1);
        }
    }

    /// No text, symbol or main line in this cell.
    pub fn is_free(&self, cell: Pt) -> bool {
        !self.glyphs.contains_key(&cell) && !self.lines[0].contains_key(&cell)
    }

    fn straight(&self, weight: Weight, style: Style, vertical: bool) -> char {
        let pair = match (self.ascii, weight, style) {
            (false, Weight::Heavy, _) | (false, _, Style::Thick) => ['┃', '━'],
            (false, Weight::Double, _) => ['║', '═'],
            (false, _, Style::Dotted) => ['┆', '┄'],
            (false, _, Style::Solid) => ['│', '─'],
            (true, Weight::Heavy, _) => ['#', '='],
            (true, Weight::Double, _) | (true, _, Style::Thick) => ['|', '='],
            (true, _, Style::Dotted) => [':', '.'],
            (true, _, Style::Solid) => ['|', '-'],
        };
        pair[usize::from(!vertical)]
    }

    fn junction(&self, bits: u8, w: [Option<Weight>; 4], round: bool) -> char {
        let present: Vec<Weight> = w.iter().flatten().copied().collect();
        let all = |k: Weight| present.iter().all(|&p| p == k);
        let [wu, wd, wl, wr] = w;
        if self.ascii {
            return match () {
                _ if all(Weight::Heavy) => '#',
                _ if round => ['\'', '.'][usize::from(bits & D != 0)],
                _ => '+',
            };
        }
        let corner = |set: [char; 4]| match bits {
            b if b == D | R => Some(set[0]),
            b if b == D | L => Some(set[1]),
            b if b == U | R => Some(set[2]),
            b if b == U | L => Some(set[3]),
            _ => None,
        };
        let tee = |heavy: char, double: char, k: Option<Weight>| match k {
            Some(Weight::Heavy) => Some(heavy),
            Some(Weight::Double) => Some(double),
            _ => None,
        };
        let light = Some(Weight::Light);
        let found = match () {
            _ if round && all(Weight::Light) => corner(['╭', '╮', '╰', '╯']),
            _ if all(Weight::Heavy) => corner(['┏', '┓', '┗', '┛']),
            _ if all(Weight::Double) => corner(['╔', '╗', '╚', '╝']),
            _ if bits == L | R | D && wl == wr && wd == light => tee('┯', '╤', wl),
            _ if bits == L | R | U && wl == wr && wu == light => tee('┷', '╧', wl),
            _ if bits == U | D | R && wu == wd && wr == light => tee('┠', '╟', wu),
            _ if bits == U | D | L && wu == wd && wl == light => tee('┨', '╢', wu),
            _ => None,
        };
        found.unwrap_or(match bits {
            b if b == D | R => '┌',
            b if b == D | L => '┐',
            b if b == U | R => '└',
            b if b == U | L => '┘',
            b if b == U | D | R => '├',
            b if b == U | D | L => '┤',
            b if b == D | L | R => '┬',
            b if b == U | L | R => '┴',
            _ => '┼',
        })
    }

    fn resolve(&self, cell: Pt, arms: &[Arm]) -> char {
        let pure = arms.iter().all(|a| a.bits & VERT == 0 || a.bits & HORZ == 0);
        let full_v = arms.iter().find(|a| a.bits == VERT);
        if let Some(v) = full_v.filter(|_| pure && arms.iter().any(|a| a.bits == HORZ)) {
            return self.straight(v.weight, v.style, true); // a crossing, not a join
        }
        let bits = arms.iter().fold(0, |b, a| b | a.bits);
        let style = if arms.iter().all(|a| a.style == arms[0].style) { arms[0].style } else { Style::Solid };
        let weight = |dir: u8| arms.iter().filter(|a| a.bits & dir != 0).map(|a| a.weight).max();
        let heaviest = arms.iter().map(|a| a.weight).max().unwrap_or(Weight::Light);
        match bits {
            b if b & HORZ == 0 => self.straight(heaviest, style, true),
            b if b & VERT == 0 => self.straight(heaviest, style, false),
            b => self.junction(b, [weight(U), weight(D), weight(L), weight(R)], self.round.contains(&cell)),
        }
    }

    fn ascii_symbol(ch: char) -> char {
        match ch {
            '▼' | '▾' => 'v',
            '▲' => '^',
            '►' => '>',
            '◄' => '<',
            '░' => '.',
            '•' => '*',
            '━' => '=',
            '─' => '-',
            c => c,
        }
    }

    /// Everything drawn so far, as trimmed lines (top-left at the smallest
    /// coordinates used).
    pub fn finish(&self) -> String {
        let mut out: HashMap<Pt, char> = HashMap::new();
        for layer in self.lines.iter().rev() {
            for (&cell, arms) in layer {
                out.insert(cell, self.resolve(cell, arms));
            }
        }
        for (&cell, &(ch, symbol)) in &self.glyphs {
            out.insert(cell, if symbol && self.ascii { Self::ascii_symbol(ch) } else { ch });
        }
        let Some(x0) = out.keys().map(|c| c.0).min() else { return String::new() };
        let y0 = out.keys().map(|c| c.1).min().unwrap_or(0);
        let x1 = out.keys().map(|c| c.0).max().unwrap_or(0);
        let y1 = out.keys().map(|c| c.1).max().unwrap_or(0);
        let mut rows = vec![vec![' '; (x1 - x0 + 1) as usize]; (y1 - y0 + 1) as usize];
        for (&(x, y), &ch) in &out {
            rows[(y - y0) as usize][(x - x0) as usize] = ch;
        }
        let lines: Vec<String> =
            rows.iter().map(|r| r.iter().filter(|&&c| c != WIDE).collect::<String>().trim_end().to_string()).collect();
        lines.join("\n")
    }
}
