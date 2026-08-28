// SPDX-License-Identifier: AGPL-3.0-only

//! A snapshot of the facts about the host that a launch depends on.

use std::collections::BTreeMap;

/// A host's POSIX identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosixUser {
    /// The uid the container should run as.
    pub uid: u32,
    /// The gid the container should run as.
    pub gid: u32,
}

/// Everything `translate` needs to know about the machine, captured once.
///
/// Taking a snapshot rather than querying the OS mid-translation is what keeps
/// translation pure and therefore testable: the same snapshot always produces
/// the same command, on any machine, with no GPU and no docker present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSnapshot {
    /// The POSIX user the container should run as, when the host has one.
    ///
    /// `None` is an answer, not a gap: a Windows account has no uid, so there
    /// is no correct number to put here. Rendering `--user 0:0` for it would
    /// run the container as root, and the `/etc/passwd` mounts that accompany
    /// `--user` name host paths that do not exist there.
    pub posix_user: Option<PosixUser>,
    /// The user's home directory, used to render a portable command.
    pub home: String,
    /// Where HuggingFace caches models on this host.
    pub hf_cache_dir: String,
    /// Host environment, for expanding `$VAR` in a recipe's `env:` block.
    pub env: BTreeMap<String, String>,
}

impl HostSnapshot {
    /// Expand `$VAR` and `${VAR}` against this snapshot's environment.
    ///
    /// An unset variable expands to the empty string, matching the shell and
    /// the reference implementation. It is deliberately not an error: recipes
    /// use this for optional passthrough, and failing a launch over an unset
    /// optional would be worse than passing an empty value.
    pub fn expand(&self, raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            let braced = chars.peek() == Some(&'{');
            if braced {
                chars.next();
            }
            let mut name = String::new();
            while let Some(&n) = chars.peek() {
                let ok = if braced {
                    n != '}'
                } else {
                    n.is_alphanumeric() || n == '_'
                };
                if !ok {
                    break;
                }
                name.push(n);
                chars.next();
            }
            if braced {
                chars.next(); // consume '}'
            }
            if name.is_empty() {
                out.push('$');
            } else {
                out.push_str(self.env.get(&name).map(String::as_str).unwrap_or(""));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostSnapshot {
        HostSnapshot {
            posix_user: Some(PosixUser {
                uid: 1000,
                gid: 1000,
            }),
            home: "/home/spark".into(),
            hf_cache_dir: "/home/spark/.cache/huggingface".into(),
            env: [("TOKEN".to_string(), "abc".to_string())].into(),
        }
    }

    #[test]
    fn a_set_variable_expands_in_both_spellings() {
        assert_eq!(host().expand("$TOKEN"), "abc");
        assert_eq!(host().expand("${TOKEN}"), "abc");
        assert_eq!(host().expand("pre-$TOKEN-post"), "pre-abc-post");
    }

    #[test]
    fn an_unset_variable_expands_to_nothing_rather_than_failing() {
        assert_eq!(host().expand("$NOPE"), "");
        assert_eq!(host().expand("a${NOPE}b"), "ab");
    }

    #[test]
    fn text_without_variables_is_untouched() {
        assert_eq!(host().expand("plain value"), "plain value");
        assert_eq!(host().expand(""), "");
    }

    #[test]
    fn a_lone_dollar_is_literal() {
        assert_eq!(host().expand("100$"), "100$");
        assert_eq!(host().expand("$ x"), "$ x");
    }
}
