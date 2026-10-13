use super::{gh, resolve::account_for_cwd};

/// What a gxserver-started command needs its GitHub account for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountCommand {
    /// A `gh` call: it gets `GH_TOKEN`.
    Gh,
    /// A `git` call that may talk to GitHub over HTTPS: `GH_TOKEN` plus gh as
    /// the credential helper for github.com.
    GitNetwork,
}

/// Environment for one of gxserver's own commands run in `cwd`, so PR status,
/// issue and project reads and `git fetch` act as the worktree's account.
/// Empty when the worktree has no account choice to make.
pub(crate) fn command_account_env(cwd: &str, kind: AccountCommand) -> Vec<(String, String)> {
    let Some(account) = account_for_cwd(cwd) else {
        return Vec::new();
    };
    let Some(token) = gh::token_for(&account.login) else {
        return Vec::new();
    };
    let mut environment = vec![("GH_TOKEN".to_string(), token)];
    if kind == AccountCommand::GitNetwork {
        if let Some(real_gh) = gh::real_gh_path() {
            let real_gh = real_gh.to_string_lossy().replace('\\', "/");
            let helper = format!("!'{real_gh}' auth git-credential");
            environment.extend(git_credential_helper_env(git_config_count(), &helper));
        }
    }
    environment
}

fn git_config_count() -> usize {
    std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// Two `GIT_CONFIG_*` entries after `base`: an empty helper that drops any
/// other credential helper for github.com (a keychain holding another
/// account's login), then `helper`.
fn git_credential_helper_env(base: usize, helper: &str) -> Vec<(String, String)> {
    let key = "credential.https://github.com.helper".to_string();
    vec![
        (format!("GIT_CONFIG_KEY_{base}"), key.clone()),
        (format!("GIT_CONFIG_VALUE_{base}"), String::new()),
        (format!("GIT_CONFIG_KEY_{}", base + 1), key),
        (format!("GIT_CONFIG_VALUE_{}", base + 1), helper.to_string()),
        ("GIT_CONFIG_COUNT".to_string(), (base + 2).to_string()),
    ]
}

/// Shell lines a session's start script runs before it starts zmx: they point
/// the session at its worktree's account file, put the gh shim first on PATH,
/// and make git ask the shim for github.com credentials. Empty when the
/// worktree has no account choice to make, so such sessions start unchanged.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: The session only learns where to look (the account file's fixed path), never which account, so a switch from `gh auth switch`, the CLI or a background access check reaches agents that are already running on their next gh or git call, with no restart and nothing stale in their environment. git uses the shim by absolute path, so pushes stay on the right account even when a shell profile puts another gh ahead of the shim on PATH.
#[cfg(not(windows))]
pub(crate) fn session_shell_exports(cwd: &str) -> String {
    use crate::zmx::scripts::shell_quote;

    let Some(account) = account_for_cwd(cwd) else {
        return String::new();
    };
    let Some(shim) = super::shim::ensure_shim() else {
        return String::new();
    };
    let shim_dir = super::shim::shim_dir();
    let helper = format!("!'{}' auth git-credential", shim.to_string_lossy());
    let real_gh = gh::real_gh_path()
        .map(|path| {
            format!(
                "export GHOSTEX_GITHUB_REAL_GH={}\n",
                shell_quote(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();
    format!(
        r#"export GHOSTEX_GITHUB_ACCOUNT_FILE={file}
{real_gh}ghostex_git_config_index=${{GIT_CONFIG_COUNT:-0}}
ghostex_git_credential_helper={helper}
export "GIT_CONFIG_KEY_${{ghostex_git_config_index}}=credential.https://github.com.helper"
export "GIT_CONFIG_VALUE_${{ghostex_git_config_index}}="
ghostex_git_config_index=$((ghostex_git_config_index + 1))
export "GIT_CONFIG_KEY_${{ghostex_git_config_index}}=credential.https://github.com.helper"
export "GIT_CONFIG_VALUE_${{ghostex_git_config_index}}=$ghostex_git_credential_helper"
export GIT_CONFIG_COUNT=$((ghostex_git_config_index + 1))
unset ghostex_git_config_index ghostex_git_credential_helper
{path}
"#,
        file = shell_quote(&account.file.to_string_lossy()),
        helper = shell_quote(&helper),
        path = path_prepend_line(&shim_dir),
    )
}

/// The PATH line again, for the agent's own startup: it runs after the user's
/// shell profiles, which may put another gh ahead of the shim.
#[cfg(not(windows))]
pub(crate) fn session_startup_path_setup(cwd: &str) -> String {
    if account_for_cwd(cwd).is_none() {
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

/// The Windows counterpart of `session_shell_exports`: the same account file
/// and git credential helper, with `ghostex github-account gh-shim` as the shim.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_session_setup(cwd: &str) -> Option<WindowsSessionSetup> {
    let account = account_for_cwd(cwd)?;
    let cli = super::shim::ensure_windows_shims()?;
    let helper = format!(
        "!'{}' github-account gh-shim auth git-credential",
        cli.to_string_lossy().replace('\\', "/")
    );
    let mut environment = vec![(
        "GHOSTEX_GITHUB_ACCOUNT_FILE".to_string(),
        account.file.to_string_lossy().into_owned(),
    )];
    if let Some(real_gh) = gh::real_gh_path() {
        environment.push((
            "GHOSTEX_GITHUB_REAL_GH".to_string(),
            real_gh.to_string_lossy().into_owned(),
        ));
    }
    environment.extend(git_credential_helper_env(git_config_count(), &helper));
    Some(WindowsSessionSetup {
        environment,
        path_dir: super::shim::shim_dir().to_string_lossy().into_owned(),
    })
}

#[cfg(windows)]
pub(crate) fn session_shell_exports(_cwd: &str) -> String {
    String::new()
}
