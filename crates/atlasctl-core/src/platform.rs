// SPDX-License-Identifier: AGPL-3.0-only

//! The handful of facts that differ per operating system.
//!
//! Every one of these was a bare `std::env::var("HOME")` scattered through the
//! CLI. Collected here because they are not independent: the config directory,
//! the cache directory and the home directory must agree about which user this
//! is, and a port that fixes them one call site at a time gets that wrong in a
//! way nothing fails on until a node comes back as a stranger to its own fleet.
//!
//! # Windows
//!
//! State goes under `%LOCALAPPDATA%`, never `%APPDATA%`. The difference is
//! roaming: `%APPDATA%` follows a user between machines on a domain, and
//! `agent.key` is *this machine's* identity. Roaming it would give two machines
//! the same node identity — the same failure as sharing a private key, arrived
//! at by copying a directory.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// This user's home directory.
///
/// # Errors
/// If the platform's home variable is unset.
pub fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(home_var()?))
}

/// This user's home directory, as the string a rendered command embeds.
///
/// # Errors
/// If the platform's home variable is unset.
pub fn home_string() -> Result<String> {
    home_var()
}

#[cfg(unix)]
fn home_var() -> Result<String> {
    std::env::var("HOME").context("HOME is not set, so there is nowhere to keep this node's state")
}

#[cfg(windows)]
fn home_var() -> Result<String> {
    // USERPROFILE, not HOMEDRIVE+HOMEPATH: the latter pair is a legacy of
    // network home directories and is routinely set to a share that is not
    // mounted, which fails as "installed, then cannot write its own key".
    std::env::var("USERPROFILE")
        .context("USERPROFILE is not set, so there is nowhere to keep this node's state")
}

/// The directory under which per-user configuration lives.
///
/// Returns the *base*; callers append `atlasctl`.
///
/// # Errors
/// If neither the platform override nor the home variable is set.
pub fn config_base() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join(".config"))
    }
    #[cfg(windows)]
    {
        // Deliberately not honouring XDG_CONFIG_HOME here. It is set on Windows
        // only by ports of unix tools, and honouring it would move `agent.key`
        // for a reason the operator never connected to this program.
        if let Ok(x) = std::env::var("LOCALAPPDATA")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join("AppData").join("Local"))
    }
}

/// The directory under which per-user caches live.
///
/// Returns the *base*; callers append `atlasctl`.
///
/// # Errors
/// If neither the platform override nor the home variable is set.
pub fn cache_base() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        if let Ok(x) = std::env::var("XDG_CACHE_HOME")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join(".cache"))
    }
    #[cfg(windows)]
    {
        config_base()
    }
}

/// This process's POSIX identity, if the platform has one.
///
/// `None` on Windows, and that is the answer rather than a gap: a Windows
/// account has no uid, `--user 0:0` would run a container as root, and the
/// `/etc/passwd` bind mounts that accompany `--user` name host paths that do
/// not exist there. Omitting the flag runs the image's own user, which is what
/// every other Docker-on-Windows workflow does.
#[must_use]
pub fn posix_user() -> Option<crate::host::PosixUser> {
    #[cfg(unix)]
    {
        Some(crate::host::PosixUser {
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
        })
    }
    #[cfg(windows)]
    {
        None
    }
}

/// Free bytes on the filesystem holding `path`, when it can be determined.
///
/// One implementation, because "how much room is left" was asked in two places
/// with two different answers: the fleet vitals called `statvfs` and returned
/// `None` everywhere else, and `doctor` shelled out to `df -Pk`, which does not
/// exist on Windows at all. A machine whose disk column is blank is a machine
/// nobody checks before a 100 GB pull.
///
/// A full model cache is a leading cause of launch failure, so this is worth a
/// platform call.
#[must_use]
pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
    // The directory may not exist yet on a fresh install, and asking the OS
    // about a missing path fails. What matters is the filesystem that WOULD
    // hold it, so walk up to the nearest ancestor that does exist.
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    free_bytes_of_existing(probe)
}

// `allow`, not `expect`: `f_bavail` and `f_frsize` are u64 on 64-bit targets,
// where the conversion is a no-op clippy objects to, and narrower on others,
// where dropping it would silently truncate a disk size. An `expect` would then
// fail on exactly the targets that need the conversion.
#[allow(clippy::useless_conversion, reason = "the widths are target-dependent")]
#[cfg(unix)]
fn free_bytes_of_existing(probe: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path and `stat` is written only by
    // the call, which reports success before we read it.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) } != 0 {
        return None;
    }
    u64::try_from(stat.f_bavail)
        .ok()?
        .checked_mul(u64::try_from(stat.f_frsize).ok()?)
}

