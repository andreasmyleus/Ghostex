use serde_json::json;

use crate::ghostex_cli::output::{cli_args_want_json, print_json};
use crate::ghostex_cli::rpc::{CliError, CliResult};
use crate::github_accounts::{self, GITHUB_HOST};

const USAGE: &str = "Ghostex GitHub accounts - which of your gh accounts each worktree uses

Ghostex picks the account by itself when more than one account is signed in to
gh (`gh auth login`) and the repository's origin is on github.com. Inside a
Ghostex session, `gh auth switch` switches the current worktree only.

Usage:
  ghostex github-account status [--cwd <folder>] [--json]
      Show the account the folder's worktree uses and why
  ghostex github-account list [--json]
      List your gh accounts and the account remembered for each owner
  ghostex github-account use [<login>] [--cwd <folder>] [--this-worktree]
      Switch the worktree to <login> (no login: the next account), and remember
      it for the repository's owner unless --this-worktree is given";

pub fn github_account_command(args: &[String]) -> CliResult<()> {
    let subcommand = args.first().map(String::as_str).unwrap_or("status");
    let rest = args.get(1..).unwrap_or_default();
    match subcommand {
        "gh-switch" => gh_switch(rest),
        "gh-shim" => gh_shim(rest),
        _ if args.iter().any(|arg| arg == "-h" || arg == "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "status" => status(rest),
        "list" => list(rest),
        "use" => use_command(rest),
        // Undocumented: prints what a session started in the folder would export.
        "session-env" => {
            print!(
                "{}",
                github_accounts::session_shell_exports(&cwd_from(rest)?)
            );
            Ok(())
        }
        other => Err(CliError::Other(format!(
            "Unknown github-account command \"{other}\".\n\n{USAGE}"
        ))),
    }
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    args.iter().enumerate().find_map(|(index, arg)| {
        if arg == name {
            args.get(index + 1).cloned()
        } else {
            arg.strip_prefix(&prefix).map(str::to_string)
        }
    })
}

fn positionals(args: &[String], value_options: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if value_options.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        values.push(arg.clone());
    }
    values
}

fn cwd_from(args: &[String]) -> CliResult<String> {
    match option_value(args, "--cwd") {
        Some(cwd) => Ok(cwd),
        None => std::env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .map_err(|error| {
                CliError::Other(format!("Could not read the current folder: {error}"))
            }),
    }
}

fn status(args: &[String]) -> CliResult<()> {
    let cwd = cwd_from(args)?;
    let accounts = github_accounts::global_accounts();
    let account = github_accounts::settled_account_for_cwd(&cwd);
    if cli_args_want_json(args) {
        print_json(&match &account {
            Some(account) => json!({
                "ok": true,
                "login": account.login,
                "reason": account.source.describe(),
                "repository": format!("{}/{}", account.repo.owner, account.repo.name),
                "worktree": account.repo.root,
                "accounts": account.accounts.logins,
            }),
            None => json!({ "ok": true, "login": null, "accounts": accounts.logins }),
        });
        return Ok(());
    }
    match account {
        Some(account) => println!(
            "{}/{} in {} uses {} ({}).",
            account.repo.owner,
            account.repo.name,
            account.repo.root.display(),
            account.login,
            account.source.describe()
        ),
        None => println!("{}", inactive_reason(&cwd, &accounts)),
    }
    Ok(())
}

fn inactive_reason(cwd: &str, accounts: &github_accounts::GhAccounts) -> String {
    if accounts.logins.len() < 2 {
        return "Ghostex chooses GitHub accounts once two or more accounts are signed in to gh. Add one with: gh auth login".to_string();
    }
    if github_accounts::repo_for_cwd(cwd).is_none() {
        return "This folder is not in a git checkout whose origin is on github.com, so gh and git use their usual login here.".to_string();
    }
    "Ghostex is not choosing a GitHub account here.".to_string()
}

fn list(args: &[String]) -> CliResult<()> {
    let accounts = github_accounts::global_accounts();
    let owners = github_accounts::owner_logins();
    if cli_args_want_json(args) {
        print_json(&json!({
            "ok": true,
            "accounts": accounts.logins,
            "ghActive": accounts.active,
            "owners": owners,
        }));
        return Ok(());
    }
    if accounts.logins.is_empty() {
        println!("No GitHub account is signed in to gh. Sign in with: gh auth login");
        return Ok(());
    }
    println!("GitHub accounts signed in to gh:");
    for login in &accounts.logins {
        let active = accounts
            .active
            .as_deref()
            .is_some_and(|active| active.eq_ignore_ascii_case(login));
        println!(
            "  {login}{}",
            if active {
                "  (gh's active account)"
            } else {
                ""
            }
        );
    }
    if !owners.is_empty() {
        println!("\nRemembered for repository owners:");
        for (owner, login) in &owners {
            println!("  {owner:<24} {login}");
        }
    }
    Ok(())
}

fn use_command(args: &[String]) -> CliResult<()> {
    let cwd = cwd_from(args)?;
    let login = positionals(args, &["--cwd"]).into_iter().next();
    let remember_owner = !args.iter().any(|arg| arg == "--this-worktree");
    switch_worktree(&cwd, login.as_deref(), remember_owner)
}

