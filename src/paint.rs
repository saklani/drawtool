//! The one place that turns geometry into characters. Renderers draw text,
//! symbols and lines (box borders are lines too); the painter derives every
//! corner, tee and junction from which directions meet in a cell.

use crate::text::char_width;
use crate::Style;
use std::collections::{HashMap, HashSet};

const U: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const R: u8 = 8;
/// Right half of a double-width character.
const WIDE: char = '\0';

// Glyphs by the directions present in a cell (U = 1, D = 2, L = 4, R = 8);
// a lone direction is a line's end, drawn as the full line.
const LIGHT: &str = " │││─┘┐┤─└┌├─┴┬┼";
const ROUND: &str = " │││─╯╮┤─╰╭├─┴┬┼";
const HEAVY: &str = " ┃┃┃━┛┓┫━┗┏┣━┻┳╋";
const DOUBLE: &str = " ║║║═╝╗╣═╚╔╠═╩╦╬";
// Mixed weights: the vertical arms' weight, then the horizontal arms'.
const LIGHT_HEAVY: &str = " │││━┙┑┥━┕┍┝━┷┯┿";
const HEAVY_LIGHT: &str = " ┃┃┃─┚┒┨─┖┎┠─┸┰╂";
const LIGHT_DOUBLE: &str = " │││═╛╕╡═╘╒╞═╧╤╪";
const DOUBLE_LIGHT: &str = " ║║║─╜╖╢─╙╓╟─╨╥╫";

