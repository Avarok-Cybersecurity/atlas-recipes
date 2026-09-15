// SPDX-License-Identifier: AGPL-3.0-only

//! The server a gate run may leave running on this node for the next one.
//!
//! With `serve_reuse: true` the node's child runs as `spark benchmark run
//! … --serve-reuse --serve-lease-owner <this agent's pid>`: instead of
//! loading the checkpoint in its own process it takes the LEASED server —
//! one an earlier run started, described in `<atlas_home>/serve-lease.json`
//! — when it is provably the server it would have started (same binary,
//! same recipe rendering; Atlas verifies over `GET /serve-config`), replaces
//! it otherwise, and leaves it up. The agent's part is small: know that the
//! leased server is its own and not a foreign tenant (`exclusive`), and
//! stop it when nothing has needed it for a while, or at shutdown. The
//! file's shape is Atlas's (`spark-server/src/cli/bench_lease.rs`); only
//! the fields the agent reads are named here.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the agent needs from the lease.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Lease {
    pub pid: u32,
    pub port: u16,
    pub model: String,
    pub owner_pid: u32,
}

pub fn path(atlas_home: &Path) -> PathBuf {
    atlas_home.join("serve-lease.json")
}

/// The lease on file, if it parses. A file that does not is treated as
/// absent here: the agent never signals a pid it cannot read.
pub fn read(atlas_home: &Path) -> Option<Lease> {
    let text = std::fs::read_to_string(path(atlas_home)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The leased server THIS agent owns and that is alive, if any.
pub fn ours(atlas_home: &Path) -> Option<Lease> {
    read(atlas_home).filter(|l| l.owner_pid == std::process::id() && alive(l.pid))
}

pub fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// SIGTERM the server's process group, wait up to `grace`, SIGKILL what is
/// left, and forget the lease. Only ever called on a lease this agent owns.
pub fn release(atlas_home: &Path, lease: &Lease, grace: Duration) {
    let pid = lease.pid;
    let _ = std::process::Command::new("kill")
        .args(["-TERM", "--", &format!("-{pid}")])
        .status();
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let until = std::time::Instant::now() + grace;
    while alive(pid) && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
    }
    if alive(pid) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status();
    }
    let _ = std::fs::remove_file(path(atlas_home));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agent-lease-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(home: &Path, pid: u32, owner: u32) {
        std::fs::write(
            path(home),
            format!(
                r#"{{"pid":{pid},"port":40001,"model":"m","recipe_id":"r","argv_sha256":"a","binary_sha256":"b","owner_pid":{owner},"started_at":0}}"#
            ),
        )
        .unwrap();
    }

    /// Only a lease this agent owns, naming a live pid, is "ours": another
    /// owner's is a foreign tenant, and a dead pid is nothing.
    #[test]
    fn ours_needs_this_owner_and_a_live_pid() {
        let h = home("ours");
        assert_eq!(ours(&h), None);
        // pid 1 is always alive.
        write(&h, 1, std::process::id());
        assert_eq!(ours(&h).map(|l| l.pid), Some(1));
        write(&h, 1, 4_000_000_000 - 3);
        assert_eq!(ours(&h), None, "another owner's lease is not ours");
        write(&h, 4_000_000_000 - 5, std::process::id());
        assert_eq!(ours(&h), None, "a dead server is not a lease");
        std::fs::write(path(&h), "junk").unwrap();
        assert_eq!(read(&h), None);
        let _ = std::fs::remove_dir_all(&h);
    }
}
