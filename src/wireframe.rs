//! Wireframes: an indented outline of UI elements drawn as a screen mockup.
//!
//! ```text
//! wireframe Add money
//!   header: ← Back | Add money | ?
//!   input: Amount ($) = 0.00
//!   row: [ $20 ] | [ $50 ] | [ $100 ]
//!   button: Continue
//! ```
//!
//! Each element draws itself at a position and width and returns its height;
//! `card`, `row` and `col` place their children.

use crate::paint::{char_width, width, wrap, Fit, Painter, Weight};
use crate::parse::{strip_comment, Style};
use crate::Error;

/// Screen width when the header doesn't give one (`width=N`).
pub const DEFAULT_WIDTH: usize = 40;
/// Screens are never squeezed narrower than this.
pub const MIN_WIDTH: usize = 20;
const SCREEN_GAP: i64 = 3;
const ROW_GAP: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
    /// Left, unless a `row` places it: first left, last right, middle centred.
    Auto,
}

#[derive(Clone, PartialEq, Debug)]
pub enum El {
    Text(String, Align),
    Divider,
    /// Up to three slots: left, centre, right; followed by a divider.
    Header(Vec<String>),
    Input {
        label: String,
        value: String,
        select: bool,
    },
    /// A full-width primary button. Inline buttons are just text: `[ OK ]`.
    Button(String),
    Image(String),
    Card {
        title: String,
        children: Vec<El>,
    },
    Row(Vec<El>),
    Col(Vec<El>),
    /// `tabs` underline the active item on a full baseline; `nav` sits under
    /// a divider (a bottom navigation bar).
    Tabs {
        items: Vec<String>,
        active: Option<usize>,
        nav: bool,
    },
    Toggle(String, bool),
    Checkbox(String, bool),
    Radio(Vec<String>, Option<usize>),
    /// `- text | right`: the optional right part is right-aligned.
    Bullet(String, Option<String>),
    Spacer,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Screen {
    pub title: String,
    pub width: usize,
    pub children: Vec<El>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Wireframe {
    pub screens: Vec<Screen>,
}

/// True if the first meaningful line starts a wireframe.
pub fn is_wireframe(text: &str) -> bool {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%")).is_some_and(is_screen_header)
}

fn is_screen_header(line: &str) -> bool {
    line.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("wireframe"))
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Line<'a> {
    no: usize,
    indent: usize,
    text: &'a str,
}

/// Parse one or more `wireframe` screens.
///
/// # Errors
///
/// Input that doesn't start with `wireframe`, a bad `width=`, or indentation
/// under an element that can't have children.
pub fn parse_wireframe(text: &str) -> Result<Wireframe, Error> {
    let lines: Vec<Line> = text
        .lines()
        .enumerate()
        .filter(|(_, raw)| !raw.trim().is_empty() && !raw.trim().starts_with("%%"))
        .map(|(i, raw)| {
            let indent = raw.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
            Line { no: i + 1, indent, text: strip_comment(raw.trim()) }
        })
        .collect();
    let mut screens = Vec::new();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        if !is_screen_header(line.text) {
            return Err(Error::at(line.no, "expected `wireframe [title]`"));
        }
        let (mut title, mut width) = (Vec::new(), DEFAULT_WIDTH);
        for word in line.text.split_whitespace().skip(1) {
            match word.strip_prefix("width=") {
                Some(n) => {
                    width = n.parse::<usize>().map_err(|_| Error::at(line.no, format!("invalid width {n:?}")))?
                }
                None => title.push(word),
            }
        }
        i += 1;
        let indent = lines.get(i).filter(|l| !is_screen_header(l.text)).map_or(0, |l| l.indent);
        let children = parse_block(&lines, &mut i, indent)?;
        screens.push(Screen { title: unquote(&title.join(" ")), width: width.max(MIN_WIDTH), children });
    }
    if screens.is_empty() {
        return Err(Error::new("expected `wireframe [title]`"));
    }
    Ok(Wireframe { screens })
}

