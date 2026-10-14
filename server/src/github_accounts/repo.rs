use std::{path::PathBuf, process::Command, time::Duration};

use super::{gh, run::capture};

const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// A GitHub repository as a remote URL names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteRef {
    /// Lowercase host: `github.com` or a GitHub Enterprise host gh is signed in to.
    pub(crate) host: String,
    pub(crate) owner: String,
    pub(crate) name: String,
    /// The remote is reached over SSH (`git@host:o/r`, `ssh://…`).
    pub(crate) ssh: bool,
}

/// The git checkout a folder belongs to and the GitHub repository its remote
/// points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoInfo {
    /// The worktree's top folder (`git rev-parse --show-toplevel`).
    pub(crate) root: PathBuf,
    /// The main checkout this worktree was added from; `None` for the main
    /// checkout itself.
    pub(crate) main_root: Option<PathBuf>,
    pub(crate) remote: RemoteRef,
}

/// `None` when `cwd` is not in a git checkout or none of its remotes is on a
/// GitHub host (github.com, or an Enterprise host gh is signed in to). `origin`
/// wins; a checkout whose GitHub remote has another name (`github`,
/// `upstream`) uses the first such remote.
pub(crate) fn repo_for_cwd(cwd: &str) -> Option<RepoInfo> {
    if cwd.trim().is_empty() || !std::path::Path::new(cwd).is_dir() {
        return None;
    }
    let remote = git(cwd, &["config", "--get", "remote.origin.url"])
        .and_then(|url| parse_github_remote(&url))
        .or_else(|| {
            git(cwd, &["config", "--get-regexp", r"^remote\..*\.url$"])?
                .lines()
                .filter_map(|line| line.split_once(char::is_whitespace))
                .find_map(|(_, url)| parse_github_remote(url))
        })?;
    let dirs = git(cwd, &["rev-parse", "--show-toplevel", "--git-common-dir"])?;
    let mut lines = dirs.lines().map(str::trim);
    let root = PathBuf::from(lines.next().filter(|root| !root.is_empty())?);
    let main_root = lines
        .next()
        .map(|common| {
            let common = PathBuf::from(common);
            if common.is_absolute() {
                common
            } else {
                std::path::Path::new(cwd).join(common)
            }
        })
        .filter(|common| common.file_name().is_some_and(|name| name == ".git"))
        .and_then(|common| common.parent().map(std::path::Path::to_path_buf))
        .and_then(|main| std::fs::canonicalize(&main).ok())
        .filter(|main| std::fs::canonicalize(&root).map_or(true, |root| &root != main));
    Some(RepoInfo {
        root,
        main_root,
        remote,
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

/// The GitHub repository a remote URL names, in any of git's spellings
/// (`https://github.com/o/r.git`, `git@github.com:o/r`, `ssh://git@host/o/r`).
/// The host must be github.com or an Enterprise host in gh's `hosts.yml`; SSH
/// host aliases (`github-work:o/r`) return `None` because they already pick
/// their identity through the user's SSH config.
pub(crate) fn parse_github_remote(url: &str) -> Option<RemoteRef> {
    let url = url.trim();
    let (scheme, authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/')?;
            (Some(scheme.to_ascii_lowercase()), authority, path)
        }
        None => {
            let (authority, path) = url.split_once(':')?;
            (None, authority, path)
        }
    };
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() || !is_github_host(&host) {
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
    let ssh = match scheme.as_deref() {
        None => true,
        Some(scheme) => scheme == "ssh" || scheme == "git+ssh",
    };
    Some(RemoteRef {
        host,
        owner: owner.to_string(),
        name: name.to_string(),
        ssh,
    })
}

fn is_github_host(host: &str) -> bool {
    host == gh::GITHUB_HOST || gh::global_hosts().contains_key(host)
}
