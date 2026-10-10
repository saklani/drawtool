//! The one place that turns geometry into characters. Renderers draw text,
//! symbols and lines (box borders are lines too); the painter derives every
//! corner, tee and junction from which directions meet in a cell.

use crate::model::Style;
use crate::text::char_width;
use std::collections::{HashMap, HashSet};

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

/// How a line is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stroke {
    pub weight: Weight,
    pub style: Style,
}

impl Stroke {
    pub const LIGHT: Stroke = Stroke::new(Weight::Light, Style::Solid);
    pub const HEAVY: Stroke = Stroke::new(Weight::Heavy, Style::Solid);
    pub const DOTTED: Stroke = Stroke::new(Weight::Light, Style::Dotted);

    #[must_use]
    pub const fn new(weight: Weight, style: Style) -> Self {
        Stroke { weight, style }
    }

    /// A light line in `style` (an edge).
    #[must_use]
    pub const fn light(style: Style) -> Self {
        Stroke::new(Weight::Light, style)
    }
}

/// Main lines, or the under layer, which only shows in otherwise empty cells
/// (group and block frames).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Main,
    Under,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corners {
    Square,
    Round,
}

#[derive(Default)]
pub struct Painter {
    pub ascii: bool,
    /// Draw crossings as `╂` (flowcharts), so they can't be mistaken for a
    /// `┼` join; otherwise the vertical line just runs through (lifelines).
    pub mark_crossings: bool,
    /// Text and symbols; the flag marks symbols, which ASCII mode translates.
    glyphs: HashMap<Pt, (char, bool)>,
    /// Main lines, then the under layer (shown only in otherwise empty cells).
    lines: [HashMap<Pt, Vec<Arm>>; 2],
    round: HashSet<Pt>,
    nets: usize,
    /// Corners of the area pasted in, which stays the canvas even where
    /// its edges have since been erased.
    extent: Option<(Pt, Pt)>,
}

