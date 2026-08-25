//! Whether to emit colour at all, and what the codes are when we do.
//!
//! Colour is a guess about where the output is going, and the guess is wrong
//! often enough that three separate things can overrule it. Getting that
//! precedence wrong writes escape sequences into a file someone is diffing.

/// The reset that ends every painted span.
const RESET: &str = "\u{1b}[0m";

/// The value of `CLICOLOR` that means "no colour". Any other value is not a
/// veto — `CLICOLOR=1` is the older way of asking *for* colour, so treating
/// the variable's presence as a refusal would invert it.
const CLICOLOR_OFF: &str = "0";

/// What the user asked for on the command line.
///
/// Distinct from what we will actually do, which is [`Palette`]: `Auto` is a
/// question, not an answer, and the environment gets to answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    /// Decide from the environment and whether stdout is a terminal.
    #[default]
    Auto,
    /// Paint regardless — for piping into a pager that understands escapes.
    Always,
    /// Never paint.
    Never,
}

impl ColorChoice {
    /// Reads the value of a `--color` flag.
    ///
    /// Surrounding whitespace is stripped and ASCII case folded, because this
    /// is typed by a person. `None` is an unrecognised value, which a caller
    /// should report as a usage error rather than quietly treating as `Auto` —
    /// silently ignoring `--color=yes` gives the user no way to learn they
    /// were not understood.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// The parts of the world that get a say in whether we paint.
///
/// Taken as data rather than read here, so the precedence between them is a
/// pure function that can be tested for every combination instead of by
/// setting variables in a live process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Environment<'a> {
    /// `NO_COLOR`, if set. The convention is that *any* non-empty value
    /// suppresses colour, whatever it says.
    pub no_color: Option<&'a str>,
    /// `CLICOLOR`, if set. Only the literal `0` suppresses colour.
    pub clicolor: Option<&'a str>,
    /// Whether stdout is attached to a terminal. Escape sequences written into
    /// a pipe or a file are corruption, not colour.
    pub stdout_is_terminal: bool,
}

impl Environment<'_> {
    /// Whether anything in the environment objects to colour.
    fn forbids_color(&self) -> bool {
        let refused_by_no_color = self.no_color.is_some_and(|value| !value.is_empty());
        let refused_by_clicolor = self.clicolor == Some(CLICOLOR_OFF);
        refused_by_no_color || refused_by_clicolor || !self.stdout_is_terminal
    }
}

/// What a span of text means, before we know whether we are painting.
///
/// Named for meaning rather than for colour so that a call site reads as the
/// judgement it is making. The mapping to codes is [`Palette`]'s business, and
/// a dimmed row overrides all of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// No styling of its own. Most cells are this.
    Plain,
    /// A heading.
    Bold,
    /// Present but not worth attention.
    Dim,
    /// Something is wrong and somebody must act.
    Red,
    /// Nothing is in the way.
    Green,
    /// Waiting on something that will finish by itself.
    Yellow,
    /// Waiting on a person.
    Blue,
    /// A rule of the repository is holding it.
    Magenta,
}

impl Style {
    /// The escape sequence that opens this style, or `None` for [`Style::Plain`].
    const fn code(self) -> Option<&'static str> {
        match self {
            Self::Plain => None,
            Self::Bold => Some("\u{1b}[1m"),
            Self::Dim => Some("\u{1b}[2m"),
            Self::Red => Some("\u{1b}[31m"),
            Self::Green => Some("\u{1b}[32m"),
            Self::Yellow => Some("\u{1b}[33m"),
            Self::Blue => Some("\u{1b}[34m"),
            Self::Magenta => Some("\u{1b}[35m"),
        }
    }
}

/// The decision about colour, already made, ready to paint with.
///
/// Built once per run from a [`ColorChoice`] and an [`Environment`], then
/// handed down. Nothing below this reads a variable or asks about a terminal,
/// so a renderer cannot accidentally paint under `NO_COLOR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    painting: bool,
    row_is_dimmed: bool,
}