/// Elements at exactly `indent`, each with its deeper-indented children.
fn parse_block(lines: &[Line], i: &mut usize, indent: usize) -> Result<Vec<El>, Error> {
    let mut out = Vec::new();
    while let Some(line) = lines.get(*i).filter(|l| !is_screen_header(l.text) && l.indent >= indent) {
        if line.indent > indent {
            return Err(Error::at(line.no, "unexpected indentation (only card, row and col take children)"));
        }
        *i += 1;
        let deeper = lines.get(*i).filter(|l| l.indent > indent && !is_screen_header(l.text)).map(|l| l.indent);
        let children = |i: &mut usize| deeper.map_or(Ok(Vec::new()), |d| parse_block(lines, i, d));
        let head = line.text.strip_suffix(':').map(str::trim);
        let (word, rest) = head.map_or(("", ""), |h| h.split_once(char::is_whitespace).unwrap_or((h, "")));
        out.push(match word.to_ascii_lowercase().as_str() {
            "card" | "box" => El::Card { title: unquote(rest), children: children(i)? },
            "row" if rest.is_empty() => El::Row(children(i)?),
            "col" | "column" if rest.is_empty() => El::Col(children(i)?),
            _ => leaf(line.text),
        });
    }
    Ok(out)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let quoted = s.len() >= 2 && (s.starts_with('"') && s.ends_with('"') || s.starts_with('\'') && s.ends_with('\''));
    if quoted { &s[1..s.len() - 1] } else { s }.to_string()
}

/// `a | *b | c` → (["a", "b", "c"], Some(1)).
fn items(rest: &str) -> (Vec<String>, Option<usize>) {
    let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
    let active = parts.iter().position(|p| p.starts_with('*'));
    (parts.iter().map(|p| unquote(p.trim_start_matches('*'))).collect(), active)
}

/// `label = value`; a bare value is a label.
fn label_value(rest: &str) -> (String, String) {
    rest.split_once('=').map_or((unquote(rest), String::new()), |(l, v)| (unquote(l), unquote(v)))
}

fn is_on(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "on" | "true" | "yes" | "x" | "checked")
}

/// A single-line element. Anything unrecognised is text.
fn leaf(text: &str) -> El {
    match text {
        "---" | "divider" => return El::Divider,
        "spacer" => return El::Spacer,
        _ => {}
    }
    if let Some(item) = text.strip_prefix("- ") {
        return match item.split_once('|') {
            Some((l, r)) => El::Bullet(unquote(l), Some(unquote(r))),
            None => El::Bullet(unquote(item), None),
        };
    }
    let Some((kw, rest)) = text.split_once(':') else { return El::Text(unquote(text), Align::Auto) };
    let (kw, rest) = (kw.trim().to_ascii_lowercase(), rest.trim());
    let text_as = |a| El::Text(unquote(rest), a);
    match kw.as_str() {
        "text" => text_as(Align::Auto),
        "left" => text_as(Align::Left),
        "center" | "centre" => text_as(Align::Center),
        "right" => text_as(Align::Right),
        "header" => El::Header(items(rest).0),
        "input" | "select" => {
            let (label, value) = label_value(rest);
            El::Input { label, value, select: kw == "select" }
        }
        "button" => El::Button(unquote(rest)),
        "image" => El::Image(unquote(rest)),
        "tabs" | "nav" => {
            let (items, active) = items(rest);
            El::Tabs { items, active, nav: kw == "nav" }
        }
        "toggle" | "checkbox" => {
            let (label, value) = label_value(rest);
            if kw == "toggle" {
                El::Toggle(label, is_on(&value))
            } else {
                El::Checkbox(label, is_on(&value))
            }
        }
        "radio" => {
            let (items, active) = items(rest);
            El::Radio(items, active)
        }
        "row" => El::Row(rest.split('|').map(|p| leaf(p.trim())).collect()),
        "col" | "column" => El::Col(rest.split('|').map(|p| leaf(p.trim())).collect()),
        _ => El::Text(unquote(text), Align::Auto),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Cut to at most `w` columns, ending in `…` if anything was dropped.
fn truncate(s: &str, w: i64) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if width(&out) + char_width(ch) > w - 1 {
            break;
        }
        out.push(ch);
    }
    if w > 0 {
        out.push('…');
    }
    out
}

