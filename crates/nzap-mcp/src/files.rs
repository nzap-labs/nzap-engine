//! Which local files an agent may hand to NZAP Engine or receive from it.
//!
//! The server reads and writes only inside its shared folders: the folder
//! the agent started it in (a project, for Claude Code) plus any
//! `--allow-dir`. Paths are checked lexically and again after resolving
//! symlinks, so `..` and links cannot reach outside.

use std::path::{Component, Path, PathBuf};

use nzap_core::{Error, Result};

#[derive(Clone, Debug)]
struct Root {
    /// As configured (absolute, normalized).
    given: PathBuf,
    /// With symlinks resolved (`/tmp` → `/private/tmp` on macOS).
    canonical: PathBuf,
}

#[derive(Clone, Debug)]
pub struct LocalFiles {
    /// Relative paths resolve against this folder.
    base: PathBuf,
    roots: Vec<Root>,
    output_dir: Option<PathBuf>,
}

/// `path` made absolute against `base`, with `.` and `..` resolved lexically.
fn normalize(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() { path.to_path_buf() } else { base.join(path) };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn not_shared(path: &Path) -> Error {
    Error::invalid(format!(
        "{} is outside the folders shared with NZAP Engine. Use a path inside the project, \
         or add --allow-dir <folder> to the server's arguments.",
        path.display()
    ))
}

impl LocalFiles {
    /// `roots` that do not exist are skipped. `output_dir`, when given, is
    /// created and shared too.
    pub fn new(base: &Path, roots: &[PathBuf], output_dir: Option<&Path>) -> Self {
        let base = normalize(Path::new("/"), base);
        let output_dir = output_dir.map(|dir| normalize(&base, dir));
        if let Some(dir) = &output_dir {
            if let Err(error) = std::fs::create_dir_all(dir) {
                tracing::warn!("Cannot create the output folder {}: {error}", dir.display());
            }
        }
        let roots = roots
            .iter()
            .map(|root| normalize(&base, root))
            .chain(output_dir.clone())
            .filter_map(|given| match dunce::canonicalize(&given) {
                Ok(canonical) => Some(Root { given, canonical }),
                Err(error) => {
                    tracing::warn!("Not sharing {}: {error}", given.display());
                    None
                }
            })
            .collect::<Vec<_>>();
        let output_dir =
            output_dir.or_else(|| roots.first().map(|root| root.given.join("nzap-output")));
        Self { base, roots, output_dir }
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|root| root.given.clone()).collect()
    }

    /// Where results go when the agent names no folder.
    pub fn output_dir(&self) -> Option<&Path> {
        self.output_dir.as_deref()
    }

    fn inside_given(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| path.starts_with(&root.given) || path.starts_with(&root.canonical))
    }

    fn inside_canonical(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(&root.canonical))
    }

    fn require_roots(&self) -> Result<()> {
        if self.roots.is_empty() {
            Err(Error::invalid(
                "No local folder is shared with NZAP Engine. Start the server from a project \
                 folder, or add --allow-dir <folder> to its arguments.",
            ))
        } else {
            Ok(())
        }
    }

    /// An existing file the agent may read.
    pub fn readable(&self, raw: &str) -> Result<PathBuf> {
        self.require_roots()?;
        let path = normalize(&self.base, Path::new(raw.trim()));
        let canonical = dunce::canonicalize(&path)
            .map_err(|_| Error::not_found(format!("No such file: {}", path.display())))?;
        if !self.inside_canonical(&canonical) {
            return Err(not_shared(&path));
        }
        if !canonical.is_file() {
            return Err(Error::invalid(format!("{} is not a file.", path.display())));
        }
        Ok(canonical)
    }

    /// A file path the agent may write (its folder is created).
    pub fn writable(&self, raw: &Path) -> Result<PathBuf> {
        self.require_roots()?;
        let path = normalize(&self.base, raw);
        if !self.inside_given(&path) {
            return Err(not_shared(&path));
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Error::invalid(format!("{} is not a file path.", path.display())));
        };
        std::fs::create_dir_all(parent)?;
        let parent = dunce::canonicalize(parent)?;
        if !self.inside_canonical(&parent) {
            return Err(not_shared(&path));
        }
        let target = parent.join(name);
        if std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(Error::invalid(format!("{} is a symbolic link.", target.display())));
        }
        Ok(target)
    }

    /// A folder for results: the agent's choice, or `<output>/<default>`.
    pub fn folder(&self, raw: Option<&str>, default: &str) -> Result<PathBuf> {
        self.require_roots()?;
        let path = match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
            Some(raw) => normalize(&self.base, Path::new(raw)),
            None => match &self.output_dir {
                Some(dir) => dir.join(default),
                None => return Err(Error::invalid("No output folder is configured.")),
            },
        };
        if !self.inside_given(&path) {
            return Err(not_shared(&path));
        }
        std::fs::create_dir_all(&path)?;
        let canonical = dunce::canonicalize(&path)?;
        if !self.inside_canonical(&canonical) {
            return Err(not_shared(&path));
        }
        Ok(canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> (tempfile::TempDir, LocalFiles) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("clip.mp4"), b"video").unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"no").unwrap();
        let files = LocalFiles::new(&project, std::slice::from_ref(&project), None);
        (dir, files)
    }

    #[test]
    fn normalizes_lexically() {
        let base = Path::new("/work/project");
        assert_eq!(
            normalize(base, Path::new("a/./b/../c.txt")),
            PathBuf::from("/work/project/a/c.txt")
        );
        assert_eq!(normalize(base, Path::new("/etc/../tmp")), PathBuf::from("/tmp"));
        assert_eq!(normalize(base, Path::new("../../..")), PathBuf::from("/"));
    }

    #[test]
    fn reads_only_inside_the_shared_folders() {
        let (dir, files) = shared();
        assert!(files.readable("clip.mp4").unwrap().ends_with("project/clip.mp4"));
        assert!(files.readable("../secret.txt").is_err());
        assert!(files.readable(dir.path().join("secret.txt").to_str().unwrap()).is_err());
        assert!(files.readable("missing.wav").is_err());
        assert!(files.readable(".").is_err(), "a folder is not a file");
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_escape() {
        let (dir, files) = shared();
        let project = dir.path().join("project");
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), project.join("link.txt"))
            .unwrap();
        std::os::unix::fs::symlink(dir.path(), project.join("up")).unwrap();
        assert!(files.readable("link.txt").is_err());
        assert!(files.readable("up/secret.txt").is_err());
        assert!(files.writable(Path::new("up/out.txt")).is_err());
        assert!(files.writable(Path::new("link.txt")).is_err());
    }

    #[test]
    fn writes_create_folders_inside_only() {
        let (_dir, files) = shared();
        let target = files.writable(Path::new("out/audio/a.wav")).unwrap();
        assert!(target.parent().unwrap().is_dir());
        assert!(files.writable(Path::new("../escape.wav")).is_err());
        let folder = files.folder(None, "jobs/run-1").unwrap();
        assert!(folder.ends_with("project/nzap-output/jobs/run-1"));
        assert!(files.folder(Some("/"), "x").is_err());
    }

    #[test]
    fn nothing_is_shared_without_a_root() {
        let files = LocalFiles::new(Path::new("/"), &[], None);
        assert!(files.readable("/etc/hosts").is_err());
        assert!(files.folder(None, "x").is_err());
        assert!(files.output_dir().is_none());
    }
}
