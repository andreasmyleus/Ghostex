use std::{process::Command, time::Duration};

use super::{resolve::account_for_cwd, run::capture, shim, ssh, store};

const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Writes the worktree's GitHub account into its own git config (`config.worktree`): the commit
/// identity include, the gh shim as the host's credential helper and, for an SSH remote, the SSH
/// wrapper, each naming the worktree's account file itself. Returns whether anything was written.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: An agentbox session's git runs through agentbox's host relay (or copies a token from the git credential helper for a cloud box), a long-lived process that never sees a session's environment, so the per-session `GIT_CONFIG_*` exports cannot reach it. Config in the worktree itself reaches every git process that works in it. It is written only when a box starts for a worktree with an account choice, and needs `extensions.worktreeConfig`, which git accepts in every repository format.
pub(crate) fn write_worktree_git_config(cwd: &str) -> bool {
    let Some(account) = account_for_cwd(cwd) else {
        return false;
    };
    #[cfg(not(windows))]
    let Some(shim) = shim::ensure_shim() else {
        return false;
    };
    #[cfg(windows)]
    let Some(shim) = shim::ensure_windows_shims() else {
        return false;
    };
    let file = account.file.to_string_lossy().replace('\\', "/");
    let shim = shim.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let helper = format!(
        "!GHOSTEX_GITHUB_ACCOUNT_FILE='{file}' '{shim}' github-account gh-shim auth git-credential"
    );
    #[cfg(not(windows))]
    let helper = format!("!GHOSTEX_GITHUB_ACCOUNT_FILE='{file}' '{shim}' auth git-credential");
    let root = account.repo.root.to_string_lossy().into_owned();
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");
        capture(command, GIT_TIMEOUT)
    };
    if git(&["config", "--get", "extensions.worktreeConfig"]).as_deref() != Some("true")
        && git(&["config", "extensions.worktreeConfig", "true"]).is_none()
    {
        return false;
    }
    let key = format!("credential.https://{}.helper", account.repo.remote.host);
    let include = store::worktree_identity_include(&account.file)
        .to_string_lossy()
        .replace('\\', "/");
    let _ = git(&[
        "config",
        "--worktree",
        "--unset-all",
        "include.path",
        &regex_literal(&include),
    ]);
    let _ = git(&["config", "--worktree", "--add", "include.path", &include]);
    let _ = git(&["config", "--worktree", "--unset-all", &key]);
    let _ = git(&["config", "--worktree", "--add", &key, ""]);
    let _ = git(&["config", "--worktree", "--add", &key, &helper]);
    if account.repo.remote.ssh && git(&["config", "--get", "core.sshCommand"]).is_none() {
        if let Some(wrapper) = ssh::ensure_ssh_wrapper() {
            let wrapper = wrapper.to_string_lossy().replace('\\', "/");
            let _ = git(&[
                "config",
                "--worktree",
                "core.sshCommand",
                &format!("GHOSTEX_GITHUB_ACCOUNT_FILE='{file}' '{wrapper}'"),
            ]);
        }
    }
    true
}

/// `value` as a git config value pattern that matches only itself.
fn regex_literal(value: &str) -> String {
    let mut pattern = String::from("^");
    for ch in value.chars() {
        if "\\^$.|?*+()[]{}".contains(ch) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('$');
    pattern
}
