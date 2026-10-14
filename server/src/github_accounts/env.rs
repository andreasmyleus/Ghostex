use super::{
    gh,
    resolve::{account_for_cwd, account_for_worktree, github_worktree_for_cwd, GithubWorktree},
    ssh, store,
};

/// What a gxserver-started command needs its GitHub account for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountCommand {
    /// A `gh` call: it gets the account's token.
    Gh,
    /// A `git` call that may talk to GitHub or make commits: the token, gh as
    /// the host's credential helper, the account's commit identity and the
    /// SSH wrapper.
    Git,
}

/// Environment for one of gxserver's own commands run in `cwd`, so PR status,
/// issue and project reads, fetches and commits act as the worktree's account.
/// Empty when the worktree has no account choice to make.
pub(crate) fn command_account_env(cwd: &str, kind: AccountCommand) -> Vec<(String, String)> {
    let Some(account) = account_for_cwd(cwd) else {
        return Vec::new();
    };
    let host = account.host().to_string();
    let Some(token) = gh::token_for(&host, &account.login) else {
        return Vec::new();
    };
    let mut environment = vec![(gh::token_env_key(&host).to_string(), token)];
    if kind == AccountCommand::Git {
        let mut entries = vec![identity_include_entry(&account.file)];
        if let Some(real_gh) = gh::real_gh_path() {
            let real_gh = real_gh.to_string_lossy().replace('\\', "/");
            entries.extend(credential_helper_entries(
                &host,
                &format!("!'{real_gh}' auth git-credential"),
            ));
        }
        environment.extend(git_config_env(git_config_count(), &entries));
        environment.push((
            "GHOSTEX_GITHUB_ACCOUNT_FILE".to_string(),
            account.file.to_string_lossy().into_owned(),
        ));
        if account.repo.remote.ssh && !user_picked_ssh() {
            if let Some(wrapper) = ssh::ensure_ssh_wrapper() {
                environment.push(("GIT_SSH_COMMAND".to_string(), wrapper_command(&wrapper)));
            }
        }
    }
    environment
}

/// Environment for cloning `url` (or looking it up with gh) before any
/// checkout exists: the token and credential helper of the account picked for
/// the repository's owner, and that account's SSH key for an SSH URL. Empty
/// when there is no choice to make. Blocking: it may ask GitHub which account
/// can reach the repository.
pub(crate) fn repository_account_env(url: &str) -> Vec<(String, String)> {
    let Some(remote) = super::repo::parse_github_remote(url) else {
        return Vec::new();
    };
    let Some(login) = super::resolve::account_for_repository(&remote) else {
        return Vec::new();
    };
    let Some(token) = gh::token_for(&remote.host, &login) else {
        return Vec::new();
    };
    let mut environment = vec![(gh::token_env_key(&remote.host).to_string(), token)];
    if let Some(real_gh) = gh::real_gh_path() {
        let real_gh = real_gh.to_string_lossy().replace('\\', "/");
        environment.extend(git_config_env(
            git_config_count(),
            &credential_helper_entries(&remote.host, &format!("!'{real_gh}' auth git-credential")),
        ));
    }
    if remote.ssh {
        if let Some(key) = ssh::account_key(&remote.host, &login) {
            environment.push((
                "GIT_SSH_COMMAND".to_string(),
                format!(
                    "ssh -i {} -o IdentitiesOnly=yes -o BatchMode=yes",
                    wrapper_command(&key)
                ),
            ));
        }
    }
    environment
}

