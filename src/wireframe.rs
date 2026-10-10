//! Wireframes: an indented outline of UI elements drawn as a screen mockup.
//!
//! ```text
//! wireframe Add money
//!   header: ← Back | Add money | ?
//!   input: Amount ($) = 0.00
//!   row: [ $20 ] | [ $50 ] | [ $100 ]
//!   radio: *Card | Bank
//!   button: Continue
//! ```
//!
//! Each element renders into a block of lines exactly as wide as the space it
//! is given; containers (`card`, `row`, `col`) compose their children's blocks.

use unicode_width::UnicodeWidthChar;

/// Screen width when the header doesn't give one (`width=N`).
pub const DEFAULT_WIDTH: usize = 40;
/// Screens are never squeezed narrower than this.
pub const MIN_WIDTH: usize = 20;
/// Columns between screens drawn side by side.
const SCREEN_GAP: usize = 3;
/// Columns between the children of a `row`.
const ROW_GAP: usize = 2;
/// Right half of a double-width character.
const WIDE: char = '\0';

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
    /// `- text | right`: the optional right part is right-aligned (amounts, dates).
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

pub struct WireFitted {
    pub text: String,
    pub width: usize,
    pub fits: bool,
}

/// True if the first meaningful line starts a wireframe.
pub fn is_wireframe(text: &str) -> bool {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%")).is_some_and(is_screen_header)
}

fn is_screen_header(line: &str) -> bool {
    let word = line.split_whitespace().next().unwrap_or("");
    word.eq_ignore_ascii_case("wireframe")
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
pub fn parse_wireframe(text: &str) -> Result<Wireframe, String> {
    let lines: Vec<Line> = text
        .lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let t = raw.trim();
            if t.is_empty() || t.starts_with("%%") {
                return None;
            }
            let indent = raw.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
            Some(Line { no: i + 1, indent, text: t })
        })
        .collect();
    let mut screens = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if !is_screen_header(line.text) {
            return Err(format!("line {}: expected `wireframe [title]`", line.no));
        }
        let (title, width) = screen_header(line)?;
        i += 1;
        let indent = lines.get(i).filter(|l| !is_screen_header(l.text)).map_or(0, |l| l.indent);
        let children = parse_block(&lines, &mut i, indent)?;
        screens.push(Screen { title, width, children });
    }
    if screens.is_empty() {
        return Err("expected `wireframe [title]`".to_string());
    }
    Ok(Wireframe { screens })
}

fn screen_header(line: &Line) -> Result<(String, usize), String> {
    let mut width = DEFAULT_WIDTH;
    let mut title = Vec::new();
    for word in line.text.split_whitespace().skip(1) {
        match word.strip_prefix("width=") {
            Some(n) => {
                width = n.parse().map_err(|_| format!("line {}: invalid width {n:?}", line.no))?;
                width = width.max(MIN_WIDTH);
            }
            None => title.push(word),
        }
    }
    Ok((unquote(&title.join(" ")), width))
}

/// Elements at exactly `indent`, each with its deeper-indented children.
fn parse_block(lines: &[Line], i: &mut usize, indent: usize) -> Result<Vec<El>, String> {
    let mut out = Vec::new();
    while *i < lines.len() && !is_screen_header(lines[*i].text) && lines[*i].indent >= indent {
        let line = &lines[*i];
        if line.indent > indent {
            return Err(format!("line {}: unexpected indentation (only card, row and col take children)", line.no));
        }
        *i += 1;
        let child_indent = lines.get(*i).filter(|l| l.indent > indent && !is_screen_header(l.text)).map(|l| l.indent);
        let mut children = || match child_indent {
            Some(ci) => parse_block(lines, i, ci),
            None => Ok(Vec::new()),
        };
        let el = match container(line.text) {
            Some(Container::Card(title)) => El::Card { title, children: children()? },
            Some(Container::Row) => El::Row(children()?),
            Some(Container::Col) => El::Col(children()?),
            None => leaf(line.text),
        };
        out.push(el);
    }
    Ok(out)
}

enum Container {
    Card(String),
    Row,
    Col,
}