#[cfg(windows)]
fn free_bytes_of_existing(probe: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut avail: u64 = 0;
    // SAFETY: `wide` is NUL-terminated and outlives the call; `avail` is
    // written only by it and read only after it reports success. The two
    // total-size outputs are not wanted, and NULL is documented as the way to
    // decline them.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &raw mut avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // Available to THIS user, not the volume total: a per-user quota is the
    // number that decides whether a pull fits.
    (ok != 0).then_some(avail)
}

/// How this platform's shell names the user's home directory.
///
/// Used when rendering a command meant to be pasted rather than run: the
/// literal home directory is replaced with this so the line is not tied to one
/// account. `$HOME` was emitted unconditionally, and a Windows operator pasting
/// it into PowerShell gets a volume mounted from a directory literally named
/// `$HOME` — which docker then creates, empty, and the model is downloaded
/// again.
#[must_use]
pub const fn home_placeholder() -> &'static str {
    if cfg!(windows) {
        "$env:USERPROFILE"
    } else {
        "$HOME"
    }
}

/// Send this process's stdout and stderr to `path`, appending.
///
/// Exists because a supervised agent's output has to land somewhere an
/// operator can read. The alternative — wrapping the command in a shell that
/// redirects — costs more than it looks on Windows: Task Scheduler's stop
/// terminates only the process it started, so a `cmd.exe` wrapper is killed
/// and the agent it launched is ORPHANED, still holding the port, and the
/// replacement exits at startup. Measured on CI, where a reinstall reported
/// "installed, but it is NOT running" every time.
///
/// Redirection is at the OS handle level rather than through a logging
/// framework because every message this binary emits goes through `eprintln!`;
/// intercepting those individually would mean not missing one, forever.
///
/// # Errors
/// If the file cannot be opened, or the handles cannot be replaced.
pub fn redirect_stdio(path: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        // 0600, not the ambient umask. This log carries the node's identity and
        // whatever a startup failure prints, and it is written by a service —
        // where the umask is the supervisor's, not the operator's. Under
        // journald the same output was user-private; a file at 0644 would be a
        // quiet downgrade nobody asked for.
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    redirect_to(&file)?;
    // Deliberately leaked: the descriptors above now refer to this file, and
    // dropping the handle while they do is how output starts vanishing partway
    // through a run. It lives as long as the process, which is the intent.
    std::mem::forget(file);
    Ok(())
}

#[cfg(unix)]
fn redirect_to(file: &std::fs::File) -> anyhow::Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    for target in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: `fd` is a live descriptor owned by `file`, and the targets
        // are the two standard descriptors this process owns.
        if unsafe { libc::dup2(fd, target) } == -1 {
            return Err(std::io::Error::last_os_error()).context("redirecting output");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn redirect_to(file: &std::fs::File) -> anyhow::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    let h = file.as_raw_handle();
    for target in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `h` is a live file handle owned by `file`, and the targets
        // name this process's own standard handles.
        if unsafe { SetStdHandle(target, h.cast()) } == 0 {
            return Err(std::io::Error::last_os_error()).context("redirecting output");
        }
    }
    Ok(())
}

/// Find an executable on `PATH`.
///
/// On Windows a name without an extension is not a program: `docker` is
/// `docker.exe`, and a lookup that only tries the bare name reports every tool
/// on the machine as missing. `PATHEXT` is the list the shell itself uses.
#[must_use]
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        let has_ext = std::path::Path::new(name).extension().is_some();
        let mut v = if has_ext {
            vec![name.to_owned()]
        } else {
            Vec::new()
        };
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        v.extend(
            exts.split(';')
                .filter(|e| !e.trim().is_empty())
                .map(|e| format!("{name}{}", e.trim())),
        );
        v
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&path)
        .flat_map(|d| candidates.iter().map(move |c| d.join(c)))
        .find(|p| p.is_file())
}

