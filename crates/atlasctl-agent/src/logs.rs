// SPDX-License-Identifier: AGPL-3.0-only

//! The tail of a launch's log.
//!
//! Read for diagnosis, not as a data source. Every *number* this project shows
//! comes from the engine's `/metrics`; the log is what an operator reads when
//! something did not start, and nothing is parsed out of it. That split is
//! deliberate — the tool this replaces derived throughput from log text, and
//! every reword silently broke it.
//!
//! A log line is attacker-influenced text about to be rendered in a browser, so
//! it is sanitised here rather than at the point of display. Doing it once, at
//! the boundary, is what stops the next surface forgetting.

use anyhow::Result;

/// Longest line kept. A single unbounded line is a denial of service against
/// the layout, and nothing useful is that wide.
const MAX_LINE: usize = 2000;

/// Most lines any request can ask for, however many it asks for.
pub const MAX_LINES: u32 = 500;

/// Reads a launch's recent output.
pub trait LogSource: Send + Sync {
    /// The last `lines` lines of a launch's log, and whether it is still
    /// running.
    ///
    /// # Errors
    /// If there is no container for that recipe, or the runtime refuses.
    fn tail(&self, recipe: &str, lines: u32) -> Result<LogTail>;
}

/// What a launch has said recently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTail {
    /// The container the lines came from.
    pub container: String,
    /// Lines, oldest first.
    pub lines: Vec<String>,
    /// Whether that container is still running.
    pub running: bool,
}

/// Strip what a terminal would act on, and cap the width.
///
/// ANSI escapes are removed rather than rendered: the engine colours its output,
/// and a browser shows the raw escape bytes as mojibake that hides the message
/// they decorate. Control characters and bidi overrides go for the same reason
/// they go from hostnames — a log line must not be able to reorder the text
/// around it.
#[must_use]
pub fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_LINE));
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        // CSI and OSC sequences: consume to their terminator.
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for t in chars.by_ref() {
                        if t.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for t in chars.by_ref() {
                        if t == '\u{7}' || t == '\u{1b}' {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        let n = c as u32;
        // C0 (tab kept, it is meaningful in log output), DEL and C1.
        if (n < 0x20 && c != '\t') || (0x7f..=0x9f).contains(&n) {
            continue;
        }
        // Bidi overrides and isolates.
        if (0x202a..=0x202e).contains(&n) || (0x2066..=0x2069).contains(&n) {
            continue;
        }
        out.push(c);
        if out.chars().count() >= MAX_LINE {
            out.push('…');
            break;
        }
    }
    out
}

/// Clamp a requested line count to something bounded.
///
/// Zero means "the default", not "none": a page that forgets to say gets a
/// useful tail rather than an empty box it cannot tell from a silent launch.
#[must_use]
pub fn clamp_lines(asked: u32) -> u32 {
    if asked == 0 {
        return 200;
    }
    asked.min(MAX_LINES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine colours its output. A browser shows raw escape bytes as
    /// mojibake that hides the message they decorate.
    #[test]
    fn ansi_colour_is_removed_and_the_message_survives() {
        let raw = "\u{1b}[2m2026-08-26T05:35:05Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Atlas Spark starting...";
        assert_eq!(
            sanitize(raw),
            "2026-08-26T05:35:05Z  INFO Atlas Spark starting..."
        );
    }

    #[test]
    fn an_osc_sequence_is_removed_too() {
        assert_eq!(sanitize("\u{1b}]0;title\u{7}after"), "after");
    }

    /// A log line must not be able to reorder the text around it.
    #[test]
    fn bidi_overrides_and_controls_are_stripped_but_tabs_survive() {
        assert_eq!(sanitize("a\u{202e}b\u{2066}c"), "abc");
        assert_eq!(sanitize("a\u{0}b\u{7f}c\u{9b}d"), "abcd");
        assert_eq!(sanitize("key\tvalue"), "key\tvalue");
    }

    /// An unbounded line is a denial of service against the layout.
    #[test]
    fn a_very_long_line_is_capped_and_marked() {
        let out = sanitize(&"x".repeat(MAX_LINE * 3));
        assert_eq!(out.chars().count(), MAX_LINE + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let line = "Error: Snapshot 'db821d7' has no weight files (metadata-only)";
        assert_eq!(sanitize(line), line);
    }

    /// A truncated escape at end of line must not swallow the rest of the log.
    #[test]
    fn a_dangling_escape_does_not_eat_the_next_line() {
        assert_eq!(sanitize("done\u{1b}"), "done");
        assert_eq!(sanitize("done\u{1b}["), "done");
    }

    mod bounds {
        use super::super::*;

        /// Zero means "the default", not "none": a page that forgets to say
        /// gets a useful tail rather than an empty box it cannot tell from a
        /// silent launch.
        #[test]
        fn zero_means_the_default_rather_than_nothing() {
            assert_eq!(clamp_lines(0), 200);
        }

        #[test]
        fn a_request_is_capped_however_large_it_is() {
            assert_eq!(clamp_lines(u32::MAX), MAX_LINES);
            assert_eq!(clamp_lines(MAX_LINES + 1), MAX_LINES);
        }

        #[test]
        fn a_reasonable_request_is_honoured() {
            assert_eq!(clamp_lines(50), 50);
        }
    }
}
