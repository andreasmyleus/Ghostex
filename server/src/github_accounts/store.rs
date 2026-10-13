use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::gh::GITHUB_HOST;

/// `<gxserver data>/github-accounts`: the shims, one small file per worktree
/// naming its account, the owner → account choices, commit identities and the
/// SSH key found for each account. It never holds tokens.
pub(crate) fn root_dir() -> PathBuf {
    ghostex_paths::GhostexPaths::resolve()
        .gxserver_data_dir()
        .join("github-accounts")
}

fn owners_file() -> PathBuf {
    root_dir().join("owners.json")
}

fn worktrees_dir() -> PathBuf {
    root_dir().join("worktrees")
}

/// The file that names a worktree's account: the login, its host and the
/// worktree folder, one per line. Its path never changes for a worktree, so
/// sessions that point at it see a switch on their next gh, git or ssh call.
pub(crate) fn worktree_account_file(root: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let hash: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    worktrees_dir().join(format!("{}-{hash}", file_label(&canonical)))
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("worktree")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .take(48)
        .collect()
}

/// The git config file a worktree's sessions include (`include.path`): it
/// includes the commit identity of the worktree's current account.
pub(crate) fn worktree_identity_include(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("worktree");
    root_dir()
        .join("identity")
        .join("worktrees")
        .join(format!("{name}.gitconfig"))
}

/// The commit identity (`[user] name/email`) of one account.
pub(crate) fn account_identity_file(host: &str, login: &str) -> PathBuf {
    root_dir()
        .join("identity")
        .join("accounts")
        .join(host.to_ascii_lowercase())
        .join(format!("{login}.gitconfig"))
}

/// The file naming the SSH key (a private key, or a `.pub` the SSH agent
/// holds) that signs in as `login` on `host`. The SSH wrapper reads it.
pub(crate) fn ssh_key_file(host: &str, login: &str) -> PathBuf {
    root_dir()
        .join("ssh-keys")
        .join(host.to_ascii_lowercase())
        .join(login)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeRecord {
    pub(crate) login: String,
    pub(crate) host: String,
}

pub(crate) fn read_worktree_record(file: &Path) -> Option<WorktreeRecord> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut lines = text.lines().map(str::trim);
    let login = lines.next().filter(|login| !login.is_empty())?.to_string();
    let host = lines
        .next()
        .filter(|host| !host.is_empty())
        .unwrap_or(GITHUB_HOST)
        .to_ascii_lowercase();
    Some(WorktreeRecord { login, host })
}

pub(crate) fn read_worktree_login(file: &Path) -> Option<String> {
    read_worktree_record(file).map(|record| record.login)
}

/// Saves a worktree's account and points its identity include at that
/// account's commit identity.
pub(crate) fn write_worktree_login(
    file: &Path,
    host: &str,
    login: &str,
    root: &Path,
) -> io::Result<()> {
    write_atomically(
        file,
        format!(
            "{login}\n{}\n{}\n",
            host.to_ascii_lowercase(),
            root.to_string_lossy()
        )
        .as_bytes(),
    )?;
    let identity = account_identity_file(host, login)
        .to_string_lossy()
        .replace('\\', "/");
    write_atomically(
        &worktree_identity_include(file),
        format!("[include]\n\tpath = {}\n", git_config_quote(&identity)).as_bytes(),
    )
}

/// A git config value in double quotes, so paths with spaces or `#` survive.
pub(crate) fn git_config_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Removes the account files of worktrees whose folder no longer exists.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: Worktrees are deleted from the sidebar, the CLI or by hand, and only the last line of each account file knows its folder, so gxserver sweeps them on start instead of every delete path having to remember the account file.
pub(crate) fn prune_removed_worktrees() {
    let Ok(entries) = std::fs::read_dir(worktrees_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(root) = text
            .lines()
            .nth(2)
            .map(str::trim)
            .filter(|root| !root.is_empty())
        else {
            continue;
        };
        if !Path::new(root).exists() {
            let _ = std::fs::remove_file(worktree_identity_include(&path));
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn owner_key(host: &str, owner: &str) -> String {
    if host.eq_ignore_ascii_case(GITHUB_HOST) {
        owner.to_ascii_lowercase()
    } else {
        format!("{host}/{owner}").to_ascii_lowercase()
    }
}

/// The account remembered for each repository owner, keyed by lowercase
/// owner (`host/owner` on GitHub Enterprise hosts).
pub(crate) fn owner_logins() -> BTreeMap<String, String> {
    std::fs::read_to_string(owners_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn owner_login(host: &str, owner: &str) -> Option<String> {
    owner_logins().remove(&owner_key(host, owner))
}

pub(crate) fn set_owner_login(host: &str, owner: &str, login: &str) -> io::Result<()> {
    let mut owners = owner_logins();
    owners.insert(owner_key(host, owner), login.to_string());
    let text = serde_json::to_string_pretty(&owners).map_err(io::Error::other)?;
    write_atomically(&owners_file(), text.as_bytes())
}

pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("account file has no folder"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        std::process::id()
    ));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}
