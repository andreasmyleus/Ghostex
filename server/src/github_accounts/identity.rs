use std::{
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{gh, run::capture, store};

const IDENTITY_FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const IDENTITY_RETRY: Duration = Duration::from_secs(60 * 60);
/// First line of an identity the user set; Ghostex never replaces it.
const CUSTOM_MARKER: &str = "# set with ghostex github-account identity";

/// A commit author for one account, and the key its commits are signed with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) name: String,
    pub(crate) email: String,
    /// `user.signingkey`: a GPG key id, or an SSH key file for `gpg.format = ssh`.
    pub(crate) signing_key: Option<String>,
    /// `gpg.format` (`openpgp`, `ssh`, `x509`) when the account signs differently from the
    /// user's global setting.
    pub(crate) signing_format: Option<String>,
}

/// Makes sure the account's commit identity file exists, fetching it from
/// GitHub in the background when it is missing.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: GitHub attributes a commit to the account whose email it carries, so commits made in a worktree must use that worktree's account's email, not the global `user.email`. Sessions include a per-worktree git config file that includes the current account's identity; the identity is the account's public name and email, or its `users.noreply` address when the email is private, and `ghostex github-account identity` replaces it.
pub(crate) fn ensure_account_identity(host: &str, login: &str) {
    if store::account_identity_file(host, login).is_file() {
        return;
    }
    // One fetch per account at a time, and an account GitHub would not answer for is tried again
    // an hour later instead of on every presentation pass.
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let attempts = ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{host}/{login}").to_ascii_lowercase();
    let start = attempts
        .lock()
        .map(|mut attempts| match attempts.get(&key) {
            Some(at) if at.elapsed() < IDENTITY_RETRY => false,
            _ => {
                attempts.insert(key, Instant::now());
                true
            }
        })
        .unwrap_or(false);
    if !start {
        return;
    }
    let host = host.to_string();
    let login = login.to_string();
    std::thread::spawn(move || {
        if let Some(identity) = fetch_identity(&host, &login) {
            let _ = write_identity(
                &host,
                &login,
                &with_auto_signing(&host, &login, identity),
                false,
            );
        }
    });
}

/// The account's name and commit email as GitHub reports them.
pub(crate) fn fetch_identity(host: &str, login: &str) -> Option<Identity> {
    let token = gh::token_for(host, login)?;
    let real_gh = gh::real_gh_path()?;
    let mut command = gh::gh_store_command(&real_gh);
    command
        .env(gh::token_env_key(host), token)
        .args(["api", "--hostname", host, "user"]);
    let user: Value = serde_json::from_str(&capture(command, IDENTITY_FETCH_TIMEOUT)?).ok()?;
    let login = user.get("login").and_then(Value::as_str).unwrap_or(login);
    let name = user
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(login)
        .to_string();
    let email = user
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let id = user.get("id").and_then(Value::as_u64)?;
            Some(format!("{id}+{login}@users.noreply.{host}"))
        })?;
    Some(Identity {
        name,
        email,
        ..Identity::default()
    })
}

/// With SSH commit signing on (`gpg.format = ssh` in the user's global git
/// config), an account signs with the SSH key that signs in as it.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: GitHub shows a commit as Verified only when its signing key belongs to the account that authored it, so a worktree that commits as another account must also sign with that account's key. SSH signing gets the account's own SSH key automatically; a GPG key has no link to an account Ghostex can see, so it is set with `ghostex github-account identity <login> --signing-key`.
pub(crate) fn with_auto_signing(host: &str, login: &str, mut identity: Identity) -> Identity {
    if identity.signing_key.is_some() || global_git_config("gpg.format").as_deref() != Some("ssh") {
        return identity;
    }
    if let Some(key) = super::ssh::account_key(host, login) {
        let public = if key.extension().is_some_and(|ext| ext == "pub") {
            key
        } else {
            let mut public = key.clone().into_os_string();
            public.push(".pub");
            let public = std::path::PathBuf::from(public);
            if public.is_file() {
                public
            } else {
                key
            }
        };
        identity.signing_key = Some(public.to_string_lossy().replace('\\', "/"));
    }
    identity
}

/// Re-applies the automatic signing key to an account's identity Ghostex
/// fetched (never to one the user set), after its SSH key became known.
pub(crate) fn refresh_auto_signing(host: &str, login: &str) {
    if let Some((identity, false)) = read_identity(host, login) {
        let updated = with_auto_signing(host, login, identity.clone());
        if updated != identity {
            let _ = write_identity(host, login, &updated, false);
        }
    }
}

fn global_git_config(key: &str) -> Option<String> {
    let mut command = std::process::Command::new("git");
    command.args(["config", "--global", "--get", key]);
    capture(command, Duration::from_secs(3)).filter(|value| !value.is_empty())
}

pub(crate) fn read_identity(host: &str, login: &str) -> Option<(Identity, bool)> {
    let text = std::fs::read_to_string(store::account_identity_file(host, login)).ok()?;
    let custom = text.lines().next() == Some(CUSTOM_MARKER);
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"').to_string())
        })
    };
    Some((
        Identity {
            name: value("name")?,
            email: value("email")?,
            signing_key: value("signingkey"),
            signing_format: value("format"),
        },
        custom,
    ))
}

pub(crate) fn write_identity(
    host: &str,
    login: &str,
    identity: &Identity,
    custom: bool,
) -> io::Result<()> {
    let marker = if custom {
        format!("{CUSTOM_MARKER}\n")
    } else {
        String::new()
    };
    let mut text = format!(
        "{marker}[user]\n\tname = {}\n\temail = {}\n",
        store::git_config_quote(&identity.name),
        store::git_config_quote(&identity.email)
    );
    if let Some(key) = &identity.signing_key {
        text.push_str(&format!(
            "\tsigningkey = {}\n",
            store::git_config_quote(key)
        ));
    }
    if let Some(format) = &identity.signing_format {
        text.push_str(&format!(
            "[gpg]\n\tformat = {}\n",
            store::git_config_quote(format)
        ));
    }
    store::write_atomically(&store::account_identity_file(host, login), text.as_bytes())
}