/// `card Title:`, `box:`, `row:`, `col:` with nothing after the colon.
fn container(text: &str) -> Option<Container> {
    let head = text.strip_suffix(':')?.trim();
    let (word, rest) = head.split_once(char::is_whitespace).unwrap_or((head, ""));
    match word.to_ascii_lowercase().as_str() {
        "card" | "box" => Some(Container::Card(unquote(rest))),
        "row" if rest.is_empty() => Some(Container::Row),
        "col" | "column" if rest.is_empty() => Some(Container::Col),
        _ => None,
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let quoted = s.len() >= 2 && (s.starts_with('"') && s.ends_with('"') || s.starts_with('\'') && s.ends_with('\''));
    if quoted {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// `a | *b | c` → (["a", "b", "c"], Some(1)).
fn items(rest: &str) -> (Vec<String>, Option<usize>) {
    let mut active = None;
    let items = rest
        .split('|')
        .map(str::trim)
        .enumerate()
        .map(|(i, s)| match s.strip_prefix('*') {
            Some(s) => {
                active = Some(i);
                unquote(s)
            }
            None => unquote(s),
        })
        .collect();
    (items, active)
}

/// `label = value`; a bare value is a label.
fn label_value(rest: &str) -> (String, String) {
    match rest.split_once('=') {
        Some((l, v)) => (unquote(l), unquote(v)),
        None => (unquote(rest), String::new()),
    }
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
    let Some((kw, rest)) = text.split_once(':') else {
        return El::Text(unquote(text), Align::Auto);
    };
    let rest = rest.trim();
    match kw.trim().to_ascii_lowercase().as_str() {
        "text" => El::Text(unquote(rest), Align::Auto),
        "left" => El::Text(unquote(rest), Align::Left),
        "center" | "centre" => El::Text(unquote(rest), Align::Center),
        "right" => El::Text(unquote(rest), Align::Right),
        "header" => El::Header(items(rest).0),
        "input" | "select" => {
            let (label, value) = label_value(rest);
            El::Input { label, value, select: kw.trim().eq_ignore_ascii_case("select") }
        }
        "button" => El::Button(unquote(rest)),
        "image" => El::Image(unquote(rest)),
        "tabs" | "nav" => {
            let (items, active) = items(rest);
            El::Tabs { items, active, nav: kw.trim().eq_ignore_ascii_case("nav") }
        }
        "toggle" => {
            let (label, value) = label_value(rest);
            El::Toggle(label, is_on(&value))
        }
        "checkbox" => {
            let (label, value) = label_value(rest);
            El::Checkbox(label, is_on(&value))
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

/// One rendered line as display cells, and whether it is a divider that a
/// surrounding border should join with tees (`├───┤`).
type Block = Vec<(Vec<char>, bool)>;

struct Glyphs {
    ascii: bool,
}

impl Glyphs {
    /// tl, tr, bl, br, horizontal, vertical, left tee, right tee
    fn light(&self) -> [char; 8] {
        if self.ascii {
            ['+', '+', '+', '+', '-', '|', '+', '+']
        } else {
            ['┌', '┐', '└', '┘', '─', '│', '├', '┤']
        }
    }

    fn heavy(&self) -> [char; 6] {
        if self.ascii {
            ['#', '#', '#', '#', '=', '#']
        } else {
            ['┏', '┓', '┗', '┛', '━', '┃']
        }
    }

    fn pick(&self, unicode: &'static str, ascii: &'static str) -> &'static str {
        if self.ascii {
            ascii
        } else {
            unicode
        }
    }
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(1).max(1)
}

fn cells(s: &str) -> Vec<char> {
    let mut out = Vec::new();
    for c in s.chars() {
        out.push(c);
        if char_width(c) == 2 {
            out.push(WIDE);
        }
    }
    out
}

/// Cut to at most `w` cells, ending in `…` if anything was dropped.
fn truncate(mut c: Vec<char>, w: usize) -> Vec<char> {
    if c.len() <= w {
        return c;
    }
    if w == 0 {
        return Vec::new();
    }
    c.truncate(w - 1);
    if c.last() == Some(&WIDE) {
        c.pop();
        c.pop();
    } else if c.last().is_some_and(|&ch| char_width(ch) == 2) {
        c.pop();
    }
    c.push('…');
    c
}

fn pad(c: Vec<char>, w: usize, align: Align) -> Vec<char> {
    let c = truncate(c, w);
    let room = w - c.len();
    let left = match align {
        Align::Left | Align::Auto => 0,
        Align::Center => room / 2,
        Align::Right => room,
    };
    let mut out = vec![' '; left];
    out.extend(c);
    out.resize(w, ' ');
    out
}

/// Greedy word wrap into lines of at most `w` cells; long words are split.
fn wrap(s: &str, w: usize) -> Vec<Vec<char>> {
    let w = w.max(1);
    let mut out: Vec<Vec<char>> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    for word in s.split_whitespace() {
        let mut word = cells(word);
        if !cur.is_empty() && cur.len() + 1 + word.len() > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        while cur.len() + word.len() > w {
            let mut cut = w - cur.len();
            if cut > 0 && word.get(cut) == Some(&WIDE) {
                cut -= 1;
            }
            if cut == 0 {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            cur.extend(word.drain(..cut));
            out.push(std::mem::take(&mut cur));
        }
        cur.extend(word);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn line(c: Vec<char>) -> (Vec<char>, bool) {
    (c, false)
}

fn repeat(ch: char, n: usize) -> Vec<char> {
    vec![ch; n]
}

/// `left` + `fill`×n + `right`.
fn edge(left: char, fill: char, right: char, w: usize) -> Vec<char> {
    let mut c = vec![left];
    c.extend(repeat(fill, w.saturating_sub(2)));
    c.push(right);
    c
}

/// Wrap a block in a border; `title` goes on the top edge.
fn framed(title: &str, inner: Block, w: usize, g: &Glyphs) -> Block {
    let [tl, tr, bl, br, h, v, lt, rt] = g.light();
    let mut top = vec![tl];
    if !title.is_empty() {
        top.push(h);
        top.push(' ');
        top.extend(truncate(cells(title), w.saturating_sub(6)));
        top.push(' ');
    }
    top.resize(w - 1, h);
    top.push(tr);
    let mut out = vec![line(top)];
    for (c, rule) in inner {
        if rule {
            out.push(line(edge(lt, h, rt, w)));
        } else {
            let mut row = vec![v, ' '];
            row.extend(c);
            row.extend([' ', v]);
            out.push(line(row));
        }
    }
    out.push(line(edge(bl, h, br, w)));
    out
}

fn render_block(children: &[El], w: usize, g: &Glyphs) -> Block {
    children.iter().flat_map(|c| render(c, w, Align::Auto, g)).collect()
}

/// Render one element `w` cells wide. `hint` is the alignment a row gives
/// to `Auto` text.
fn render(el: &El, w: usize, hint: Align, g: &Glyphs) -> Block {
    let resolve = |a: Align| if a == Align::Auto { hint } else { a };
    match el {
        El::Text(s, a) => wrap(s, w).into_iter().map(|c| line(pad(c, w, resolve(*a)))).collect(),
        El::Spacer => vec![line(repeat(' ', w))],
        El::Divider => vec![(repeat(g.light()[4], w), true)],
        El::Header(slots) => {
            let mut row = repeat(' ', w);
            let n = slots.len().min(3);
            for (i, slot) in slots.iter().take(3).enumerate() {
                let align = match (i, n) {
                    (0, _) => Align::Left,
                    (1, 2) | (2, _) => Align::Right,
                    _ => Align::Center,
                };
                let placed = pad(cells(slot), w, align);
                for (cell, ch) in row.iter_mut().zip(placed) {
                    if ch != ' ' {
                        *cell = ch;
                    }
                }
            }
            vec![line(row), (repeat(g.light()[4], w), true)]
        }
        El::Input { label, value, select } => {
            let [tl, tr, bl, br, h, v, _, _] = g.light();
            let mut out: Block = Vec::new();
            if !label.is_empty() {
                out.extend(wrap(label, w).into_iter().map(|c| line(pad(c, w, Align::Left))));
            }
            let arrow = if *select { 2 } else { 0 };
            let mut mid = vec![v, ' '];
            mid.extend(pad(cells(value), w.saturating_sub(4 + arrow), Align::Left));
            if *select {
                mid.extend(cells(g.pick(" ▾", " v")));
            }
            mid.extend([' ', v]);
            out.push(line(edge(tl, h, tr, w)));
            out.push(line(mid));
            out.push(line(edge(bl, h, br, w)));
            out
        }
        El::Button(label) => {
            let [tl, tr, bl, br, h, v] = g.heavy();
            let mut mid = vec![v];
            mid.extend(pad(cells(label), w.saturating_sub(2), Align::Center));
            mid.push(v);
            vec![line(edge(tl, h, tr, w)), line(mid), line(edge(bl, h, br, w))]
        }
        El::Image(caption) => {
            let [tl, tr, bl, br, h, v, _, _] = g.light();
            let fill = if g.ascii { '.' } else { '░' };
            let inner = w.saturating_sub(2);
            let mut out = vec![line(edge(tl, h, tr, w))];
            for r in 0..3 {
                let mut row = vec![v];
                let mut body = repeat(fill, inner);
                if r == 1 && !caption.is_empty() {
                    let text = truncate(cells(&format!(" {caption} ")), inner);
                    let at = (inner - text.len()) / 2;
                    body.splice(at..at + text.len(), text);
                }
                row.extend(body);
                row.push(v);
                out.push(line(row));
            }
            out.push(line(edge(bl, h, br, w)));
            out
        }
        El::Card { title, children } => framed(title, render_block(children, w.saturating_sub(4), g), w, g),
        El::Col(children) => render_block(children, w, g),
        El::Row(children) => {
            let n = children.len();
            if n == 0 {
                return Vec::new();
            }
            let gaps = ROW_GAP * (n - 1);
            let base = w.saturating_sub(gaps) / n;
            let extra = w.saturating_sub(gaps) - base * n;
            let cols: Vec<Block> = children
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let cw = base + usize::from(i < extra);
                    let hint = match (i, n) {
                        (_, 1) | (0, _) => Align::Left,
                        (i, n) if i == n - 1 => Align::Right,
                        _ => Align::Center,
                    };
                    render(c, cw, hint, g)
                })
                .collect();
            let height = cols.iter().map(Vec::len).max().unwrap_or(0);
            (0..height)
                .map(|r| {
                    let mut row = Vec::new();
                    for (i, col) in cols.iter().enumerate() {
                        let cw = base + usize::from(i < extra);
                        if i > 0 {
                            row.extend(repeat(' ', ROW_GAP));
                        }
                        row.extend(col.get(r).map_or_else(|| repeat(' ', cw), |l| l.0.clone()));
                    }
                    row.resize(w, ' ');
                    line(row)
                })
                .collect()
        }
        El::Tabs { items, active, nav } => {
            let n = items.len().max(1);
            let base = w / n;
            let mut names = Vec::new();
            let mut under = repeat(if *nav { ' ' } else { g.light()[4] }, w);
            let mark = g.heavy()[4];
            let mut x = 0;
            for (i, item) in items.iter().enumerate() {
                let cw = if i == n - 1 { w - x } else { base };
                let cell = pad(cells(item), cw, Align::Center);
                if Some(i) == *active {
                    let start = cell.iter().position(|&c| c != ' ').unwrap_or(0);
                    let end = cell.iter().rposition(|&c| c != ' ').map_or(0, |e| e + 1);
                    for u in &mut under[x + start..x + end] {
                        *u = mark;
                    }
                }
                names.extend(cell);
                x += cw;
            }
            let mut out = Vec::new();
            if *nav {
                out.push((repeat(g.light()[4], w), true));
            }
            out.push(line(names));
            if !*nav || active.is_some() {
                out.push(line(under));
            }
            out
        }
        El::Toggle(label, on) => {
            let switch = match (on, g.ascii) {
                (true, false) => "(  ●)",
                (false, false) => "(○  )",
                (true, true) => "[ ON]",
                (false, true) => "[OFF]",
            };
            let sw = cells(switch);
            let mut row = pad(cells(label), w.saturating_sub(sw.len() + 1), Align::Left);
            row.push(' ');
            row.extend(sw);
            vec![line(pad(row, w, Align::Left))]
        }
        El::Checkbox(label, on) => {
            let box_ = if *on { "[x] " } else { "[ ] " };
            hanging(box_, label, w)
        }
        El::Radio(options, active) => {
            let dot = g.pick("•", "*");
            let options: Vec<String> = options
                .iter()
                .enumerate()
                .map(|(i, o)| if Some(i) == *active { format!("({dot}) {o}") } else { format!("( ) {o}") })
                .collect();
            let one_line = cells(&options.join("   "));
            if one_line.len() <= w {
                vec![line(pad(one_line, w, Align::Left))]
            } else {
                options.iter().flat_map(|o| wrap(o, w)).map(|c| line(pad(c, w, Align::Left))).collect()
            }
        }
        El::Bullet(text, right) => {
            let right = right.as_deref().map(cells).unwrap_or_default();
            let room = if right.is_empty() { w } else { w.saturating_sub(right.len() + 1) };
            let mut out = hanging(g.pick("• ", "* "), text, room);
            for (row, _) in out.iter_mut() {
                row.resize(w, ' ');
            }
            if let Some((first, _)) = out.first_mut() {
                let at = w - right.len().min(w);
                first.splice(at.., truncate(right, w));
            }
            out
        }
    }
}

/// `marker` then `text`, wrapped with continuation lines indented under it.
fn hanging(marker: &str, text: &str, w: usize) -> Block {
    let m = cells(marker);
    wrap(text, w.saturating_sub(m.len()))
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut row = if i == 0 { m.clone() } else { repeat(' ', m.len()) };
            row.extend(c);
            line(pad(row, w, Align::Left))
        })
        .collect()
}

fn to_string(c: &[char]) -> String {
    c.iter().filter(|&&ch| ch != WIDE).collect::<String>().trim_end().to_string()
}

/// Render every screen at its own width.
pub fn render_wireframe(wf: &Wireframe, ascii: bool) -> String {
    render_wireframe_fit(wf, ascii, 0).text
}

/// Render screens side by side, wrapping to a new row of screens when the
/// next one would pass `max_width` (0 = unlimited). A screen wider than the
/// budget is narrowed, down to `MIN_WIDTH`.
pub fn render_wireframe_fit(wf: &Wireframe, ascii: bool, max_width: usize) -> WireFitted {
    let g = Glyphs { ascii };
    let screens: Vec<Block> = wf
        .screens
        .iter()
        .map(|s| {
            let w = if max_width == 0 { s.width } else { s.width.min(max_width).max(MIN_WIDTH) };
            framed(&s.title, render_block(&s.children, w - 4, &g), w, &g)
        })
        .collect();
    let mut rows: Vec<Vec<&Block>> = Vec::new();
    let mut used = 0;
    for s in &screens {
        let w = s[0].0.len();
        if rows.is_empty() || (max_width > 0 && used + SCREEN_GAP + w > max_width) {
            rows.push(Vec::new());
            used = 0;
        } else {
            used += SCREEN_GAP;
        }
        rows.last_mut().unwrap().push(s);
        used += w;
    }
    let mut lines: Vec<String> = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        if ri > 0 {
            lines.push(String::new());
        }
        let height = row.iter().map(|s| s.len()).max().unwrap_or(0);
        for r in 0..height {
            let mut out: Vec<char> = Vec::new();
            for (i, s) in row.iter().enumerate() {
                if i > 0 {
                    out.extend(repeat(' ', SCREEN_GAP));
                }
                let w = s[0].0.len();
                out.extend(s.get(r).map_or_else(|| repeat(' ', w), |l| l.0.clone()));
            }
            lines.push(to_string(&out));
        }
    }
    let width = lines.iter().map(|l| cells(l).len()).max().unwrap_or(0);
    WireFitted { text: lines.join("\n"), width, fits: max_width == 0 || width <= max_width }
}
