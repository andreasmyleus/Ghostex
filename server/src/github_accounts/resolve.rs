use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::{
    gh::{self, GhAccounts},
    identity,
    repo::{self, RemoteRef, RepoInfo},
    run::capture,
    ssh, store,
};

const ACCESS_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const ACCESS_CHECK_RETRY: Duration = Duration::from_secs(10 * 60);

/// Why a worktree runs as its account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountSource {
    /// Chosen for this worktree (`gh auth switch`, `ghostex github-account use`, the app, or an earlier pick).
    Worktree,
    /// Remembered for the repository's owner.
    Owner,
    /// The account has the owner's name.
    OwnerName,
    /// gh's active account, kept until the access check finishes.
    GhActive,
}

impl AccountSource {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Worktree => "chosen for this worktree",
            Self::Owner => "remembered for this owner",
            Self::OwnerName => "the account has the owner's name",
            Self::GhActive => {
                "gh's active account (checking which account can reach the repository)"
            }
        }
    }

    pub(crate) fn as_wire(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Owner => "owner",
            Self::OwnerName => "ownerName",
            Self::GhActive => "ghActive",
        }
    }
}

/// A git checkout on a GitHub host and the account file its sessions read.
#[derive(Clone, Debug)]
pub(crate) struct GithubWorktree {
    pub(crate) repo: RepoInfo,
    pub(crate) file: PathBuf,
    /// gh's accounts on the repository's host.
    pub(crate) accounts: GhAccounts,
}

#[derive(Clone, Debug)]
pub(crate) struct WorktreeAccount {
    pub(crate) repo: RepoInfo,
    pub(crate) file: PathBuf,
    pub(crate) login: String,
    pub(crate) source: AccountSource,
    pub(crate) accounts: GhAccounts,
}

impl WorktreeAccount {
    pub(crate) fn host(&self) -> &str {
        &self.repo.remote.host
    }
}

/// The folder's checkout when its `origin` is on a GitHub host gh is signed
/// in to with at least one account, whether or not there is a choice to make.
pub(crate) fn github_worktree_for_cwd(cwd: &str) -> Option<GithubWorktree> {
    let repo = repo::repo_for_cwd(cwd)?;
    let accounts = gh::accounts_for_host(&repo.remote.host);
    if accounts.logins.is_empty() {
        return None;
    }
    let file = store::worktree_account_file(&repo.root);
    Some(GithubWorktree {
        repo,
        file,
        accounts,
    })
}

/// The account a folder's worktree runs as, writing the choice to the
/// worktree's account file so sessions, the shims and the SSH wrapper read the
/// same answer.
///
/// CDXC:GitHubAccounts 2026-10-13 DECISION: The user asked for this to be seamless and simple, with no tokens to create: every account signed in to gh (`gh auth login`) can be used, and `gh auth switch` must keep working. So nothing is configured: with fewer than two gh accounts on the repository's host, or no GitHub `origin`, everything behaves exactly as before (`None`). Otherwise the account is, in order, the one chosen for this worktree, the one remembered for the repository's owner, the account named like the owner, and else gh's active account while a background check asks GitHub which accounts can reach the repository and remembers the answer for the owner.
pub(crate) fn account_for_cwd(cwd: &str) -> Option<WorktreeAccount> {
    account_for_worktree(github_worktree_for_cwd(cwd)?)
}

pub(crate) fn account_for_worktree(worktree: GithubWorktree) -> Option<WorktreeAccount> {
    let GithubWorktree {
        repo,
        file,
        accounts,
    } = worktree;
    if accounts.logins.len() < 2 {
        return None;
    }
    let remote = repo.remote.clone();
    let chosen = store::read_worktree_record(&file)
        .filter(|record| record.host == remote.host)
        .and_then(|record| accounts.find(&record.login).cloned());
    let (login, source) = if let Some(login) = chosen {
        (login, AccountSource::Worktree)
    } else if let Some((login, source)) = remembered_or_named(&remote, &accounts) {
        let _ = store::write_worktree_login(&file, &remote.host, &login, &repo.root);
        (login, source)
    } else {
        // Nothing is written yet: with no account file the shims run gh as gh's
        // active account, which is this provisional answer, until the check decides.
        spawn_access_check(&repo, file.clone(), accounts.clone());
        (gh_active(&accounts), AccountSource::GhActive)
    };
    identity::ensure_account_identity(&remote.host, &login);
    if remote.ssh {
        ssh::ensure_keys_discovered(&remote.host, &accounts);
    }
    Some(WorktreeAccount {
        repo,
        file,
        login,
        source,
        accounts,
    })
}

fn remembered_or_named(
    remote: &RemoteRef,
    accounts: &GhAccounts,
) -> Option<(String, AccountSource)> {
    if let Some(login) = store::owner_login(&remote.host, &remote.owner)
        .and_then(|login| accounts.find(&login).cloned())
    {
        return Some((login, AccountSource::Owner));
    }
    let login = accounts.find(&remote.owner).cloned()?;
    let _ = store::set_owner_login(&remote.host, &remote.owner, &login);
    Some((login, AccountSource::OwnerName))
}

fn gh_active(accounts: &GhAccounts) -> String {
    accounts
        .active
        .as_deref()
        .and_then(|active| accounts.find(active))
        .unwrap_or(&accounts.logins[0])
        .clone()
}

