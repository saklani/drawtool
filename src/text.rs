//! Measuring and wrapping text, and picking the rendering that fits.

use unicode_width::UnicodeWidthChar;

#[must_use]
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
        if best.as_ref().is_none_or(|b| width < b.width) {
            best = Some(Fit { text, width, fits: false });
        }
    }
    best.expect("at least one attempt")
}
