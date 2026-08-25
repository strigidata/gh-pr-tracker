//! How wide a string is on a terminal, which is not how long it is.
//!
//! Three things make the two differ, and a column that assumes otherwise
//! misaligns for all three: colour escapes take bytes and no cells, CJK and
//! emoji take one character and two cells, and a combining mark takes a
//! character and no cells.

use unicode_width::UnicodeWidthChar as _;

/// The byte that opens an escape sequence.
const ESC: char = '\u{1b}';

/// The byte that follows [`ESC`] in a Control Sequence Introducer — the family
/// every colour code belongs to.
const CSI: char = '[';

/// Steps over an escape sequence, returning what is left after it.
///
/// A CSI sequence runs from `ESC [` to the first byte in the range `@` to `~`,
/// which is the terminator. Anything else beginning with `ESC` is a two-byte
/// escape this renderer never emits; one character is consumed so that a
/// stray one cannot spin the caller.
fn skip_escape(rest: &str) -> &str {
    let mut chars = rest.chars();
    match chars.next() {
        Some(CSI) => {
            let after_bracket = chars.as_str();
            after_bracket
                .char_indices()
                .find(|&(_, ch)| matches!(ch, '@'..='~'))
                .map_or("", |(index, ch)| {
                    after_bracket
                        .get(index + ch.len_utf8()..)
                        .unwrap_or_default()
                })
        }
        Some(_) | None => chars.as_str(),
    }
}

/// How many terminal cells `text` occupies once escape sequences are
/// discounted.
///
/// This is the measurement every column in the board is aligned on. Character
/// count is the wrong answer three ways over — see the module documentation.
/// The easiest of the three to get wrong is stripping `[31m` while leaving the
/// escape byte itself in the count, which makes every coloured cell read one
/// cell wider than it draws.
#[must_use]
pub fn visible_width(text: &str) -> usize {
    let mut cells = 0;
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        rest = rest.get(ch.len_utf8()..).unwrap_or_default();
        if ch == ESC {
            rest = skip_escape(rest);
        } else {
            // `None` is a control character, which draws nothing.
            cells += ch.width().unwrap_or(0);
        }
    }
    cells
}

/// Fills `text` out to `cells` columns with spaces.
///
/// A cell already at or past its column still gets one space, so that an
/// over-long value pushes the rest of the row right instead of running into
/// its neighbour and reading as one word. Nothing is truncated here; a column
/// that must not overflow says so by calling [`truncate_to`] first.
#[must_use]
pub fn pad_to(text: &str, cells: usize) -> String {
    let used = visible_width(text);
    let spaces = cells.saturating_sub(used).max(1);
    let mut padded = String::with_capacity(text.len() + spaces);
    padded.push_str(text);
    for _ in 0..spaces {
        padded.push(' ');
    }
    padded
}

/// Cuts `text` down to at most `cells` columns.
///
/// The cut lands on a character boundary, so a wide glyph that would straddle
/// the limit is dropped rather than halved — half a glyph is not a thing a
/// terminal can draw, and the byte-slice for it would not be valid UTF-8
/// either.
///
/// Escape sequences are measured as zero-width, so a cut can in principle land
/// between a colour and its reset. Nothing here does that: truncation is
/// applied to raw values before any paint.
#[must_use]
pub fn truncate_to(text: &str, cells: usize) -> &str {
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        used += ch.width().unwrap_or(0);
        if used > cells {
            return text.get(..index).unwrap_or(text);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{pad_to, truncate_to, visible_width};

    /// A red foreground, then a reset. Written from the escape byte up so the
    /// test does not depend on the palette agreeing with it.
    const RED: &str = "\u{1b}[31m";
    const RESET: &str = "\u{1b}[0m";

    #[test]
    fn escape_sequences_occupy_no_cells() {
        // Stripping `[31m` but leaving the escape byte itself in the count
        // makes every coloured cell measure one cell too wide, and the column
        // loses a space. The whole sequence has to go, escape byte included.
        for (text, cells) in [
            ("plain", 5),
            ("\u{1b}[31mplain\u{1b}[0m", 5),
            ("\u{1b}[1m\u{1b}[2mplain\u{1b}[0m", 5),
            ("\u{1b}[38;5;214mplain\u{1b}[0m", 5),
            ("\u{1b}[0m", 0),
            ("", 0),
        ] {
            assert_eq!(
                visible_width(text),
                cells,
                "{text:?} occupies {cells} cells once the escapes are discounted"
            );
        }
    }

    #[test]
    fn a_wide_glyph_occupies_two_cells() {
        // Terminals give CJK and emoji two cells each. Counting characters
        // instead pads such a cell two spaces short per glyph, which shifts
        // every column to its right.
        for (text, cells) in [
            ("ab", 2),
            ("日本", 4),
            ("a日b", 4),
            ("日\u{1b}[31m本\u{1b}[0m", 4),
        ] {
            assert_eq!(
                visible_width(text),
                cells,
                "{text:?} occupies {cells} cells"
            );
        }
    }

    #[test]
    fn a_combining_mark_occupies_no_cells_of_its_own() {
        // `e` plus a combining acute is one cell, not two: the mark is drawn
        // on top of the letter it follows.
        assert_eq!(visible_width("e\u{301}"), 1);
        assert_eq!(visible_width("caf\u{e9}"), 4);
        assert_eq!(visible_width("cafe\u{301}"), 4);
    }

    #[test]
    fn padding_fills_a_column_to_its_visible_width() {
        for (text, column, padded) in [("ab", 5, "ab   "), ("", 3, "   "), ("日", 5, "日   ")] {
            assert_eq!(
                pad_to(text, column),
                padded,
                "{text:?} padded into {column} cells"
            );
        }
    }

    #[test]
    fn a_coloured_cell_is_padded_by_what_it_shows_not_by_what_it_holds() {
        // The whole point of measuring visible width. The escapes travel with
        // the cell, so the string is longer than the column while the glyphs
        // are not.
        let painted = format!("{RED}ok{RESET}");
        assert_eq!(
            pad_to(&painted, 5),
            format!("{RED}ok{RESET}   "),
            "three spaces, because two cells of the five are used"
        );
    }

    #[test]
    fn an_oversized_cell_keeps_one_space_so_two_columns_never_touch() {
        // A cell wider than its column pushes the rest of the row right rather
        // than being cut. Losing the separating space instead would run two
        // values together and read as one word.
        for (text, column) in [("abcdef", 3), ("abc", 3), ("日本", 3)] {
            let padded = pad_to(text, column);
            assert!(
                padded.starts_with(text) && padded.ends_with(' '),
                "{text:?} in {column} cells must keep its text and end in a space, got {padded:?}"
            );
        }
        assert_eq!(pad_to("abcdef", 3), "abcdef ");
    }

    #[test]
    fn truncation_counts_cells_so_a_wide_glyph_cannot_overrun() {
        // Truncating by characters lets a column of thirteen "characters" draw
        // twenty-six cells. The cut is on a character boundary, so a glyph is
        // dropped rather than halved.
        for (text, cells, kept) in [
            ("octocat", 13, "octocat"),
            ("a-very-long-login-name", 13, "a-very-long-l"),
            ("日本語日本語日本語", 5, "日本"),
            ("日本語", 6, "日本語"),
            ("a日本", 2, "a"),
        ] {
            assert_eq!(
                truncate_to(text, cells),
                kept,
                "{text:?} cut to {cells} cells"
            );
            assert!(
                visible_width(truncate_to(text, cells)) <= cells,
                "{text:?} must never exceed {cells} cells once cut"
            );
        }
    }
}