/// Changes the account of the worktree that holds `cwd`. Without a login it
/// moves to the next account, as `gh auth switch` does with two accounts.
fn switch_worktree(cwd: &str, login: Option<&str>, remember_owner: bool) -> CliResult<()> {
    let accounts = github_accounts::global_accounts();
    let Some(current) = github_accounts::account_for_cwd(cwd) else {
        return Err(CliError::Other(inactive_reason(cwd, &accounts)));
    };
    let target = match login {
        Some(login) => current.accounts.find(login).cloned().ok_or_else(|| {
            CliError::Other(format!(
                "{login} is not signed in to gh. Signed in: {}. Add it with: gh auth login",
                current.accounts.logins.join(", ")
            ))
        })?,
        None => {
            let logins = &current.accounts.logins;
            let index = logins
                .iter()
                .position(|login| login.eq_ignore_ascii_case(&current.login))
                .map(|index| (index + 1) % logins.len())
                .unwrap_or(0);
            logins[index].clone()
        }
    };
    github_accounts::write_worktree_login(&current.file, &target).map_err(|error| {
        CliError::Other(format!(
            "Could not save the worktree's GitHub account: {error}"
        ))
    })?;
    println!(
        "✓ Switched the GitHub account for this worktree ({}) to {target}",
        current.repo.root.display()
    );
    if remember_owner {
        github_accounts::set_owner_login(&current.repo.owner, &target).map_err(|error| {
            CliError::Other(format!(
                "Could not remember the account for {}: {error}",
                current.repo.owner
            ))
        })?;
        println!(
            "  New worktrees of {} repositories will use {target} too.",
            current.repo.owner
        );
    }
    Ok(())
}

/// `gh auth switch` run inside a Ghostex session, forwarded by the gh shim.
/// It understands gh's own flags; a switch for another host goes to the real gh.
fn gh_switch(args: &[String]) -> CliResult<()> {
    let user = option_value(args, "--user").or_else(|| option_value(args, "-u"));
    let host = option_value(args, "--hostname").or_else(|| option_value(args, "-h"));
    if host
        .as_deref()
        .is_some_and(|host| !host.eq_ignore_ascii_case(GITHUB_HOST))
    {
        return run_real_gh_switch(args);
    }
    let cwd = cwd_from(&[])?;
    switch_worktree(&cwd, user.as_deref(), true)
}

fn run_real_gh_switch(args: &[String]) -> CliResult<()> {
    let real_gh = std::env::var_os("GHOSTEX_GITHUB_REAL_GH")
        .map(std::path::PathBuf::from)
        .or_else(github_accounts::real_gh_path)
        .ok_or_else(|| CliError::Other("gh: command not found".to_string()))?;
    let status = std::process::Command::new(real_gh)
        .args(["auth", "switch"])
        .args(args)
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .status()
        .map_err(|error| CliError::Other(format!("Could not run gh: {error}")))?;
    crate::ghostex_cli::set_exit_code(status.code().unwrap_or(1));
    Ok(())
}

/// `gh` inside a native Windows session (`gh.cmd` and the Git Bash `gh` call
/// this): the same rules as the POSIX shim in `github_accounts::shim`. It runs
/// the real gh as the worktree's account, sends `gh auth switch` to
/// `gh-switch`, and leaves gh's own sign-in commands alone.
fn gh_shim(args: &[String]) -> CliResult<()> {
    let real_gh = github_accounts::real_gh_for_shim()
        .ok_or_else(|| CliError::Other("gh: command not found".to_string()))?;
    let login = std::env::var_os("GHOSTEX_GITHUB_ACCOUNT_FILE")
        .and_then(|file| github_accounts::read_worktree_login(std::path::Path::new(&file)));
    let Some(login) = login else {
        return run_gh(&real_gh, args, GhToken::Inherit);
    };
    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("auth"), Some("switch")) => {
            std::env::set_var("GHOSTEX_GITHUB_REAL_GH", &real_gh);
            gh_switch(&args[2..])
        }
        (Some("auth"), Some("login" | "logout" | "refresh" | "setup-git")) => {
            run_gh(&real_gh, args, GhToken::Remove)
        }
        _ => match github_accounts::token_for(&login) {
            Some(token) => run_gh(&real_gh, args, GhToken::Set(token)),
            None => {
                eprintln!("Ghostex: the GitHub account {login} is not signed in to gh, so gh uses its active account. Sign it in with: gh auth login");
                run_gh(&real_gh, args, GhToken::Inherit)
            }
        },
    }
}

enum GhToken {
    Inherit,
    Remove,
    Set(String),
}

fn run_gh(real_gh: &std::path::Path, args: &[String], token: GhToken) -> CliResult<()> {
    let mut command = std::process::Command::new(real_gh);
    command.args(args);
    match token {
        GhToken::Inherit => {}
        GhToken::Remove => {
            command.env_remove("GH_TOKEN").env_remove("GITHUB_TOKEN");
        }
        GhToken::Set(token) => {
            command.env("GH_TOKEN", token);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(CliError::Other(format!("Could not run gh: {error}")))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| CliError::Other(format!("Could not run gh: {error}")))?;
        crate::ghostex_cli::set_exit_code(status.code().unwrap_or(1));
        Ok(())
    }
}
