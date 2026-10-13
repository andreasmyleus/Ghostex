use std::{path::PathBuf, process::Command, time::Duration};

use super::run::capture;

const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// The git checkout a folder belongs to and the GitHub repository its
/// `origin` points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoInfo {
    /// The worktree's top folder (`git rev-parse --show-toplevel`).
    pub(crate) root: PathBuf,
    pub(crate) owner: String,
    pub(crate) name: String,
}

/// `None` when `cwd` is not in a git checkout or `origin` is not on github.com.
pub(crate) fn repo_for_cwd(cwd: &str) -> Option<RepoInfo> {
    if cwd.trim().is_empty() || !std::path::Path::new(cwd).is_dir() {
        return None;
    }
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let url = git(cwd, &["config", "--get", "remote.origin.url"])?;
    let (owner, name) = parse_github_remote(&url)?;
    Some(RepoInfo {
        root: PathBuf::from(root),
        owner,
        name,
    })
}

fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    capture(command, GIT_TIMEOUT).filter(|value| !value.is_empty())
}

/// `(owner, repo)` of a github.com remote URL in any of git's spellings
/// (`https://github.com/o/r.git`, `git@github.com:o/r`, `ssh://git@github.com/o/r`).
/// SSH host aliases (`github-work:o/r`) return `None`: they already pick their
/// identity through the user's SSH config.
pub(crate) fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next().filter(|part| !part.is_empty())?;
    let name = parts.next().filter(|part| !part.is_empty())?;
    if parts.next().is_some() {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}
