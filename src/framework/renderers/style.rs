//! Colour and emphasis for the text renderers.
//!
//! Only the formats read by a person are painted: the machine formats never see
//! a style. Every choice here is made from the value's own kind and the
//! column's declared type, so a plugin never says how its output should look.
//!
//! A style is applied after a cell has been measured and padded, which keeps
//! escape sequences out of every width calculation.

use std::borrow::Cow;
use std::io::IsTerminal;

/// When to colour the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum When {
    /// Colour a terminal, and only a terminal.
    #[default]
    Auto,
    Always,
    Never,
}

/// The palette to draw from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Brighter colours, for a dark background.
    #[default]
    Dark,
    /// Deeper colours, for a light background.
    Light,
    /// No colour at all.
    Mono,
}

/// What a piece of text means, rather than what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Text that says nothing special.
    Plain,
    /// A column name.
    Header,
    /// Text that is there but not the point: the banner, a summary, the marks
    /// that show a tree's shape.
    Faint,
    /// An address or an offset, which is scanned rather than read.
    Address,
    /// A point in time.
    Time,
    /// A yes-or-no answer.
    Bool(bool),
    /// A process ID, coloured from its own value so that the same process
    /// keeps the same colour in every plugin.
    Pid(u8),
    /// Bytes that could not be read.
    Unreadable,
    /// Bytes that were read but not understood.
    Unparsable,
    /// A field that does not apply here.
    NotApplicable,
    /// A cell a filter matched.
    Match,
    /// Help text: `usage:`, the program, options, their values and headings.
    Usage,
    Program,
    Option,
    OptionStrong,
    OptionLong,
    OptionLongStrong,
    Positional,
    PositionalStrong,
    Metavar,
    MetavarStrong,
    Heading,
}

/// The colours a theme gives to the values a plugin produces.
///
/// Help is painted the way the reference implementation's parser paints it,
/// whichever theme is in use, so those codes are shared.
#[derive(Debug, Clone, Copy)]
struct Theme {
    header: &'static str,
    faint: &'static str,
    address: &'static str,
    time: &'static str,
    yes: &'static str,
    unreadable: &'static str,
    unparsable: &'static str,
    not_applicable: &'static str,
    highlight: &'static str,
    pids: [&'static str; 6],
}

const USAGE: &str = "1;34";
const PROGRAM: &str = "1;35";
const OPTION: &str = "32";
const OPTION_STRONG: &str = "1;32";
const OPTION_LONG: &str = "36";
const OPTION_LONG_STRONG: &str = "1;36";
const POSITIONAL: &str = "32";
const POSITIONAL_STRONG: &str = "1;32";
const METAVAR: &str = "33";
const METAVAR_STRONG: &str = "1;33";
const HEADING: &str = "1;34";

impl Theme {
    const DARK: Theme = Theme {
        header: "1",
        faint: "2",
        address: "96",
        time: "94",
        yes: "92",
        unreadable: "91",
        unparsable: "93",
        not_applicable: "2",
        highlight: "1;4",
        pids: ["96", "95", "92", "93", "94", "97"],
    };

    const LIGHT: Theme = Theme {
        header: "1",
        faint: "2",
        address: "36",
        time: "34",
        yes: "32",
        unreadable: "31",
        unparsable: "33",
        not_applicable: "2",
        highlight: "1;4",
        pids: ["36", "35", "32", "33", "34", "30"],
    };

    /// The code a role is painted with, empty where it is left alone.
    fn code(&self, role: Role) -> &'static str {
        match role {
            Role::Plain => "",
            Role::Header => self.header,
            Role::Faint => self.faint,
            Role::Address => self.address,
            Role::Time => self.time,
            Role::Bool(true) => self.yes,
            Role::Bool(false) => self.faint,
            Role::Pid(index) => self.pids[index as usize % self.pids.len()],
            Role::Unreadable => self.unreadable,
            Role::Unparsable => self.unparsable,
            Role::NotApplicable => self.not_applicable,
            Role::Match => self.highlight,
            Role::Usage => USAGE,
            Role::Program => PROGRAM,
            Role::Option => OPTION,
            Role::OptionStrong => OPTION_STRONG,
            Role::OptionLong => OPTION_LONG,
            Role::OptionLongStrong => OPTION_LONG_STRONG,
            Role::Positional => POSITIONAL,
            Role::PositionalStrong => POSITIONAL_STRONG,
            Role::Metavar => METAVAR,
            Role::MetavarStrong => METAVAR_STRONG,
            Role::Heading => HEADING,
        }
    }
}

/// How the output should be painted.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    theme: Theme,
    enabled: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self::plain()
    }
}

impl Style {
    /// A style that prints text unchanged.
    pub fn plain() -> Self {
        Self {
            theme: Theme::DARK,
            enabled: false,
        }
    }

    /// The style the command line asked for.
    pub fn new(when: When, kind: ThemeKind) -> Self {
        let theme = match kind {
            ThemeKind::Light => Theme::LIGHT,
            _ => Theme::DARK,
        };
        Self {
            theme,
            enabled: kind != ThemeKind::Mono && when.wanted(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The text as it should be printed, painted when styling is on.
    pub fn paint<'a>(&self, role: Role, text: &'a str) -> Cow<'a, str> {
        if !self.enabled || text.is_empty() {
            return Cow::Borrowed(text);
        }
        let code = self.theme.code(role);
        if code.is_empty() {
            return Cow::Borrowed(text);
        }
        Cow::Owned(format!("\x1b[{code}m{text}\x1b[0m"))
    }
}

impl When {
    /// Whether a terminal that did not opt out asked for colour.
    fn wanted(self) -> bool {
        match self {
            When::Always => true,
            When::Never => false,
            When::Auto => {
                // The conventions a terminal user already has: `NO_COLOR`
                // turns colour off everywhere, `FORCE_COLOR` turns it on for a
                // pipe, and a dumb terminal is never painted.
                if std::env::var_os("NO_COLOR").is_some() {
                    return false;
                }
                if let Some(value) = std::env::var_os("FORCE_COLOR") {
                    return value.to_string_lossy() != "0";
                }
                if std::env::var_os("CLICOLOR").is_some_and(|value| value.to_string_lossy() == "0") {
                    return false;
                }
                if std::env::var_os("TERM").is_some_and(|value| value.to_string_lossy() == "dumb") {
                    return false;
                }
                std::io::stdout().is_terminal()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_style_changes_nothing() {
        assert_eq!(Style::plain().paint(Role::Unreadable, "-"), "-");
    }

    #[test]
    fn a_forced_style_wraps_and_resets() {
        let style = Style::new(When::Always, ThemeKind::Dark);
        assert_eq!(style.paint(Role::Address, "0x10"), "\x1b[96m0x10\x1b[0m");
    }

    #[test]
    fn monochrome_wins_over_always() {
        let style = Style::new(When::Always, ThemeKind::Mono);
        assert_eq!(style.paint(Role::Address, "0x10"), "0x10");
    }
}
