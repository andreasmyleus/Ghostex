use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// `<gxserver data>/github-accounts`: the shim, one small file per worktree
/// naming its account, and the owner → account choices. It never holds tokens.
pub(super) fn root_dir() -> PathBuf {
    ghostex_paths::GhostexPaths::resolve()
        .gxserver_data_dir()
        .join("github-accounts")
}

fn owners_file() -> PathBuf {
    root_dir().join("owners.json")
}

/// The file that names a worktree's account. Its path never changes for a
/// worktree, so sessions that point at it see a switch on their next gh call.
pub(crate) fn worktree_account_file(root: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let hash: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let label: String = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("worktree")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    root_dir().join("worktrees").join(format!("{label}-{hash}"))
}

pub(crate) fn read_worktree_login(file: &Path) -> Option<String> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| text.lines().next().map(str::trim).map(str::to_string))
        .filter(|login| !login.is_empty())
}

pub(crate) fn write_worktree_login(file: &Path, login: &str) -> io::Result<()> {
    write_atomically(file, format!("{login}\n").as_bytes())
}

/// The account remembered for each repository owner, keyed by lowercase owner.
pub(crate) fn owner_logins() -> BTreeMap<String, String> {
    std::fs::read_to_string(owners_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn owner_login(owner: &str) -> Option<String> {
    owner_logins().remove(&owner.to_ascii_lowercase())
}

pub(crate) fn set_owner_login(owner: &str, login: &str) -> io::Result<()> {
    let mut owners = owner_logins();
    owners.insert(owner.to_ascii_lowercase(), login.to_string());
    let text = serde_json::to_string_pretty(&owners).map_err(io::Error::other)?;
    write_atomically(&owners_file(), text.as_bytes())
}

pub(super) fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
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
