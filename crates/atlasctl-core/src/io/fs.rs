// SPDX-License-Identifier: AGPL-3.0-only

//! Filesystem access.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Reads and writes files.
pub trait FileSystem: Send + Sync {
    /// Read a file as text.
    fn read_to_string(&self, path: &Path) -> Result<String>;

    /// Write a file, creating parent directories as needed.
    ///
    /// Writes are atomic: content goes to a temporary file in the same
    /// directory and is renamed into place, so an interrupted write cannot
    /// leave a half-written config behind.
    fn write_atomic(&self, path: &Path, contents: &str) -> Result<()>;

    /// Whether a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Create a directory and its parents.
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// List files directly under `dir` with the given extension, sorted.
    fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>>;
}

/// The real implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        std::fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
    }

    fn write_atomic(&self, path: &Path, contents: &str) -> Result<()> {
        let parent = path.parent().context("path has no parent directory")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
        let tmp = parent.join(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("atlasctl")
        ));
        std::fs::write(&tmp, contents)
            .with_context(|| format!("could not write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("could not place {}", path.display()))?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)
            .with_context(|| format!("could not create {}", path.display()))
    }

    fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some(extension) {
                out.push(p);
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(any(test, feature = "test-mocks"))]
mod mock {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// An in-memory filesystem that records what was written.
    #[derive(Debug, Default)]
    pub struct MemFileSystem {
        files: Mutex<BTreeMap<PathBuf, String>>,
        dirs: Mutex<Vec<PathBuf>>,
    }

    impl MemFileSystem {
        /// An empty filesystem.
        pub fn new() -> Self {
            Self::default()
        }

        /// Seed a file.
        pub fn insert(&self, path: impl Into<PathBuf>, contents: impl Into<String>) {
            self.files
                .lock()
                .expect("lock")
                .insert(path.into(), contents.into());
        }

        /// Read back what a test wrote.
        pub fn get(&self, path: impl AsRef<Path>) -> Option<String> {
            self.files.lock().expect("lock").get(path.as_ref()).cloned()
        }

        /// Directories that were created.
        pub fn created_dirs(&self) -> Vec<PathBuf> {
            self.dirs.lock().expect("lock").clone()
        }
    }

    impl FileSystem for MemFileSystem {
        fn read_to_string(&self, path: &Path) -> Result<String> {
            self.files
                .lock()
                .expect("lock")
                .get(path)
                .cloned()
                .with_context(|| format!("could not read {}", path.display()))
        }

        fn write_atomic(&self, path: &Path, contents: &str) -> Result<()> {
            self.files
                .lock()
                .expect("lock")
                .insert(path.to_path_buf(), contents.to_string());
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            self.files.lock().expect("lock").contains_key(path)
        }

        fn create_dir_all(&self, path: &Path) -> Result<()> {
            self.dirs.lock().expect("lock").push(path.to_path_buf());
            Ok(())
        }

        fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
            let files = self.files.lock().expect("lock");
            let mut out: Vec<PathBuf> = files
                .keys()
                .filter(|p| p.parent() == Some(dir))
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(extension))
                .cloned()
                .collect();
            out.sort();
            Ok(out)
        }
    }
}

#[cfg(any(test, feature = "test-mocks"))]
pub use mock::MemFileSystem;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mock_round_trips_a_write() {
        let fs = MemFileSystem::new();
        fs.write_atomic(Path::new("/cfg/registries.yaml"), "body")
            .unwrap();
        assert_eq!(fs.get("/cfg/registries.yaml").as_deref(), Some("body"));
        assert!(fs.exists(Path::new("/cfg/registries.yaml")));
    }

    #[test]
    fn reading_an_absent_file_is_an_error_naming_it() {
        let err = MemFileSystem::new()
            .read_to_string(Path::new("/nope.yaml"))
            .expect_err("must fail");
        assert!(
            err.to_string().contains("/nope.yaml"),
            "error should name the path: {err}"
        );
    }

    #[test]
    fn listing_filters_by_extension_and_directory() {
        let fs = MemFileSystem::new();
        fs.insert("/r/a.yaml", "");
        fs.insert("/r/b.yaml", "");
        fs.insert("/r/c.txt", "");
        fs.insert("/other/d.yaml", "");
        let found = fs.list_files(Path::new("/r"), "yaml").unwrap();
        assert_eq!(
            found,
            [PathBuf::from("/r/a.yaml"), PathBuf::from("/r/b.yaml")]
        );
    }
}