/// The account for a repository that is not checked out yet (a clone or a
/// repository lookup): the remembered or owner-named account, else the answer
/// GitHub gives now. `None` when there is no choice to make.
pub(crate) fn account_for_repository(remote: &RemoteRef) -> Option<String> {
    let accounts = gh::accounts_for_host(&remote.host);
    if accounts.logins.len() < 2 {
        return None;
    }
    if let Some((login, _)) = remembered_or_named(remote, &accounts) {
        return Some(login);
    }
    let login = best_account_for_repository(remote, &accounts)?;
    let _ = store::set_owner_login(&remote.host, &remote.owner, &login);
    Some(login)
}

/// Sets the account of the worktree that holds `cwd`, optionally remembering
/// it for the repository's owner. Returns the updated account.
pub(crate) fn choose_account_for_cwd(
    cwd: &str,
    login: &str,
    remember_owner: bool,
) -> Result<WorktreeAccount, String> {
    let worktree = github_worktree_for_cwd(cwd).ok_or_else(|| {
        "This folder is not in a git checkout whose origin is on a GitHub host signed in to gh."
            .to_string()
    })?;
    let login = worktree.accounts.find(login).cloned().ok_or_else(|| {
        format!(
            "{login} is not signed in to gh. Signed in: {}. Add it with: gh auth login",
            worktree.accounts.logins.join(", ")
        )
    })?;
    let remote = worktree.repo.remote.clone();
    store::write_worktree_login(&worktree.file, &remote.host, &login, &worktree.repo.root)
        .map_err(|error| format!("Could not save the worktree's GitHub account: {error}"))?;
    if remember_owner {
        store::set_owner_login(&remote.host, &remote.owner, &login).map_err(|error| {
            format!(
                "Could not remember the account for {}: {error}",
                remote.owner
            )
        })?;
    }
    account_for_worktree(worktree)
        .ok_or_else(|| "Only one account is signed in to gh on this host.".to_string())
}

/// `account_for_cwd`, but a pending access check runs in the caller's thread:
/// a short-lived process such as the CLI would exit before a background one ends.
pub(crate) fn settled_account_for_cwd(cwd: &str) -> Option<WorktreeAccount> {
    let account = account_for_cwd(cwd)?;
    if account.source != AccountSource::GhActive {
        return Some(account);
    }
    let remote = account.repo.remote.clone();
    if let Some(login) = best_account_for_repository(&remote, &account.accounts) {
        let _ = store::set_owner_login(&remote.host, &remote.owner, &login);
        if store::read_worktree_login(&account.file).is_none() {
            let _ = store::write_worktree_login(
                &account.file,
                &remote.host,
                &login,
                &account.repo.root,
            );
        }
    }
    account_for_cwd(cwd)
}

/// Asks GitHub which signed-in accounts can reach the repository, at most once
/// per repository every ten minutes. The best one is remembered for the owner
/// and becomes the worktree's account unless someone chose one meanwhile.
fn spawn_access_check(repo: &RepoInfo, file: PathBuf, accounts: GhAccounts) {
    static STARTED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let started = STARTED.get_or_init(|| Mutex::new(HashMap::new()));
    let remote = repo.remote.clone();
    let key = format!("{}/{}/{}", remote.host, remote.owner, remote.name).to_ascii_lowercase();
    let start = started
        .lock()
        .map(|mut started| match started.get(&key) {
            Some(at) if at.elapsed() < ACCESS_CHECK_RETRY => false,
            _ => {
                started.insert(key, Instant::now());
                true
            }
        })
        .unwrap_or(false);
    if !start {
        return;
    }
    let root = repo.root.clone();
    std::thread::spawn(move || {
        let Some(login) = best_account_for_repository(&remote, &accounts) else {
            return;
        };
        let _ = store::set_owner_login(&remote.host, &remote.owner, &login);
        if store::read_worktree_login(&file).is_none() {
            let _ = store::write_worktree_login(&file, &remote.host, &login, &root);
        }
    });
}

/// An account that can push wins over one that can only read; among equals,
/// gh's active account wins, then the first in name order.
fn best_account_for_repository(remote: &RemoteRef, accounts: &GhAccounts) -> Option<String> {
    let real_gh = gh::real_gh_path()?;
    let mut can_push = Vec::new();
    let mut can_read = Vec::new();
    for login in &accounts.logins {
        let Some(token) = gh::token_for(&remote.host, login) else {
            continue;
        };
        let mut command: Command = gh::gh_store_command(&real_gh);
        command.env(gh::token_env_key(&remote.host), token).args([
            "api",
            "--hostname",
            &remote.host,
            &format!("repos/{}/{}", remote.owner, remote.name),
            "--jq",
            ".permissions.push",
        ]);
        match capture(command, ACCESS_PROBE_TIMEOUT).as_deref() {
            Some("true") => can_push.push(login.clone()),
            Some(_) => can_read.push(login.clone()),
            None => {}
        }
    }
    let pick = |candidates: &[String]| -> Option<String> {
        accounts
            .active
            .as_ref()
            .and_then(|active| {
                candidates
                    .iter()
                    .find(|login| login.eq_ignore_ascii_case(active))
            })
            .or_else(|| candidates.first())
            .cloned()
    };
    pick(&can_push).or_else(|| pick(&can_read))
}
