use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::{
    gh::{self, GhAccounts},
    repo::{self, RepoInfo},
    run::capture,
    store,
};

const ACCESS_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a worktree runs as its account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountSource {
    /// Chosen for this worktree (`gh auth switch`, `ghostex github-account use`, or an earlier pick).
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
}

#[derive(Clone, Debug)]
pub(crate) struct WorktreeAccount {
    pub(crate) repo: RepoInfo,
    pub(crate) file: PathBuf,
    pub(crate) login: String,
    pub(crate) source: AccountSource,
    pub(crate) accounts: GhAccounts,
}

/// The account a folder's worktree runs as, writing the choice to the
/// worktree's account file so sessions and the shim read the same answer.
///
/// CDXC:GitHubAccounts 2026-10-13 DECISION: The user asked for this to be seamless and simple, with no tokens to create: every account signed in to gh (`gh auth login`) can be used, and `gh auth switch` must keep working. So nothing is configured: with fewer than two gh accounts, or no github.com `origin`, everything behaves exactly as before (`None`). Otherwise the account is, in order, the one chosen for this worktree, the one remembered for the repository's owner, the account named like the owner, and else gh's active account while a background check asks GitHub which accounts can reach the repository and remembers the answer for the owner.
pub(crate) fn account_for_cwd(cwd: &str) -> Option<WorktreeAccount> {
    let accounts = gh::global_accounts();
    if accounts.logins.len() < 2 {
        return None;
    }
    let repo = repo::repo_for_cwd(cwd)?;
    let file = store::worktree_account_file(&repo.root);
    if let Some(login) =
        store::read_worktree_login(&file).and_then(|login| accounts.find(&login).cloned())
    {
        return Some(WorktreeAccount {
            repo,
            file,
            login,
            source: AccountSource::Worktree,
            accounts,
        });
    }
    if let Some(login) =
        store::owner_login(&repo.owner).and_then(|login| accounts.find(&login).cloned())
    {
        let _ = store::write_worktree_login(&file, &login);
        return Some(WorktreeAccount {
            repo,
            file,
            login,
            source: AccountSource::Owner,
            accounts,
        });
    }
    if let Some(login) = accounts.find(&repo.owner).cloned() {
        let _ = store::set_owner_login(&repo.owner, &login);
        let _ = store::write_worktree_login(&file, &login);
        return Some(WorktreeAccount {
            repo,
            file,
            login,
            source: AccountSource::OwnerName,
            accounts,
        });
    }
    // Nothing is written yet: with no account file the shim runs gh as gh's
    // active account, which is this provisional answer, until the check decides.
    let login = accounts
        .active
        .as_deref()
        .and_then(|active| accounts.find(active))
        .unwrap_or(&accounts.logins[0])
        .clone();
    spawn_access_check(&repo, file.clone(), accounts.clone());
    Some(WorktreeAccount {
        repo,
        file,
        login,
        source: AccountSource::GhActive,
        accounts,
    })
}

/// `account_for_cwd`, but a pending access check runs in the caller's thread:
/// a short-lived process such as the CLI would exit before a background one ends.
pub(crate) fn settled_account_for_cwd(cwd: &str) -> Option<WorktreeAccount> {
    let account = account_for_cwd(cwd)?;
    if account.source != AccountSource::GhActive {
        return Some(account);
    }
    if let Some(login) =
        best_account_for_repository(&account.repo.owner, &account.repo.name, &account.accounts)
    {
        let _ = store::set_owner_login(&account.repo.owner, &login);
        if store::read_worktree_login(&account.file).is_none() {
            let _ = store::write_worktree_login(&account.file, &login);
        }
    }
    account_for_cwd(cwd)
}

const ACCESS_CHECK_RETRY: Duration = Duration::from_secs(10 * 60);

/// Asks GitHub which signed-in accounts can reach the repository, at most once
/// per repository every ten minutes. The best one is remembered for the owner
/// and becomes the worktree's account unless someone chose one meanwhile.
fn spawn_access_check(repo: &RepoInfo, file: PathBuf, accounts: GhAccounts) {
    static STARTED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let started = STARTED.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase();
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
    let owner = repo.owner.clone();
    let name = repo.name.clone();
    std::thread::spawn(move || {
        let Some(login) = best_account_for_repository(&owner, &name, &accounts) else {
            return;
        };
        let _ = store::set_owner_login(&owner, &login);
        if store::read_worktree_login(&file).is_none() {
            let _ = store::write_worktree_login(&file, &login);
        }
    });
}

/// An account that can push wins over one that can only read; among equals,
/// gh's active account wins, then the first in name order.
fn best_account_for_repository(owner: &str, name: &str, accounts: &GhAccounts) -> Option<String> {
    let real_gh = gh::real_gh_path()?;
    let mut can_push = Vec::new();
    let mut can_read = Vec::new();
    for login in &accounts.logins {
        let Some(token) = gh::token_for(login) else {
            continue;
        };
        let mut command: Command = gh::gh_store_command(&real_gh);
        command.env("GH_TOKEN", token).args([
            "api",
            &format!("repos/{owner}/{name}"),
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