impl Palette {
    /// Settles whether to paint, from what the user asked and what the world
    /// says.
    ///
    /// An explicit `--color` wins outright in both directions: it is the user
    /// speaking now, where the variables are how they set their shell up once.
    /// Only [`ColorChoice::Auto`] consults the environment, and there any one
    /// objection is enough.
    #[must_use]
    pub fn resolve(choice: ColorChoice, environment: &Environment<'_>) -> Self {
        let painting = match choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => !environment.forbids_color(),
        };
        Self {
            painting,
            row_is_dimmed: false,
        }
    }

    /// A palette that never paints, for text that is data rather than display.
    ///
    /// Distinct from resolving [`ColorChoice::Never`]: that is a decision about
    /// a terminal, and this is the absence of one. JSON is not a terminal and
    /// has no environment to consult.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            painting: false,
            row_is_dimmed: false,
        }
    }

    /// The same decision about colour, but with every style flattened to dim.
    ///
    /// A row is dimmed to say "you cannot act on this yet", and that has to
    /// survive the whole row. Opening the row with a dim code and letting the
    /// cells keep their own colours does not work: the first cell that paints
    /// itself ends with a reset, which clears the dim, and the rest of the row
    /// reads as ordinary. Suppressing the cells' colours instead keeps one
    /// meaning per row.
    #[must_use]
    pub const fn dimmed(self) -> Self {
        Self {
            painting: self.painting,
            row_is_dimmed: true,
        }
    }

    /// Wraps `text` in the escapes for `style`, or hands it back untouched.
    #[must_use]
    pub fn paint(self, style: Style, text: &str) -> String {
        if !self.painting {
            return text.to_owned();
        }
        let effective = if self.row_is_dimmed {
            Style::Dim
        } else {
            style
        };
        effective
            .code()
            .map_or_else(|| text.to_owned(), |code| format!("{code}{text}{RESET}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorChoice, Environment, Palette, Style};

    /// An interactive terminal with neither variable set — the only condition
    /// under which `auto` paints anything.
    fn interactive() -> Environment<'static> {
        Environment {
            no_color: None,
            clicolor: None,
            stdout_is_terminal: true,
        }
    }

    #[test]
    fn auto_paints_only_an_interactive_terminal() {
        for (stdout_is_terminal, painted) in [(true, true), (false, false)] {
            let environment = Environment {
                stdout_is_terminal,
                ..interactive()
            };
            assert_eq!(
                Palette::resolve(ColorChoice::Auto, &environment).paint(Style::Red, "x"),
                if painted { "\u{1b}[31mx\u{1b}[0m" } else { "x" },
                "a pipe gets no escapes and a terminal does (tty={stdout_is_terminal})"
            );
        }
    }

    #[test]
    fn no_color_silences_an_interactive_terminal_unless_it_is_empty() {
        // The NO_COLOR convention is "present and not empty". An empty value
        // is a variable someone unset badly, not a request.
        for (value, painted) in [
            (None, true),
            (Some(""), true),
            (Some("1"), false),
            (Some("0"), false),
            (Some("anything at all"), false),
        ] {
            let environment = Environment {
                no_color: value,
                ..interactive()
            };
            assert_eq!(
                Palette::resolve(ColorChoice::Auto, &environment).paint(Style::Red, "x"),
                if painted { "\u{1b}[31mx\u{1b}[0m" } else { "x" },
                "NO_COLOR={value:?} must {} colour",
                if painted { "allow" } else { "suppress" }
            );
        }
    }

    #[test]
    fn clicolor_zero_silences_an_interactive_terminal() {
        // Only the literal `0` means off. `CLICOLOR=1` is the older way of
        // asking *for* colour, so treating any value as a veto would invert it.
        for (value, painted) in [(None, true), (Some("1"), true), (Some("0"), false)] {
            let environment = Environment {
                clicolor: value,
                ..interactive()
            };
            assert_eq!(
                Palette::resolve(ColorChoice::Auto, &environment).paint(Style::Red, "x"),
                if painted { "\u{1b}[31mx\u{1b}[0m" } else { "x" },
                "CLICOLOR={value:?}"
            );
        }
    }

    #[test]
    fn an_explicit_flag_overrules_the_environment_in_both_directions() {
        // The flag is the user speaking now; the variables are how they set
        // their shell up once. Someone piping into `less -R` needs `--color
        // always` to win, and someone who asked for none needs `--color never`
        // to win over an interactive terminal.
        let hostile = Environment {
            no_color: Some("1"),
            clicolor: Some("0"),
            stdout_is_terminal: false,
        };
        assert_eq!(
            Palette::resolve(ColorChoice::Always, &hostile).paint(Style::Red, "x"),
            "\u{1b}[31mx\u{1b}[0m",
            "--color always paints into a pipe with both variables set against it"
        );
        assert_eq!(
            Palette::resolve(ColorChoice::Never, &interactive()).paint(Style::Red, "x"),
            "x",
            "--color never stays silent on an interactive terminal"
        );
    }

    #[test]
    fn every_style_has_its_own_code_and_resets_after_itself() {
        // A style that shared a code with another would render two different
        // meanings identically; one that forgot to reset would bleed into the
        // rest of the line.
        let palette = Palette::resolve(ColorChoice::Always, &interactive());
        let cases: [(Style, &str); 8] = [
            (Style::Plain, "x"),
            (Style::Bold, "\u{1b}[1mx\u{1b}[0m"),
            (Style::Dim, "\u{1b}[2mx\u{1b}[0m"),
            (Style::Red, "\u{1b}[31mx\u{1b}[0m"),
            (Style::Green, "\u{1b}[32mx\u{1b}[0m"),
            (Style::Yellow, "\u{1b}[33mx\u{1b}[0m"),
            (Style::Blue, "\u{1b}[34mx\u{1b}[0m"),
            (Style::Magenta, "\u{1b}[35mx\u{1b}[0m"),
        ];
        for (style, painted) in cases {
            assert_eq!(palette.paint(style, "x"), painted, "{style:?}");
        }
    }

    #[test]
    fn a_plain_style_adds_nothing_even_when_colour_is_on() {
        // Most cells carry no colour of their own. Wrapping them in a
        // no-op sequence would make every row longer for nothing and would
        // interrupt a row-level dim.
        let palette = Palette::resolve(ColorChoice::Always, &interactive());
        assert_eq!(palette.paint(Style::Plain, "#4225"), "#4225");
    }

    #[test]
    fn a_dimmed_row_stays_dim_all_the_way_across() {
        // A row is dimmed to say "you cannot act on this". Opening the row
        // with a dim code and letting the cells' own colours reset it stops the
        // dimming at the first coloured cell, leaving the rest of the row
        // reading as ordinary. Suppressing the cell colours
        // instead keeps one meaning per row.
        let dimmed = Palette::resolve(ColorChoice::Always, &interactive()).dimmed();
        for style in [Style::Plain, Style::Red, Style::Green, Style::Bold] {
            assert_eq!(
                dimmed.paint(style, "x"),
                "\u{1b}[2mx\u{1b}[0m",
                "{style:?} must come out dim in a dimmed row, not in its own colour"
            );
        }
    }

    #[test]
    fn dimming_a_palette_that_is_not_painting_still_emits_nothing() {
        let dimmed = Palette::resolve(ColorChoice::Never, &interactive()).dimmed();
        assert_eq!(dimmed.paint(Style::Red, "x"), "x");
    }

    #[test]
    fn a_color_choice_reads_back_from_what_a_user_would_type() {
        for (typed, choice) in [
            ("auto", Some(ColorChoice::Auto)),
            ("always", Some(ColorChoice::Always)),
            ("never", Some(ColorChoice::Never)),
            ("Always", Some(ColorChoice::Always)),
            ("  never  ", Some(ColorChoice::Never)),
            ("yes", None),
            ("", None),
        ] {
            assert_eq!(ColorChoice::parse(typed), choice, "--color {typed:?}");
        }
    }
}