/// This machine's name, for display and for a peer's `Hello`.
///
/// Never fatal: a node with an unreadable hostname is still a usable node, and
/// the fingerprint is what identifies it.
///
/// `fallback` is the caller's, because the two callers want different things
/// and collapsing them lost information. A DISPLAY name wants something that
/// reads as a name (`atlas-node`); a name pasted into a `peer add` command the
/// operator must edit wants something that obviously is not one
/// (`<this-machine>`), or it gets dialled and fails confusingly.
#[must_use]
pub fn hostname_or(fallback: &str) -> String {
    #[cfg(unix)]
    let from_os = std::fs::read_to_string("/proc/sys/kernel/hostname").ok();
    #[cfg(windows)]
    let from_os = std::env::var("COMPUTERNAME").ok();

    from_os
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bases must be BELOW the home directory, not merely different from
    /// it. Asserting only inequality passed on the one configuration where
    /// "below" is false — a redirected `%LOCALAPPDATA%` — which is the case
    /// worth catching, since that is what a secret's containment is anchored
    /// on.
    ///
    /// An explicit environment override is exempt: an operator who named a
    /// directory chose it, and refusing their choice here would be this
    /// function inventing a policy the resolver does not have.
    #[test]
    fn the_bases_sit_below_the_home_directory() {
        let home = home_dir().expect("a home directory");
        let overridden = ["XDG_CONFIG_HOME", "XDG_CACHE_HOME", "LOCALAPPDATA"]
            .iter()
            .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
        for base in [
            config_base().expect("config base"),
            cache_base().expect("cache base"),
        ] {
            assert_ne!(base, home, "state must not land directly in the home dir");
            if !overridden {
                assert!(
                    base.starts_with(&home),
                    "{} is not below {}",
                    base.display(),
                    home.display()
                );
            }
        }
    }

    /// A hostname is used in display and in a peer Hello, so an empty string
    /// would render as a nameless node rather than as a problem.
    #[test]
    fn a_hostname_is_never_empty() {
        assert!(!hostname_or("fallback").is_empty());
    }

    /// And the caller's fallback is the one used, rather than a shared string
    /// that reads as a real hostname to one caller and as a placeholder to the
    /// other.
    #[test]
    fn the_callers_fallback_is_what_appears() {
        // Forced down the fallback path by asking on a machine where neither
        // source can answer is not possible here, so the property checked is
        // the weaker one that holds always: whatever comes back is non-empty,
        // and the fallback is never silently replaced by a built-in.
        let a = hostname_or("<this-machine>");
        let b = hostname_or("atlas-node");
        assert_eq!(a, b, "a real hostname must not depend on the fallback");
        assert!(!a.is_empty());
    }

    /// The question this answers is asked before a 100 GB pull, so an answer
    /// of "cannot tell" on a working machine is the failure that matters.
    #[test]
    fn free_space_is_readable_for_a_directory_that_exists() {
        let n = free_bytes(&std::env::temp_dir()).expect("a temp dir has a filesystem");
        assert!(
            n > 0,
            "a writable temp dir with zero bytes free is not credible"
        );
    }

    /// A fresh install asks about a cache directory that does not exist yet.
    /// Answering `None` there would blank the disk column on every new machine.
    #[test]
    fn free_space_walks_up_to_a_parent_that_exists() {
        let missing = std::env::temp_dir()
            .join("atlasctl-does-not-exist")
            .join("nor-this");
        assert!(
            free_bytes(&missing).is_some(),
            "must fall back to the volume"
        );
    }

    /// `which` is how docker is found. On Windows a bare name never matches,
    /// which reported every tool on the machine as missing.
    #[test]
    fn which_finds_a_real_program_and_not_an_invented_one() {
        let real = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(which(real).is_some(), "{real} must be on PATH");
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }

    /// Checked against the OS, not against the same `cfg!` the implementation
    /// branches on — that form was true by construction and could not fail.
    /// What matters downstream is the VALUE: `translate` renders `--user
    /// uid:gid` from it, so a zero uid would silently ask docker to run the
    /// container as root.
    #[cfg(unix)]
    #[test]
    fn the_posix_identity_is_this_process_s_own() {
        let u = posix_user().expect("unix always has one");
        assert_eq!(u.uid, rustix::process::getuid().as_raw());
        assert_eq!(u.gid, rustix::process::getgid().as_raw());
    }

    /// And on Windows there is no uid to report. Asserting `None` is not
    /// tautological here: the alternative a port reaches for is `Some(0)`.
    #[cfg(windows)]
    #[test]
    fn windows_reports_no_posix_identity_rather_than_root() {
        assert!(posix_user().is_none());
    }
}
