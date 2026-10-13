use std::path::PathBuf;

use super::store;

pub(crate) fn shim_dir() -> PathBuf {
    store::root_dir().join("bin")
}

/// Whether `dir` is the shim folder, however PATH spells it.
pub(crate) fn is_shim_dir(dir: &std::path::Path) -> bool {
    let shim = shim_dir();
    if dir == shim {
        return true;
    }
    match (std::fs::canonicalize(dir), std::fs::canonicalize(&shim)) {
        (Ok(dir), Ok(shim)) => dir == shim,
        _ => false,
    }
}

/// The shims a native Windows (wmx) session finds first on PATH: `gh.cmd` for
/// PowerShell and cmd, and an extensionless `gh` for Git Bash (the shell
/// Claude Code runs its commands in on Windows). Both hand the call to
/// `ghostex github-account gh-shim`, which does what the POSIX shim does.
/// Returns the `ghostex` executable they call.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn ensure_windows_shims() -> Option<PathBuf> {
    let cli = std::env::current_exe()
        .ok()?
        .with_file_name(if cfg!(windows) {
            "ghostex.exe"
        } else {
            "ghostex"
        });
    if !cli.is_file() {
        return None;
    }
    let dir = shim_dir();
    let cmd = format!(
        "@echo off\r\n\"{}\" github-account gh-shim %*\r\nexit /b %ERRORLEVEL%\r\n",
        cli.to_string_lossy()
    );
    let bash = format!(
        "#!/bin/sh\n# Ghostex GitHub account shim for Git Bash. gxserver writes this file; edits are overwritten.\nexec '{}' github-account gh-shim \"$@\"\n",
        cli.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")
    );
    for (name, contents) in [("gh.cmd", cmd), ("gh", bash)] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
            store::write_atomically(&path, contents.as_bytes()).ok()?;
        }
    }
    Some(cli)
}

/// Writes the `gh` shim when it is missing or out of date and returns its path.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: Giving each worktree its own `GH_CONFIG_DIR` was tried first and must not come back: gh 2.40+ keeps the active token in one shared OS keyring entry per host, and `gh auth switch` deletes and rewrites that entry (`activateUser` in cli/cli internal/config/config.go), so a switch inside one worktree switched every gh on the computer. The shim instead reads the worktree's account file on every call, runs the real gh with `GH_TOKEN` from `gh auth token --user <login>` (a read-only lookup), and turns `gh auth switch` into `ghostex github-account gh-switch`, so a switch stays inside the worktree and no token is ever written to disk.
/// SEE-ALSO: server/src/github_accounts/env.rs (the session exports that put the shim on PATH and use it as git's credential helper), server/src/ghostex_cli/github_account.rs (`gh-switch`, and `gh-shim`, the same rules in Rust for native Windows sessions), server/src/zmx/scripts_windows.rs (`start`).
#[cfg(not(windows))]
pub(crate) fn ensure_shim() -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let dir = shim_dir();
    let path = dir.join("gh");
    let script = shim_script(&dir);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        store::write_atomically(&path, script.as_bytes()).ok()?;
    }
    let mut permissions = std::fs::metadata(&path).ok()?.permissions();
    if permissions.mode() & 0o777 != 0o755 {
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).ok()?;
    }
    Some(path)
}

#[cfg(not(windows))]
fn shim_script(dir: &std::path::Path) -> String {
    use crate::zmx::scripts::shell_quote;

    let cli = crate::zmx::scripts::session_ghostex_cli_executable().unwrap_or_default();
    format!(
        r#"#!/bin/sh
# Ghostex GitHub account shim. gxserver writes this file; edits are overwritten.
# Runs the real gh as the GitHub account Ghostex chose for this worktree, and
# makes `gh auth switch` switch this worktree only. See `ghostex github-account`.
ghostex_shim_dir={dir}
ghostex_cli={cli}
ghostex_real_gh=
ghostex_saved_ifs=$IFS
IFS=:
for ghostex_path_dir in $PATH; do
  case "$ghostex_path_dir" in ""|"$ghostex_shim_dir") continue ;; esac
  if [ -f "$ghostex_path_dir/gh" ] && [ -x "$ghostex_path_dir/gh" ]; then
    ghostex_real_gh=$ghostex_path_dir/gh
    break
  fi
done
IFS=$ghostex_saved_ifs
if [ -z "$ghostex_real_gh" ] && [ -n "${{GHOSTEX_GITHUB_REAL_GH:-}}" ] && [ -x "$GHOSTEX_GITHUB_REAL_GH" ]; then
  ghostex_real_gh=$GHOSTEX_GITHUB_REAL_GH
fi
if [ -z "$ghostex_real_gh" ]; then
  printf '%s\n' 'gh: command not found' >&2
  exit 127
fi
ghostex_login=
if [ -n "${{GHOSTEX_GITHUB_ACCOUNT_FILE:-}}" ] && [ -r "$GHOSTEX_GITHUB_ACCOUNT_FILE" ]; then
  IFS= read -r ghostex_login < "$GHOSTEX_GITHUB_ACCOUNT_FILE" || :
fi
if [ -z "$ghostex_login" ]; then
  exec "$ghostex_real_gh" "$@"
fi
if [ "${{1:-}}" = auth ]; then
  case "${{2:-}}" in
    switch)
      shift 2
      export GHOSTEX_GITHUB_REAL_GH="$ghostex_real_gh"
      if [ -n "$ghostex_cli" ] && [ -x "$ghostex_cli" ]; then
        exec "$ghostex_cli" github-account gh-switch "$@"
      fi
      if command -v ghostex >/dev/null 2>&1; then
        exec ghostex github-account gh-switch "$@"
      fi
      printf '%s\n' 'Ghostex could not find its ghostex command, so this worktree keeps its GitHub account.' >&2
      exit 1
      ;;
    login|logout|refresh|setup-git)
      exec env -u GH_TOKEN -u GITHUB_TOKEN "$ghostex_real_gh" "$@"
      ;;
  esac
fi
ghostex_token=$(env -u GH_TOKEN -u GITHUB_TOKEN "$ghostex_real_gh" auth token --hostname github.com --user "$ghostex_login" 2>/dev/null)
if [ -z "$ghostex_token" ]; then
  printf 'Ghostex: the GitHub account %s is not signed in to gh, so gh uses its active account. Sign it in with: gh auth login\n' "$ghostex_login" >&2
  exec "$ghostex_real_gh" "$@"
fi
GH_TOKEN=$ghostex_token
export GH_TOKEN
unset ghostex_token
exec "$ghostex_real_gh" "$@"
"#,
        dir = shell_quote(&dir.to_string_lossy()),
        cli = shell_quote(&cli),
    )
}
