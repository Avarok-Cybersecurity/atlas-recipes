// SPDX-License-Identifier: AGPL-3.0-only

//! POSIX shell quoting for the human-readable rendering of a command.
//!
//! This is display only. Execution goes through an argv vector and never a
//! shell, so quoting can never be the difference between a safe and an unsafe
//! launch. It exists so the string the website shows can be pasted into a
//! terminal and do exactly what atlasctl would have done.

/// Characters that are safe unquoted in every POSIX shell.
fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | ',' | '@' | '+')
}

/// Quote one argument the way `shlex.quote` does.
///
/// Single quotes are the strong form — nothing inside them is special — so the
/// only case needing care is a literal single quote, which is closed, escaped,
/// and reopened.
pub fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(is_safe) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r#"'"'"'"#))
}

/// Join an argv into a pasteable command line.
pub fn shell_join<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|a| shell_quote(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_arguments_are_left_alone() {
        for s in [
            "docker",
            "run",
            "--ipc=host",
            "avarok/atlas-gb10:latest",
            "0.88",
            "8888",
        ] {
            assert_eq!(shell_quote(s), s, "{s} needed no quoting");
        }
    }

    #[test]
    fn the_empty_string_becomes_an_explicit_empty_argument() {
        // `--entrypoint ""` is load-bearing: it clears the image entrypoint.
        // Rendering it as nothing at all would change what the command does.
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_metacharacters_are_neutralized() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a;b"), "'a;b'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("a|b"), "'a|b'");
        assert_eq!(shell_quote("a\nb"), "'a\nb'");
        assert_eq!(shell_quote("`id`"), "'`id`'");
    }

    #[test]
    fn embedded_single_quotes_round_trip() {
        // The classic quoting trap: close, escape, reopen.
        assert_eq!(shell_quote("it's"), r#"'it'"'"'s'"#);
    }

    #[test]
    fn joining_produces_a_pasteable_line() {
        assert_eq!(
            shell_join(["docker", "run", "--entrypoint", "", "img", "a b"]),
            "docker run --entrypoint '' img 'a b'"
        );
    }
}
