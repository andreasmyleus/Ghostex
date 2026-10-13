use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::{gh::GhAccounts, store};

const DISCOVERY_RETRY: Duration = Duration::from_secs(6 * 60 * 60);

/// The SSH wrapper git uses (`GIT_SSH_COMMAND`) in sessions with a GitHub
/// remote. gxserver writes it; it returns its path.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: An SSH remote (`git@github.com:o/r`) signs in with whichever key ssh offers first, so pushes went out as the same account in every worktree. The wrapper reads the worktree's account file on every connection and, only for a connection to that account's host, adds `-i <key> -o IdentitiesOnly=yes` with the key found for the account (`ssh -T` answers "Hi <login>!" per key); any other connection, or an account without a known key, runs the user's `core.sshCommand` or plain ssh unchanged.
pub(crate) fn ensure_ssh_wrapper() -> Option<PathBuf> {
    let dir = super::shim::shim_dir();
    let path = dir.join("ghostex-ssh");
    let script = wrapper_script(&store::root_dir());
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        store::write_atomically(&path, script.as_bytes()).ok()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path).ok()?.permissions();
        if permissions.mode() & 0o777 != 0o755 {
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).ok()?;
        }
    }
    Some(path)
}

fn wrapper_script(root: &Path) -> String {
    let root = root
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "'\\''");
    format!(
        r#"#!/bin/sh
# Ghostex SSH wrapper for GitHub accounts. gxserver writes this file; edits are overwritten.
# Signs in to the worktree's GitHub host with the key of the worktree's account.
ghostex_root='{root}'
ghostex_login=
ghostex_host=
if [ -n "${{GHOSTEX_GITHUB_ACCOUNT_FILE:-}}" ] && [ -r "$GHOSTEX_GITHUB_ACCOUNT_FILE" ]; then
  {{ IFS= read -r ghostex_login; IFS= read -r ghostex_host; }} < "$GHOSTEX_GITHUB_ACCOUNT_FILE" || :
fi
ghostex_key=
if [ -n "$ghostex_login" ] && [ -n "$ghostex_host" ] && [ -r "$ghostex_root/ssh-keys/$ghostex_host/$ghostex_login" ]; then
  IFS= read -r ghostex_key < "$ghostex_root/ssh-keys/$ghostex_host/$ghostex_login" || :
fi
ghostex_match=
if [ -n "$ghostex_key" ] && [ -r "$ghostex_key" ]; then
  for ghostex_arg in "$@"; do
    case "$ghostex_arg" in
      "$ghostex_host"|*@"$ghostex_host") ghostex_match=1; break ;;
    esac
  done
fi
if [ -n "$ghostex_match" ]; then
  exec ssh -i "$ghostex_key" -o IdentitiesOnly=yes "$@"
fi
ghostex_ssh_command=$(git config --get core.sshCommand 2>/dev/null)
if [ -n "$ghostex_ssh_command" ]; then
  eval "exec $ghostex_ssh_command \"\$@\""
fi
exec ssh "$@"
"#
    )
}

/// The key saved for an account, if any.
pub(crate) fn account_key(host: &str, login: &str) -> Option<PathBuf> {
    std::fs::read_to_string(store::ssh_key_file(host, login))
        .ok()
        .map(|text| PathBuf::from(text.trim()))
        .filter(|path| path.is_file())
}

pub(crate) fn set_account_key(host: &str, login: &str, key: &Path) -> std::io::Result<()> {
    store::write_atomically(
        &store::ssh_key_file(host, login),
        format!("{}\n", key.to_string_lossy().replace('\\', "/")).as_bytes(),
    )
}

/// Finds which of the user's SSH keys sign in as which account on `host`, in
/// the background, when an account has no key yet. At most every six hours
/// per host.
pub(crate) fn ensure_keys_discovered(host: &str, accounts: &GhAccounts) {
    if accounts
        .logins
        .iter()
        .all(|login| account_key(host, login).is_some())
    {
        return;
    }
    static STARTED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let started = STARTED.get_or_init(|| Mutex::new(HashMap::new()));
    let start = started
        .lock()
        .map(|mut started| match started.get(host) {
            Some(at) if at.elapsed() < DISCOVERY_RETRY => false,
            _ => {
                started.insert(host.to_string(), Instant::now());
                true
            }
        })
        .unwrap_or(false);
    if !start {
        return;
    }
    let host = host.to_string();
    let accounts = accounts.clone();
    std::thread::spawn(move || discover_keys(&host, &accounts));
}

/// Tries every candidate key against `host` and saves the ones GitHub greets
/// as a signed-in account. Never replaces a key the user chose.
pub(crate) fn discover_keys(host: &str, accounts: &GhAccounts) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for key in candidate_keys() {
        let Some(login) = greeted_login(host, &key) else {
            continue;
        };
        let Some(login) = accounts.find(&login).cloned() else {
            continue;
        };
        if account_key(host, &login).is_none() && set_account_key(host, &login, &key).is_ok() {
            found.push((login, key));
        }
    }
    found
}

/// `~/.ssh` keys (the private key when present, else its `.pub` for a key the
/// agent holds) and the SSH agent's other keys, written out as `.pub` files.
fn candidate_keys() -> Vec<PathBuf> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    let ssh_dir = ghostex_paths::GhostexPaths::resolve().home_dir.join(".ssh");
    if let Ok(entries) = std::fs::read_dir(&ssh_dir) {
        let mut public: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "pub"))
            .collect();
        public.sort();
        for public_key in public {
            if let Ok(text) = std::fs::read_to_string(&public_key) {
                seen.insert(key_body(&text));
            }
            let private_key = public_key.with_extension("");
            keys.push(if private_key.is_file() {
                private_key
            } else {
                public_key
            });
        }
    }
    let agent_dir = store::root_dir().join("ssh-keys").join("agent");
    if let Ok(output) = Command::new("ssh-add")
        .arg("-L")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        for (index, line) in String::from_utf8_lossy(&output.stdout).lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || !seen.insert(key_body(line)) {
                continue;
            }
            let path = agent_dir.join(format!("agent-{index}.pub"));
            if store::write_atomically(&path, format!("{line}\n").as_bytes()).is_ok() {
                keys.push(path);
            }
        }
    }
    keys
}

fn key_body(text: &str) -> String {
    text.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `ssh -T git@<host>` with one key; GitHub answers on stderr with
/// "Hi <login>! You've successfully authenticated".
fn greeted_login(host: &str, key: &Path) -> Option<String> {
    let mut command = Command::new("ssh");
    command
        .args(["-T", "-i"])
        .arg(key)
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ConnectTimeout=8",
            &format!("git@{host}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::platform::process::NoConsoleWindow::no_console_window(&mut command);
    let output = command.output().ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let rest = stderr.split("Hi ").nth(1)?;
    let login = rest.split('!').next()?.trim();
    (!login.is_empty() && !login.contains(char::is_whitespace)).then(|| login.to_string())
}