impl Painter {
    #[must_use]
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
    pub fn line(&mut self, pts: &[Pt], net: usize, stroke: Stroke, layer: Layer) {
        let Stroke { weight, style } = stroke;
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            let (lo, hi) = if x0 == x1 { (y0.min(y1), y0.max(y1)) } else { (x0.min(x1), x0.max(x1)) };
            for t in lo..=hi {
                let (back, fwd) = if x0 == x1 { (U, D) } else { (L, R) };
                let bits = if t > lo { back } else { 0 } | if t < hi { fwd } else { 0 };
                let cell = if x0 == x1 { (x0, t) } else { (t, y0) };
                let arms = self.lines[usize::from(layer == Layer::Under)].entry(cell).or_default();
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
    pub fn rect(&mut self, (x, y): Pt, (w, h): Pt, stroke: Stroke, corners: Corners, layer: Layer) {
        let (x1, y1) = (x + w - 1, y + h - 1);
        let net = self.net();
        self.line(&[(x, y), (x1, y), (x1, y1), (x, y1), (x, y)], net, stroke, layer);
        if corners == Corners::Round {
            self.round.extend([(x, y), (x1, y), (x, y1), (x1, y1)]);
        }
    }

    /// Erase every line in a rectangle (for boxes drawn over other lines).
    pub fn clear(&mut self, (x0, y0): Pt, (x1, y1): Pt) {
        for layer in &mut self.lines {
            layer.retain(|&(x, y), _| x < x0 || x > x1 || y < y0 || y > y1);
        }
    }

    /// Erase lines, text and symbols in a rectangle.
    pub fn erase(&mut self, (x0, y0): Pt, (x1, y1): Pt) {
        self.remap(|(x, y)| (x < x0 || x > x1 || y < y0 || y > y1).then_some((x, y)));
    }

    /// Remove columns `lo..=hi`, moving everything right of them left to close the gap.
    pub fn remove_columns(&mut self, lo: i64, hi: i64) {
        self.remap(|(x, y)| match x {
            x if x < lo => Some((x, y)),
            x if x > hi => Some((x - (hi - lo + 1), y)),
            _ => None,
        });
    }

    /// Move every cell to `to(cell)`, dropping the ones it maps to `None`.
    fn remap(&mut self, to: impl Fn(Pt) -> Option<Pt>) {
        self.glyphs = self.glyphs.drain().filter_map(|(pt, g)| Some((to(pt)?, g))).collect();
        for layer in &mut self.lines {
            *layer = layer.drain().filter_map(|(pt, arms)| Some((to(pt)?, arms))).collect();
        }
        self.round = self.round.drain().filter_map(&to).collect();
    }

    /// Draw everything `other` holds here, moved by (`dx`, `dy`). Its nets
    /// stay its own: they never join ours.
    pub fn paste(&mut self, other: &Painter, (dx, dy): Pt) {
        let at = |(x, y): Pt| (x + dx, y + dy);
        let first = self.nets;
        self.glyphs.extend(other.glyphs.iter().map(|(&pt, &g)| (at(pt), g)));
        for (mine, theirs) in self.lines.iter_mut().zip(&other.lines) {
            for (&pt, arms) in theirs {
                mine.entry(at(pt)).or_default().extend(arms.iter().map(|&a| Arm { net: first + a.net, ..a }));
            }
        }
        self.round.extend(other.round.iter().map(|&pt| at(pt)));
        self.nets += other.nets;
        if let Some((lo, hi)) = other.bounds() {
            let (lo, hi) = (at(lo), at(hi));
            let (a, b) = self.extent.unwrap_or((lo, hi));
            self.extent = Some(((a.0.min(lo.0), a.1.min(lo.1)), (b.0.max(hi.0), b.1.max(hi.1))));
        }
    }

    /// Top-left and bottom-right of everything drawn (and pasted).
    #[must_use]
    pub fn bounds(&self) -> Option<(Pt, Pt)> {
        let cells = self.glyphs.keys().chain(self.lines.iter().flat_map(HashMap::keys));
        let corners = self.extent.iter().flat_map(|(lo, hi)| [lo, hi]);
        let (xs, ys): (Vec<i64>, Vec<i64>) = cells.chain(corners).copied().unzip();
        Some(((*xs.iter().min()?, *ys.iter().min()?), (*xs.iter().max()?, *ys.iter().max()?)))
    }

    /// The character a cell shows: text or symbol, else main line, else under.
    #[must_use]
    pub fn cell(&self, pt: Pt) -> char {
        if let Some(&(ch, symbol)) = self.glyphs.get(&pt) {
            return if symbol && self.ascii { Self::ascii_symbol(ch) } else { ch };
        }
        self.lines.iter().find_map(|layer| layer.get(&pt)).map_or(' ', |arms| self.resolve(pt, arms))
    }

    /// No text, symbol or main line in this cell.
    #[must_use]
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

    /// The glyph where lines meet. Arms are the weights reaching up, down,
    /// left and right; a light edge meeting a heavy or double border gets a
    /// mixed tee (`┯`, `╤`, ...).
    fn junction(&self, bits: u8, arms: [Option<Weight>; 4], round: bool) -> char {
        use Weight::{Double, Heavy, Light};
        let present: Vec<Weight> = arms.iter().flatten().copied().collect();
        let all = |k: Weight| present.iter().all(|&p| p == k);
        if self.ascii {
            return if all(Heavy) {
                '#'
            } else if round && bits & D != 0 {
                '.'
            } else if round {
                '\''
            } else {
                '+'
            };
        }
        // Index into the rows below: four corners, four tees, then the cross.
        let shapes = [D | R, D | L, U | R, U | L, U | D | R, U | D | L, D | L | R, U | L | R];
        let i = shapes.iter().position(|&b| b == bits).unwrap_or(8);
        let [up, down, left, right] = arms;
        let border = if (bits == U | D | R && right == Some(Light)) || (bits == U | D | L && left == Some(Light)) {
            up.filter(|_| up == down)
        } else if (bits == D | L | R && down == Some(Light)) || (bits == U | L | R && up == Some(Light)) {
            left.filter(|_| left == right)
        } else {
            None
        };
        let h = left.or(right);
        let corner_mix = i < 4 && h == Some(Double) && up.or(down) == Some(Light);
        let row = if corner_mix {
            "╒╕╘╛"
        } else if round && all(Light) {
            "╭╮╰╯"
        } else if all(Heavy) {
            "┏┓┗┛"
        } else if all(Double) {
            "╔╗╚╝"
        } else if border == Some(Heavy) {
            "┌┐└┘┠┨┯┷"
        } else if border == Some(Double) {
            "┌┐└┘╟╢╤╧"
        } else {
            ""
        };
        row.chars().nth(i).or_else(|| "┌┐└┘├┤┬┴".chars().nth(i)).unwrap_or('┼')
    }

    fn resolve(&self, cell: Pt, arms: &[Arm]) -> char {
        let pure = arms.iter().all(|a| a.bits & VERT == 0 || a.bits & HORZ == 0);
        let full_v = arms.iter().find(|a| a.bits == VERT);
        if let Some(v) = full_v.filter(|_| pure && arms.iter().any(|a| a.bits == HORZ)) {
            // A crossing, not a join.
            return if self.mark_crossings && !self.ascii { '╂' } else { self.straight(v.weight, v.style, true) };
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
    #[must_use]
    pub fn finish(&self) -> String {
        let Some(((x0, y0), (x1, y1))) = self.bounds() else { return String::new() };
        let row = |y| (x0..=x1).map(|x| self.cell((x, y))).filter(|&c| c != WIDE).collect::<String>();
        (y0..=y1).map(|y| row(y).trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(ascii: bool, draw: impl FnOnce(&mut Painter)) -> String {
        let mut p = Painter::new(ascii);
        draw(&mut p);
        p.finish()
    }

    #[test]
    fn an_edge_leaving_a_box_border_makes_a_tee() {
        let out = painted(false, |p| {
            p.rect((0, 0), (5, 3), Stroke::LIGHT, Corners::Square, Layer::Main);
            let net = p.net();
            p.line(&[(2, 2), (2, 4)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out, "┌───┐\n│   │\n└─┬─┘\n  │\n  │");
    }

    #[test]
    fn rounded_heavy_and_double_corners() {
        let corners = |stroke, corners| painted(false, |p| p.rect((0, 0), (3, 3), stroke, corners, Layer::Main));
        assert!(corners(Stroke::LIGHT, Corners::Round).starts_with('╭'));
        assert!(corners(Stroke::HEAVY, Corners::Square).starts_with('┏'));
        assert!(corners(Stroke::new(Weight::Double, Style::Solid), Corners::Square).starts_with('╔'));
    }

    #[test]
    fn a_light_edge_on_a_double_border_is_a_mixed_tee() {
        let out = painted(false, |p| {
            p.rect((0, 0), (5, 3), Stroke::new(Weight::Double, Style::Solid), Corners::Square, Layer::Main);
            let net = p.net();
            p.line(&[(2, 2), (2, 3)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out.lines().nth(2), Some("╚═╤═╝"));
    }

    #[test]
    fn crossing_vs_join() {
        let cross = |mark_crossings, same_net| {
            painted(false, |p| {
                p.mark_crossings = mark_crossings;
                let (a, b) = (p.net(), p.net());
                p.line(&[(0, 1), (2, 1)], a, Stroke::LIGHT, Layer::Main);
                p.line(&[(1, 0), (1, 2)], if same_net { a } else { b }, Stroke::LIGHT, Layer::Main);
            })
        };
        assert_eq!(cross(false, true).lines().nth(1), Some("─┼─"), "one net: a join");
        assert_eq!(cross(true, false).lines().nth(1), Some("─╂─"), "two nets: a marked crossing");
        assert_eq!(cross(false, false).lines().nth(1), Some("─│─"), "unmarked: the vertical runs through");
    }

    #[test]
    fn a_trunk_shared_by_two_styles_is_drawn_solid() {
        let out = painted(false, |p| {
            let net = p.net();
            p.line(&[(0, 0), (3, 0)], net, Stroke::DOTTED, Layer::Main);
            p.line(&[(0, 0), (3, 0)], net, Stroke::light(Style::Thick), Layer::Main);
        });
        assert_eq!(out, "────");
    }

    #[test]
    fn the_under_layer_only_shows_in_empty_cells() {
        let out = painted(false, |p| {
            p.rect((0, 0), (5, 3), Stroke::DOTTED, Corners::Square, Layer::Under);
            let net = p.net();
            p.line(&[(0, 1), (4, 1)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out, "┌┄┄┄┐\n─────\n└┄┄┄┘");
    }

    #[test]
    fn clear_erases_lines_but_text_stays() {
        let out = painted(false, |p| {
            let net = p.net();
            p.line(&[(0, 0), (4, 0)], net, Stroke::LIGHT, Layer::Main);
            p.clear((1, 0), (3, 0));
            p.text(2, 0, "x");
        });
        assert_eq!(out, "─ x ─");
    }

    #[test]
    fn pasted_lines_keep_their_nets_and_size() {
        let out = painted(false, |p| {
            let mut other = Painter::new(false);
            let net = other.net();
            other.line(&[(1, 0), (1, 2)], net, Stroke::LIGHT, Layer::Main);
            other.text(0, 3, "ab");
            let net = p.net();
            p.line(&[(0, 1), (2, 1)], net, Stroke::LIGHT, Layer::Main);
            p.paste(&other, (0, 0));
            p.erase((0, 3), (1, 3));
        });
        assert_eq!(out, " │\n─│─\n │\n", "a crossing, not a join; the erased row stays");
    }

    #[test]
    fn removed_columns_close_the_gap() {
        let out = painted(false, |p| {
            p.text(0, 0, "a..b");
            p.remove_columns(1, 2);
        });
        assert_eq!(out, "ab");
    }

    #[test]
    fn ascii_translates_symbols_but_never_text() {
        let out = painted(true, |p| {
            p.rect((0, 0), (5, 3), Stroke::LIGHT, Corners::Round, Layer::Main);
            p.symbol(0, 3, '►');
            p.text(1, 3, "►");
        });
        assert_eq!(out, ".---.\n|   |\n'---'\n>►");
    }

    #[test]
    fn wide_text_takes_two_cells_and_negative_coordinates_shift_to_zero() {
        let out = painted(false, |p| {
            p.text(-3, -1, "数据");
            p.symbol(1, -1, '!');
        });
        assert_eq!(out, "数据!");
    }
}
