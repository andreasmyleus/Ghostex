//! Per-worktree GitHub accounts: which of the user's `gh` accounts each git
//! worktree (the main checkout included) runs as, for agents in its sessions
//! and for gxserver's own `gh` and `git` calls there.

mod env;
mod gh;
mod identity;
mod presentation;
mod repo;
mod resolve;
mod run;
mod shim;
mod ssh;
mod store;
mod worktree_config;

#[cfg(windows)]
pub(crate) use env::windows_session_setup;
pub(crate) use env::{
    command_account_env, repository_account_env, session_shell_exports, AccountCommand,
};
#[cfg(not(windows))]
pub(crate) use env::{session_shim_dir, session_startup_path_setup};
pub(crate) use gh::{
    global_accounts, global_hosts, real_gh_for_shim, real_gh_path, token_env_key, token_for,
    GITHUB_HOST,
};
pub(crate) use identity::{
    fetch_identity, read_identity, with_auto_signing, write_identity, Identity,
};
pub(crate) use presentation::{cached_project_github_account, refresh_project_github_accounts};
pub(crate) use repo::repo_for_cwd;
pub(crate) use resolve::{
    account_for_cwd, choose_account_for_cwd, github_worktree_for_cwd, settled_account_for_cwd,
};
pub(crate) use ssh::{account_key, discover_keys, set_account_key};
pub(crate) use store::{owner_logins, prune_removed_worktrees, read_worktree_record};
pub(crate) use worktree_config::write_worktree_git_config;