/// Draw `s` (cut to `w`) aligned within `x..x+w`.
fn put(p: &mut Painter, x: i64, y: i64, w: i64, s: &str, align: Align) {
    let s = truncate(s, w);
    let room = w - width(&s);
    let dx = match align {
        Align::Left | Align::Auto => 0,
        Align::Center => room / 2,
        Align::Right => room,
    };
    p.text(x + dx, y, &s);
}

/// Wrapped text; returns its height.
fn para(p: &mut Painter, (x, y): (i64, i64), w: i64, s: &str, align: Align) -> i64 {
    let lines = wrap(s, w as usize, true);
    for (k, l) in lines.iter().enumerate() {
        put(p, x, y + k as i64, w, l, align);
    }
    lines.len() as i64
}

/// `marker` then `text`, wrapped with continuation lines indented under it.
fn hanging(p: &mut Painter, (x, y): (i64, i64), w: i64, marker: &str, text: &str) -> i64 {
    p.text(x, y, marker);
    para(p, (x + width(marker), y), w - width(marker), text, Align::Left)
}

/// A horizontal line; `edge` extends it into a surrounding frame's border.
fn rule(p: &mut Painter, x: i64, y: i64, w: i64, edge: i64) {
    let net = p.net();
    p.line(&[(x - edge, y), (x + w - 1 + edge, y)], net, Weight::Light, Style::Solid, false);
}

fn draw_all(p: &mut Painter, children: &[El], (x, y): (i64, i64), w: i64, edge: i64) -> i64 {
    children.iter().fold(0, |h, c| h + draw(p, c, (x, y + h), w, Align::Auto, edge))
}

/// A bordered block: children inside, `title` on the top edge.
fn frame(p: &mut Painter, title: &str, children: &[El], (x, y): (i64, i64), w: i64) -> i64 {
    let h = draw_all(p, children, (x + 2, y + 1), w - 4, 2) + 2;
    p.rect((x, y), (w, h), Weight::Light, Style::Solid, false, false);
    if !title.is_empty() {
        p.text(x + 2, y, &format!(" {} ", truncate(title, w - 6)));
    }
    h
}