pub type Pt = (i64, i64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Weight {
    Light,
    Heavy,
    Double,
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

/// One net's line directions in a cell. Lines of the same net join; lines of
/// different nets that pass straight through each other cross.
#[derive(Clone, Copy)]
struct Arm {
    net: usize,
    bits: u8,
    stroke: Stroke,
}

#[derive(Default)]
pub struct Painter {
    /// Draw crossings as `╂` (flowcharts), so they can't be mistaken for a
    /// `┼` join; otherwise the vertical line just runs through (lifelines).
    pub mark_crossings: bool,
    /// Text and symbols.
    glyphs: HashMap<Pt, char>,
    /// Main lines, then the under layer.
    lines: [HashMap<Pt, Vec<Arm>>; 2],
    round: HashSet<Pt>,
    nets: usize,
}

impl Painter {
    #[must_use]
    pub fn new() -> Self {
        Painter::default()
    }

    /// A fresh net id.
    pub fn net(&mut self) -> usize {
        self.nets += 1;
        self.nets
    }

    pub fn text(&mut self, mut x: i64, y: i64, s: &str) {
        for ch in s.chars() {
            self.glyphs.insert((x, y), ch);
            if char_width(ch) == 2 {
                self.glyphs.insert((x + 1, y), WIDE);
            }
            x += char_width(ch);
        }
    }

    pub fn symbol(&mut self, x: i64, y: i64, ch: char) {
        self.glyphs.insert((x, y), ch);
    }

    /// An axis-aligned polyline through `pts`.
    pub fn line(&mut self, pts: &[Pt], net: usize, stroke: Stroke, layer: Layer) {
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            let vertical = x0 == x1;
            let (lo, hi) = if vertical { (y0.min(y1), y0.max(y1)) } else { (x0.min(x1), x0.max(x1)) };
            let (back, fwd) = if vertical { (U, D) } else { (L, R) };
            for t in lo..=hi {
                let bits = if t > lo { back } else { 0 } | if t < hi { fwd } else { 0 };
                let cell = if vertical { (x0, t) } else { (t, y0) };
                let arms = self.lines[usize::from(layer == Layer::Under)].entry(cell).or_default();
                match arms.iter_mut().find(|a| a.net == net) {
                    Some(a) => {
                        a.bits |= bits;
                        // A trunk shared by edges of different styles belongs to none of them.
                        if a.stroke.style != stroke.style {
                            a.stroke.style = Style::Solid;
                        }
                    }
                    None => arms.push(Arm { net, bits, stroke }),
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

    /// Top-left and bottom-right of everything drawn.
    pub(crate) fn bounds(&self) -> Option<(Pt, Pt)> {
        let cells = self.glyphs.keys().chain(self.lines.iter().flat_map(HashMap::keys));
        let (xs, ys): (Vec<i64>, Vec<i64>) = cells.copied().unzip();
        Some(((*xs.iter().min()?, *ys.iter().min()?), (*xs.iter().max()?, *ys.iter().max()?)))
    }

    /// The character a cell shows: text or symbol, else main line, else under.
    #[must_use]
    pub fn cell(&self, pt: Pt) -> char {
        if let Some(&ch) = self.glyphs.get(&pt) {
            return ch;
        }
        self.lines.iter().find_map(|layer| layer.get(&pt)).map_or(' ', |arms| self.resolve(pt, arms))
    }

    /// No text, symbol or main line in this cell.
    #[must_use]
    pub fn is_free(&self, cell: Pt) -> bool {
        !self.glyphs.contains_key(&cell) && !self.lines[0].contains_key(&cell)
    }

    fn straight(&self, Stroke { weight, style }: Stroke, vertical: bool) -> char {
        let pair = match (weight, style) {
            (Weight::Heavy, _) | (_, Style::Thick) => ['┃', '━'],
            (Weight::Double, _) => ['║', '═'],
            (_, Style::Dotted) => ['┆', '┄'],
            (_, Style::Solid) => ['│', '─'],
        };
        pair[usize::from(!vertical)]
    }

    /// The glyph where lines of one net meet, by the weights of its vertical
    /// and horizontal arms.
    fn junction(&self, bits: u8, vertical: Weight, horizontal: Weight, round: bool) -> char {
        use Weight::{Double, Heavy, Light};
        let table = match (vertical, horizontal) {
            (Light, Light) if round => ROUND,
            (Light, Light) => LIGHT,
            (Heavy, Heavy) => HEAVY,
            (Double, Double) => DOUBLE,
            (Light, Heavy) => LIGHT_HEAVY,
            (Heavy, Light) => HEAVY_LIGHT,
            (Light, Double) => LIGHT_DOUBLE,
            (Double, Light) => DOUBLE_LIGHT,
            (Heavy, Double) | (Double, Heavy) => DOUBLE,
        };
        table.chars().nth(usize::from(bits)).unwrap_or('┼')
    }

    fn resolve(&self, cell: Pt, arms: &[Arm]) -> char {
        let (vert, horz) = (U | D, L | R);
        let pure = arms.iter().all(|a| a.bits & vert == 0 || a.bits & horz == 0);
        let through = arms.iter().find(|a| a.bits == vert);
        if let Some(v) = through.filter(|_| pure && arms.iter().any(|a| a.bits == horz)) {
            // A crossing, not a join.
            return if self.mark_crossings { '╂' } else { self.straight(v.stroke, true) };
        }
        let bits = arms.iter().fold(0, |b, a| b | a.bits);
        let style = if arms.iter().all(|a| a.stroke.style == arms[0].stroke.style) {
            arms[0].stroke.style
        } else {
            Style::Solid
        };
        let weight = |dirs: u8| arms.iter().filter(|a| a.bits & dirs != 0).map(|a| a.stroke.weight).max();
        let heaviest = weight(vert | horz).unwrap_or(Weight::Light);
        match (weight(vert), weight(horz)) {
            (_, None) => self.straight(Stroke::new(heaviest, style), true),
            (None, _) => self.straight(Stroke::new(heaviest, style), false),
            (Some(v), Some(h)) => self.junction(bits, v, h, self.round.contains(&cell)),
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

    fn painted(draw: impl FnOnce(&mut Painter)) -> String {
        let mut p = Painter::new();
        draw(&mut p);
        p.finish()
    }

    #[test]
    fn an_edge_leaving_a_box_border_makes_a_tee() {
        let out = painted(|p| {
            p.rect((0, 0), (5, 3), Stroke::LIGHT, Corners::Square, Layer::Main);
            let net = p.net();
            p.line(&[(2, 2), (2, 4)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out, "┌───┐\n│   │\n└─┬─┘\n  │\n  │");
    }

    #[test]
    fn rounded_heavy_and_double_corners() {
        let corners = |stroke, corners| painted(|p| p.rect((0, 0), (3, 3), stroke, corners, Layer::Main));
        assert!(corners(Stroke::LIGHT, Corners::Round).starts_with('╭'));
        assert!(corners(Stroke::HEAVY, Corners::Square).starts_with('┏'));
        assert!(corners(Stroke::new(Weight::Double, Style::Solid), Corners::Square).starts_with('╔'));
    }

    #[test]
    fn a_light_edge_on_a_double_border_is_a_mixed_tee() {
        let out = painted(|p| {
            p.rect((0, 0), (5, 3), Stroke::new(Weight::Double, Style::Solid), Corners::Square, Layer::Main);
            let net = p.net();
            p.line(&[(2, 2), (2, 3)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out.lines().nth(2), Some("╚═╤═╝"));
    }

    #[test]
    fn crossing_vs_join() {
        let cross = |mark_crossings, same_net| {
            painted(|p| {
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
        let out = painted(|p| {
            let net = p.net();
            p.line(&[(0, 0), (3, 0)], net, Stroke::DOTTED, Layer::Main);
            p.line(&[(0, 0), (3, 0)], net, Stroke::light(Style::Thick), Layer::Main);
        });
        assert_eq!(out, "────");
    }

    #[test]
    fn the_under_layer_only_shows_in_empty_cells() {
        let out = painted(|p| {
            p.rect((0, 0), (5, 3), Stroke::DOTTED, Corners::Square, Layer::Under);
            let net = p.net();
            p.line(&[(0, 1), (4, 1)], net, Stroke::LIGHT, Layer::Main);
        });
        assert_eq!(out, "┌┄┄┄┐\n─────\n└┄┄┄┘");
    }

    #[test]
    fn wide_text_takes_two_cells_and_negative_coordinates_shift_to_zero() {
        let out = painted(|p| {
            p.text(-3, -1, "数据");
            p.symbol(1, -1, '!');
        });
        assert_eq!(out, "数据!");
    }
}
