//! Per-worktree GitHub accounts: which of the user's `gh` accounts each git
//! worktree (the main checkout included) runs as, for agents in its sessions
//! and for gxserver's own `gh` and `git` calls there.

mod env;
mod gh;
mod repo;
mod resolve;
mod run;
mod shim;
mod store;

pub(crate) use env::{
    command_account_env, session_shell_exports, session_startup_path_setup, AccountCommand,
};
pub(crate) use gh::{global_accounts, real_gh_path, GhAccounts, GITHUB_HOST};
pub(crate) use repo::repo_for_cwd;
pub(crate) use resolve::{account_for_cwd, settled_account_for_cwd};
pub(crate) use store::{owner_logins, set_owner_login, write_worktree_login};