/// Draw one element `w` columns wide at (`x`, `y`) and return its height.
/// `hint` is the alignment a row gives to `Auto` text; `edge` is how far a
/// divider must reach to meet the surrounding frame.
fn draw(p: &mut Painter, el: &El, (x, y): (i64, i64), w: i64, hint: Align, edge: i64) -> i64 {
    let pick = |p: &Painter, u: &'static str, a: &'static str| if p.ascii { a } else { u };
    match el {
        El::Text(s, a) => para(p, (x, y), w, s, if *a == Align::Auto { hint } else { *a }),
        El::Spacer => 1,
        El::Divider => {
            rule(p, x, y, w, edge);
            1
        }
        El::Header(slots) => {
            let n = slots.len().min(3);
            for (i, slot) in slots.iter().take(3).enumerate() {
                let align = match (i, n) {
                    (0, _) => Align::Left,
                    (1, 2) | (2, _) => Align::Right,
                    _ => Align::Center,
                };
                put(p, x, y, w, slot, align);
            }
            rule(p, x, y + 1, w, edge);
            2
        }
        El::Input { label, value, select } => {
            let top = if label.is_empty() { 0 } else { para(p, (x, y), w, label, Align::Left) };
            p.rect((x, y + top), (w, 3), Weight::Light, Style::Solid, false, false);
            put(p, x + 2, y + top + 1, w - 4 - if *select { 2 } else { 0 }, value, Align::Left);
            if *select {
                p.symbol(x + w - 3, y + top + 1, '▾');
            }
            top + 3
        }
        El::Button(label) => {
            p.rect((x, y), (w, 3), Weight::Heavy, Style::Solid, false, false);
            put(p, x + 1, y + 1, w - 2, label, Align::Center);
            3
        }
        El::Image(caption) => {
            p.rect((x, y), (w, 5), Weight::Light, Style::Solid, false, false);
            for (r, c) in (1..4).flat_map(|r| (1..w - 1).map(move |c| (r, c))) {
                p.symbol(x + c, y + r, '░');
            }
            if !caption.is_empty() {
                put(p, x + 1, y + 2, w - 2, &format!(" {caption} "), Align::Center);
            }
            5
        }
        El::Card { title, children } => frame(p, title, children, (x, y), w),
        El::Col(children) => draw_all(p, children, (x, y), w, 0),
        El::Row(children) => {
            let n = children.len() as i64;
            let room = (w - ROW_GAP * (n - 1)).max(0);
            let (base, extra) = (room / n.max(1), room % n.max(1));
            let mut cx = x;
            let mut h = 0;
            for (i, c) in children.iter().enumerate() {
                let i = i as i64;
                let cw = base + i64::from(i < extra);
                let hint = match i {
                    _ if n == 1 || i == 0 => Align::Left,
                    _ if i == n - 1 => Align::Right,
                    _ => Align::Center,
                };
                h = h.max(draw(p, c, (cx, y), cw, hint, 0));
                cx += cw + ROW_GAP;
            }
            h
        }
        El::Tabs { items, active, nav } => {
            let n = items.len().max(1) as i64;
            let y = y + i64::from(*nav);
            if *nav {
                rule(p, x, y - 1, w, edge);
            } else {
                rule(p, x, y + 1, w, 0);
            }
            let mut cx = x;
            for (i, item) in items.iter().enumerate() {
                let cw = if i as i64 == n - 1 { x + w - cx } else { w / n };
                put(p, cx, y, cw, item, Align::Center);
                if Some(i) == *active {
                    let tw = width(&truncate(item, cw));
                    let net = p.net();
                    let start = cx + (cw - tw) / 2;
                    p.line(&[(start, y + 1), (start + tw - 1, y + 1)], net, Weight::Heavy, Style::Solid, false);
                }
                cx += cw;
            }
            1 + i64::from(*nav) + i64::from(!*nav || active.is_some())
        }
        El::Toggle(label, on) => {
            let switch = match (on, p.ascii) {
                (true, false) => "(  ●)",
                (false, false) => "(○  )",
                (true, true) => "[ ON]",
                (false, true) => "[OFF]",
            };
            put(p, x, y, w - 6, label, Align::Left);
            p.text(x + w - 5, y, switch);
            1
        }
        El::Checkbox(label, on) => hanging(p, (x, y), w, if *on { "[x] " } else { "[ ] " }, label),
        El::Radio(options, active) => {
            let dot = pick(p, "•", "*");
            let options: Vec<String> = options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("({}) {o}", if Some(i) == *active { dot } else { " " }))
                .collect();
            let one = options.join("   ");
            if width(&one) <= w {
                p.text(x, y, &one);
                return 1;
            }
            options.iter().fold(0, |h, o| h + para(p, (x, y + h), w, o, Align::Left))
        }
        El::Bullet(text, right) => {
            let right = right.as_deref().unwrap_or("");
            let room = if right.is_empty() { w } else { w - width(right) - 1 };
            let h = hanging(p, (x, y), room, pick(p, "• ", "* "), text);
            put(p, x, y, w, right, Align::Right);
            h
        }
    }
}

/// Render every screen at its own width.
#[must_use]
pub fn render_wireframe(wf: &Wireframe, ascii: bool) -> String {
    render_wireframe_fit(wf, ascii, 0).text
}

/// Render screens side by side, wrapping to a new row of screens when the
/// next one would pass `max_width` (0 = unlimited). A screen wider than the
/// budget is narrowed, down to `MIN_WIDTH`.
pub fn render_wireframe_fit(wf: &Wireframe, ascii: bool, max_width: usize) -> Fit {
    let mut p = Painter::new(ascii);
    let (mut x, mut y, mut row_h) = (0i64, 0i64, 0i64);
    for s in &wf.screens {
        let w = if max_width == 0 { s.width } else { s.width.min(max_width).max(MIN_WIDTH) } as i64;
        if x > 0 && max_width > 0 && x + w > max_width as i64 {
            (x, y, row_h) = (0, y + row_h + 1, 0);
        }
        row_h = row_h.max(frame(&mut p, &s.title, &s.children, (x, y), w));
        x += w + SCREEN_GAP;
    }
    let text = p.finish();
    let width = text.lines().map(width).max().unwrap_or(0) as usize;
    Fit { text, width, fits: max_width == 0 || width <= max_width }
}
