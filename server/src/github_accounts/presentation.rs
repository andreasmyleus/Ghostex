use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::{
    gh,
    resolve::{account_for_worktree, github_worktree_for_cwd, GithubWorktree},
};

/// How long a project's repository facts (its root and `origin`, two git
/// spawns) are reused; the account itself is re-read on every pass.
const REPOSITORY_TTL: Duration = Duration::from_secs(10 * 60);

/// What the sidebar shows for a project: the account its worktree runs as and
/// the accounts it can switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectGithubAccount {
    pub(crate) login: String,
    pub(crate) accounts: Vec<String>,
    pub(crate) host: String,
    pub(crate) source: &'static str,
}

struct Entry {
    checked_at: Instant,
    worktree: Option<GithubWorktree>,
    value: Option<ProjectGithubAccount>,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The cached answer for a project folder. A pure read, so presentation stays
/// an in-memory projection.
pub(crate) fn cached_project_github_account(path: &str) -> Option<ProjectGithubAccount> {
    cache().lock().ok()?.get(path)?.value.clone()
}

/// Recomputes the account of each project folder and returns the folders whose
/// answer changed. `force` re-reads the repository facts too (after a switch).
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: The sidebar shows and switches each worktree's account, but presentation must not run git or write files, so a background pass fills this cache and the snapshot only reads it. Repository facts cost two git spawns and almost never change, so they are kept ten minutes; the account comes from file reads every pass, so a `gh auth switch` in a terminal shows within a minute.
pub(crate) fn refresh_project_github_accounts(paths: &[String], force: bool) -> Vec<String> {
    let mut changed = Vec::new();
    for path in paths {
        let previous = cache().lock().ok().and_then(|cache| {
            cache
                .get(path)
                .filter(|entry| !force && entry.checked_at.elapsed() < REPOSITORY_TTL)
                .map(|entry| (entry.checked_at, entry.worktree.clone()))
        });
        let (checked_at, worktree) = match previous {
            Some(previous) => previous,
            None => (Instant::now(), github_worktree_for_cwd(path)),
        };
        let value = worktree.clone().and_then(|mut worktree| {
            worktree.accounts = gh::accounts_for_host(&worktree.repo.remote.host);
            account_for_worktree(worktree).map(|account| ProjectGithubAccount {
                login: account.login.clone(),
                accounts: account.accounts.logins.clone(),
                host: account.repo.remote.host.clone(),
                source: account.source.as_wire(),
            })
        });
        let Ok(mut cache) = cache().lock() else {
            continue;
        };
        let before = cache.get(path).and_then(|entry| entry.value.clone());
        if before != value {
            changed.push(path.clone());
        }
        cache.insert(
            path.clone(),
            Entry {
                checked_at,
                worktree,
                value,
            },
        );
    }
    changed
}