fn user_picked_ssh() -> bool {
    ["GIT_SSH_COMMAND", "GIT_SSH"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

fn wrapper_command(wrapper: &std::path::Path) -> String {
    format!(
        "'{}'",
        wrapper
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}

fn git_config_count() -> usize {
    std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// `include.path` of the worktree's identity file, which includes the current
/// account's `[user] name/email`. git ignores an include that does not exist.
fn identity_include_entry(file: &std::path::Path) -> (String, String) {
    (
        "include.path".to_string(),
        store::worktree_identity_include(file)
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

/// An empty helper that drops any other credential helper for the host (a
/// keychain holding another account's login), then `helper`.
fn credential_helper_entries(host: &str, helper: &str) -> Vec<(String, String)> {
    let key = format!("credential.https://{host}.helper");
    vec![(key.clone(), String::new()), (key, helper.to_string())]
}

fn git_config_env(base: usize, entries: &[(String, String)]) -> Vec<(String, String)> {
    let mut environment = Vec::new();
    for (offset, (key, value)) in entries.iter().enumerate() {
        environment.push((format!("GIT_CONFIG_KEY_{}", base + offset), key.clone()));
        environment.push((format!("GIT_CONFIG_VALUE_{}", base + offset), value.clone()));
    }
    environment.push((
        "GIT_CONFIG_COUNT".to_string(),
        (base + entries.len()).to_string(),
    ));
    environment
}

/// The git config entries a session in this worktree gets: always the
/// identity include, and gh as the credential helper once there is an account
/// choice to make.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: Sessions in a GitHub worktree get the shim, the account file, the identity include and the SSH wrapper even while gh has only one account, because all of them do nothing until the account file exists: signing in a second account later then reaches sessions that are already running. Only the credential helper waits for a real choice, since it replaces the user's own helper (a keychain) for the host and would break HTTPS pushes for someone whose single account is not signed in to gh's git credentials.
fn session_git_entries(worktree: &GithubWorktree, helper: &str) -> Vec<(String, String)> {
    let mut entries = vec![identity_include_entry(&worktree.file)];
    if account_for_worktree(worktree.clone()).is_some() {
        entries.extend(credential_helper_entries(
            &worktree.repo.remote.host,
            helper,
        ));
    }
    entries
}

/// Shell lines a session's start script runs before it starts zmx: they point
/// the session at its worktree's account file, put the gh shim first on PATH,
/// use the SSH wrapper for an SSH remote, include the account's commit identity
/// and make git ask the shim for the host's credentials. Empty for a folder
/// that is not in a GitHub checkout, so such sessions start unchanged.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: The session only learns where to look (the account file's fixed path), never which account, so a switch from `gh auth switch`, the CLI, the app or a background access check reaches agents that are already running on their next gh, git or ssh call, with no restart and nothing stale in their environment. git uses the shim and the SSH wrapper by absolute path, so pushes stay on the right account even when a shell profile puts another gh ahead of the shim on PATH.
#[cfg(not(windows))]
pub(crate) fn session_shell_exports(cwd: &str) -> String {
    use crate::zmx::scripts::shell_quote;

    let Some(worktree) = github_worktree_for_cwd(cwd) else {
        return String::new();
    };
    let (Some(shim), Some(real_gh)) = (super::shim::ensure_shim(), gh::real_gh_path()) else {
        return String::new();
    };
    let helper = format!("!'{}' auth git-credential", shim.to_string_lossy());
    let mut lines = vec![
        format!(
            "export GHOSTEX_GITHUB_ACCOUNT_FILE={}",
            shell_quote(&worktree.file.to_string_lossy())
        ),
        format!(
            "export GHOSTEX_GITHUB_REAL_GH={}",
            shell_quote(&real_gh.to_string_lossy())
        ),
        "ghostex_git_config_index=${GIT_CONFIG_COUNT:-0}".to_string(),
    ];
    for (key, value) in session_git_entries(&worktree, &helper) {
        lines.push(format!("ghostex_git_config_value={}", shell_quote(&value)));
        lines.push(format!(
            "export \"GIT_CONFIG_KEY_${{ghostex_git_config_index}}={key}\""
        ));
        lines.push(
            "export \"GIT_CONFIG_VALUE_${ghostex_git_config_index}=$ghostex_git_config_value\""
                .to_string(),
        );
        lines.push("ghostex_git_config_index=$((ghostex_git_config_index + 1))".to_string());
    }
    lines.push("export GIT_CONFIG_COUNT=$ghostex_git_config_index".to_string());
    lines.push("unset ghostex_git_config_index ghostex_git_config_value".to_string());
    if worktree.repo.remote.ssh {
        if let Some(wrapper) = ssh::ensure_ssh_wrapper() {
            lines.push(format!(
                "if [ -z \"${{GIT_SSH_COMMAND:-}}\" ] && [ -z \"${{GIT_SSH:-}}\" ]; then export GIT_SSH_COMMAND={}; fi",
                shell_quote(&wrapper_command(&wrapper))
            ));
        }
    }
    lines.push(path_prepend_line(&super::shim::shim_dir()));
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The shim folder a session in a GitHub worktree puts first on PATH, to put
/// there again after the user's shell profiles (which may put another gh ahead
/// of it). `None` outside a GitHub worktree.
#[cfg(not(windows))]
pub(crate) fn session_shim_dir(cwd: &str) -> Option<String> {
    github_worktree_for_cwd(cwd)?;
    gh::real_gh_path()?;
    Some(super::shim::shim_dir().to_string_lossy().into_owned())
}

/// The PATH line again, or empty, for the agent's startup.
#[cfg(not(windows))]
pub(crate) fn session_startup_path_setup(cwd: &str) -> String {
    if session_shim_dir(cwd).is_none() {
        return String::new();
    }
    format!("{}\n", path_prepend_line(&super::shim::shim_dir()))
}

#[cfg(not(windows))]
fn path_prepend_line(dir: &std::path::Path) -> String {
    format!(
        "export PATH={}:\"$PATH\"",
        crate::zmx::scripts::shell_quote(&dir.to_string_lossy())
    )
}

/// What a native Windows (wmx) session needs: the environment to start it
/// with and the shim folder to put first on PATH after the PowerShell profile.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct WindowsSessionSetup {
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) path_dir: String,
}

/// The Windows counterpart of `session_shell_exports`: the same account file,
/// identity include, SSH wrapper and credential helper, with
/// `ghostex github-account gh-shim` as the shim.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_session_setup(cwd: &str) -> Option<WindowsSessionSetup> {
    let worktree = github_worktree_for_cwd(cwd)?;
    let cli = super::shim::ensure_windows_shims()?;
    let real_gh = gh::real_gh_path()?;
    let helper = format!(
        "!'{}' github-account gh-shim auth git-credential",
        cli.to_string_lossy().replace('\\', "/")
    );
    let mut environment = vec![
        (
            "GHOSTEX_GITHUB_ACCOUNT_FILE".to_string(),
            worktree.file.to_string_lossy().into_owned(),
        ),
        (
            "GHOSTEX_GITHUB_REAL_GH".to_string(),
            real_gh.to_string_lossy().into_owned(),
        ),
    ];
    environment.extend(git_config_env(
        git_config_count(),
        &session_git_entries(&worktree, &helper),
    ));
    if worktree.repo.remote.ssh && !user_picked_ssh() {
        if let Some(wrapper) = ssh::ensure_ssh_wrapper() {
            environment.push(("GIT_SSH_COMMAND".to_string(), wrapper_command(&wrapper)));
        }
    }
    Some(WindowsSessionSetup {
        environment,
        path_dir: super::shim::shim_dir().to_string_lossy().into_owned(),
    })
}

#[cfg(windows)]
pub(crate) fn session_shell_exports(_cwd: &str) -> String {
    String::new()
}
