// SPDX-License-Identifier: AGPL-3.0-only

//! The pairing token.
//!
//! The site is static and served from a different origin, so it cannot ship a
//! per-install secret. The user pastes one in once, from a terminal on the
//! machine that owns the agent, which is what makes "this browser was
//! deliberately connected to this agent" a statement anyone can rely on.
//!
//! What this defends: a page that somehow passes the origin check, and any
//! other *user* on a shared machine, since loopback is reachable by every local
//! account. What it does not defend: a process running as the same user, which
//! can read the token file. That boundary is stated plainly in the docs rather
//! than implied away — no transport design changes it, because such a process
//! can run `docker` directly.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;

/// Bytes of entropy in a token.
const TOKEN_BYTES: usize = 32;

/// Where the token lives, relative to the config directory.
pub const TOKEN_FILE: &str = "browser.token";

/// Render bytes as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Generate a fresh token from the operating system's CSPRNG.
///
/// Through `getrandom`, like the other three places this codebase draws
/// entropy — including the identity key seed, which matters more than this
/// does. Reading `/dev/urandom` by hand was the odd one out, and it failed on
/// Windows with `The system cannot find the path specified`, which is to say
/// no browser token could ever be minted there.
pub fn generate() -> Result<String> {
    let mut buf = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut buf).context("reading from the system CSPRNG")?;
    Ok(hex(&buf))
}

/// Path of the token file within a config directory.
pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(TOKEN_FILE)
}

/// Load the token, creating one on first use.
///
/// The file is written `0600`. A wider mode is refused rather than silently
/// tightened: if another account could already read it, the token should be
/// replaced, and only the user can decide that.
pub fn load_or_create(config_dir: &Path) -> Result<String> {
    let p = path(config_dir);
    if p.exists() {
        let token = std::fs::read_to_string(&p)
            .with_context(|| format!("reading {}", p.display()))?
            .trim()
            .to_string();
        atlasctl_core::secretfile::verify(&p)?;
        if token.len() != TOKEN_BYTES * 2 {
            bail!(
                "{} does not contain a well-formed token; delete it and run \
                 `atlasctl agent token --rotate`",
                p.display()
            );
        }
        return Ok(token);
    }

    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("creating {}", config_dir.display()))?;
    let token = generate()?;
    atlasctl_core::secretfile::write(&p, token.as_bytes())?;
    Ok(token)
}

/// Replace the token.
pub fn rotate(config_dir: &Path) -> Result<String> {
    let token = generate()?;
    std::fs::create_dir_all(config_dir)?;
    atlasctl_core::secretfile::write(&path(config_dir), token.as_bytes())?;
    Ok(token)
}

/// Compare a presented token against the real one, in constant time.
///
/// A byte-by-byte comparison leaks how much of a guess was correct, which turns
/// a 256-bit secret into a series of 8-bit ones.
pub fn matches(expected: &str, presented: &str) -> bool {
    let a = expected.as_bytes();
    let b = presented.as_bytes();
    if a.len() != b.len() {
        // Length alone is not secret: the format is fixed and public.
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_token_has_the_expected_shape_and_is_not_repeated() {
        let a = generate().expect("generates");
        let b = generate().expect("generates");
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two tokens must not be identical");
    }

    #[test]
    fn comparison_accepts_only_the_exact_token() {
        let t = generate().unwrap();
        assert!(matches(&t, &t));
        assert!(!matches(&t, ""));
        assert!(!matches(&t, &t[..t.len() - 1]));
        let mut wrong = t.clone();
        wrong.replace_range(0..1, if t.starts_with('a') { "b" } else { "a" });
        assert!(!matches(&t, &wrong), "a single differing byte must fail");
    }

    #[test]
    fn a_token_survives_a_round_trip_through_a_directory() {
        let dir = std::env::temp_dir().join(format!("atlasctl-tok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = load_or_create(&dir).expect("creates");
        let second = load_or_create(&dir).expect("loads");
        assert_eq!(first, second, "loading must not mint a new token");
        let rotated = rotate(&dir).expect("rotates");
        assert_ne!(rotated, first, "rotation must change the token");
        assert_eq!(load_or_create(&dir).unwrap(), rotated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_created_private_and_a_loose_one_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("atlasctl-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        load_or_create(&dir).expect("creates");
        let p = path(&dir);
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token must be created private, got {mode:o}");

        // Widening it must be reported, not quietly accepted.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_or_create(&dir).expect_err("a readable token must be refused");
        assert!(
            err.to_string().contains("readable by other accounts"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
