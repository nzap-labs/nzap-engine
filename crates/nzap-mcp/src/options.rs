//! `nzap-engine mcp [options]`.

use std::path::{Path, PathBuf};

use crate::files::LocalFiles;

pub const USAGE: &str = "\
Usage: nzap-engine mcp [options]

Serve NZAP Engine to an AI agent over MCP (stdin/stdout). Connect Google once
in the NZAP Engine app first; the server uses the same connection.

Options:
  --allow-dir <folder>   Also share this folder (repeatable). The folder the
                         server starts in is always shared.
  --output-dir <folder>  Where results go by default (default: ./nzap-output).
  --max-runtimes <n>     Runtimes this agent may hold at once (default 2).
  --max-file-mb <n>      Largest file moved in either direction (default 500).
  --keep-runtimes        Do not release runtimes when the agent disconnects.
  -h, --help             Show this help.

Example (Claude Code):
  claude mcp add nzap -- nzap-engine mcp";

pub const DEFAULT_MAX_RUNTIMES: usize = 2;
pub const DEFAULT_MAX_FILE_MB: u64 = 500;

#[derive(Clone, Debug)]
pub struct Options {
    pub files: LocalFiles,
    /// Runtimes (including a running job's) this server may hold at once.
    pub max_runtimes: usize,
    /// Largest file uploaded or downloaded, in bytes.
    pub max_file_bytes: u64,
    /// Release the runtimes this server started when the agent goes away.
    pub release_on_exit: bool,
}

impl Options {
    /// Defaults for an agent started in `cwd`.
    pub fn new(cwd: &Path) -> Self {
        Self::parse(std::iter::empty::<String>(), cwd).unwrap_or_else(|_| unreachable!())
    }

    /// Parse the arguments after `mcp`. `Err` carries the message to print
    /// (the usage text for `--help`).
    pub fn parse<I, S>(args: I, cwd: &Path) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut allow = Vec::new();
        let mut output_dir = None;
        let mut max_runtimes = DEFAULT_MAX_RUNTIMES;
        let mut max_file_mb = DEFAULT_MAX_FILE_MB;
        let mut release_on_exit = true;
        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => {
                    (flag.to_owned(), Some(value.to_owned()))
                }
                _ => (arg.clone(), None),
            };
            let mut value = |name: &str| {
                inline
                    .clone()
                    .or_else(|| args.next())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("{name} needs a value.\n\n{USAGE}"))
            };
            match flag.as_str() {
                "--allow-dir" => allow.push(PathBuf::from(value("--allow-dir")?)),
                "--output-dir" => output_dir = Some(PathBuf::from(value("--output-dir")?)),
                "--max-runtimes" => {
                    max_runtimes = value("--max-runtimes")?
                        .parse()
                        .ok()
                        .filter(|count| *count >= 1)
                        .ok_or("--max-runtimes must be a whole number of at least 1.")?;
                }
                "--max-file-mb" => {
                    max_file_mb = value("--max-file-mb")?
                        .parse()
                        .ok()
                        .filter(|mb| *mb >= 1)
                        .ok_or("--max-file-mb must be a whole number of at least 1.")?;
                }
                "--keep-runtimes" => release_on_exit = false,
                "-h" | "--help" => return Err(USAGE.to_owned()),
                other => return Err(format!("Unknown option: {other}\n\n{USAGE}")),
            }
        }
        // The starting folder is shared unless it is a filesystem root (some
        // clients start servers in `/`).
        let mut roots: Vec<PathBuf> = Vec::new();
        if cwd.parent().is_some() {
            roots.push(cwd.to_path_buf());
        }
        roots.extend(allow);
        Ok(Self {
            files: LocalFiles::new(cwd, &roots, output_dir.as_deref()),
            max_runtimes,
            max_file_bytes: max_file_mb * 1024 * 1024,
            release_on_exit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_share_the_starting_folder() {
        let dir = tempfile::tempdir().unwrap();
        let options = Options::new(dir.path());
        assert_eq!(options.max_runtimes, DEFAULT_MAX_RUNTIMES);
        assert!(options.release_on_exit);
        assert_eq!(options.files.roots().len(), 1);
        assert!(options.files.output_dir().unwrap().ends_with("nzap-output"));
    }

    #[test]
    fn flags() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).unwrap();
        let options = Options::parse(
            [
                "--allow-dir",
                media.to_str().unwrap(),
                "--max-runtimes=3",
                "--max-file-mb",
                "50",
                "--keep-runtimes",
                "--output-dir",
                "results",
            ],
            dir.path(),
        )
        .unwrap();
        assert_eq!(options.max_runtimes, 3);
        assert_eq!(options.max_file_bytes, 50 * 1024 * 1024);
        assert!(!options.release_on_exit);
        assert_eq!(options.files.roots().len(), 3);
        assert!(options.files.output_dir().unwrap().ends_with("results"));
        assert!(dir.path().join("results").is_dir());
    }

    #[test]
    fn bad_flags_explain_themselves() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Options::parse(["--help"], dir.path()).unwrap_err().starts_with("Usage:"));
        assert!(Options::parse(["--frobnicate"], dir.path())
            .unwrap_err()
            .contains("Unknown option"));
        assert!(Options::parse(["--max-runtimes", "0"], dir.path()).is_err());
        assert!(Options::parse(["--allow-dir"], dir.path()).unwrap_err().contains("needs a value"));
    }

    #[test]
    fn a_filesystem_root_is_never_shared_by_default() {
        let options = Options::new(Path::new("/"));
        assert!(options.files.roots().is_empty());
    }
}
