use std::path::{Path, PathBuf};

use serde_json::json;

use crate::ghostex_cli::output::{cli_args_want_json, print_json};
use crate::ghostex_cli::rpc::{CliError, CliResult};
use crate::github_accounts;

const USAGE: &str = "Ghostex GitHub accounts - which of your gh accounts each worktree uses

Ghostex picks the account by itself when more than one account is signed in to
gh (`gh auth login`) on the repository's GitHub host (github.com or a GitHub
Enterprise host). Inside a Ghostex session, `gh auth switch` switches the
current worktree only.

Usage:
  ghostex github-account status [--cwd <folder>] [--json]
      Show the account the folder's worktree uses and why
  ghostex github-account list [--json]
      List your gh accounts, their commit identity and SSH key, and the account
      remembered for each owner
  ghostex github-account use [<login>] [--cwd <folder>] [--this-worktree]
      Switch the worktree to <login> (no login: the next account), and remember
      it for the repository's owner unless --this-worktree is given
  ghostex github-account identity <login> [--name <name>] [--email <email>]
      [--signing-key <key id or SSH key file>] [--signing-format openpgp|ssh] [--host <host>]
      Show or set the name, email and signing key commits made as <login> use
  ghostex github-account repos [<search words>] [--refresh] [--json]
      List the repositories every signed-in account can reach, with the
      accounts that reach each one (the most recently pushed first)
  ghostex github-account write-config [--cwd <folder>]
      Write the worktree's account into its own git config, so git uses it
      outside Ghostex sessions too (other terminals, editors, agentbox)
  ghostex github-account ssh-key <login> [<key file>] [--host <host>]
      Show or set the SSH key that signs in as <login>; with no key file and
      no key saved, Ghostex looks for it among your SSH keys";

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
        "identity" => identity_command(rest),
        "ssh-key" => ssh_key_command(rest),
        "repos" => repos_command(rest),
        "write-config" => {
            let cwd = cwd_from(rest)?;
            if github_accounts::write_worktree_git_config(&cwd) {
                println!("✓ This worktree's own git config now uses its GitHub account, in every terminal and editor.");
                Ok(())
            } else {
                Err(CliError::Other(inactive_reason(&cwd)))
            }
        }
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

fn host_from(args: &[String]) -> String {
    option_value(args, "--host")
        .map(|host| host.to_ascii_lowercase())
        .unwrap_or_else(|| github_accounts::GITHUB_HOST.to_string())
}

fn status(args: &[String]) -> CliResult<()> {
    let cwd = cwd_from(args)?;
    let account = github_accounts::settled_account_for_cwd(&cwd);
    if cli_args_want_json(args) {
        print_json(&match &account {
            Some(account) => json!({
                "ok": true,
                "login": account.login,
                "host": account.repo.remote.host,
                "reason": account.source.describe(),
                "source": account.source.as_wire(),
                "repository": format!("{}/{}", account.repo.remote.owner, account.repo.remote.name),
                "worktree": account.repo.root,
                "accounts": account.accounts.logins,
            }),
            None => json!({
                "ok": true,
                "login": null,
                "accounts": github_accounts::global_accounts().logins,
                "reason": inactive_reason(&cwd),
            }),
        });
        return Ok(());
    }
    match account {
        Some(account) => println!(
            "{}/{} in {} uses {} on {} ({}).",
            account.repo.remote.owner,
            account.repo.remote.name,
            account.repo.root.display(),
            account.login,
            account.repo.remote.host,
            account.source.describe()
        ),
        None => println!("{}", inactive_reason(&cwd)),
    }
    Ok(())
}

fn inactive_reason(cwd: &str) -> String {
    let Some(worktree) = github_accounts::github_worktree_for_cwd(cwd) else {
        if github_accounts::repo_for_cwd(cwd).is_some() {
            return "No account is signed in to gh for this repository's host. Sign in with: gh auth login".to_string();
        }
        return "This folder is not in a git checkout whose origin is on a GitHub host, so gh and git use their usual login here.".to_string();
    };
    format!(
        "Ghostex chooses GitHub accounts once two or more accounts are signed in to gh on {}. Add one with: gh auth login --hostname {}",
        worktree.repo.remote.host, worktree.repo.remote.host
    )
}

fn list(args: &[String]) -> CliResult<()> {
    let hosts = github_accounts::global_hosts();
    let owners = github_accounts::owner_logins();
    let describe = |host: &str, login: &str| {
        let identity = github_accounts::read_identity(host, login).map(|(identity, custom)| {
            json!({ "name": identity.name, "email": identity.email, "custom": custom })
        });
        let key = github_accounts::account_key(host, login);
        (identity, key)
    };
    if cli_args_want_json(args) {
        let hosts_json: serde_json::Map<String, serde_json::Value> = hosts
            .iter()
            .map(|(host, accounts)| {
                let logins: Vec<serde_json::Value> = accounts
                    .logins
                    .iter()
                    .map(|login| {
                        let (identity, key) = describe(host, login);
                        json!({ "login": login, "identity": identity, "sshKey": key })
                    })
                    .collect();
                (
                    host.clone(),
                    json!({ "accounts": logins, "ghActive": accounts.active }),
                )
            })
            .collect();
        print_json(&json!({ "ok": true, "hosts": hosts_json, "owners": owners }));
        return Ok(());
    }
    if hosts.is_empty() {
        println!("No GitHub account is signed in to gh. Sign in with: gh auth login");
        return Ok(());
    }
    for (host, accounts) in &hosts {
        println!("GitHub accounts signed in to gh on {host}:");
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
            let (identity, key) = describe(host, login);
            if let Some(identity) = identity {
                println!(
                    "      commits as {} <{}>",
                    identity["name"].as_str().unwrap_or_default(),
                    identity["email"].as_str().unwrap_or_default()
                );
            }
            if let Some(key) = key {
                println!("      SSH key {}", key.display());
            }
        }
    }
    if !owners.is_empty() {
        println!("\nRemembered for repository owners:");
        for (owner, login) in &owners {
            println!("  {owner:<32} {login}");
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
    let Some(current) = github_accounts::account_for_cwd(cwd) else {
        return Err(CliError::Other(inactive_reason(cwd)));
    };
    let target = match login {
        Some(login) => login.to_string(),
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
    let account = github_accounts::choose_account_for_cwd(cwd, &target, remember_owner)
        .map_err(CliError::Other)?;
    println!(
        "✓ Switched the GitHub account for this worktree ({}) to {}",
        account.repo.root.display(),
        account.login
    );
    if remember_owner {
        println!(
            "  New worktrees of {} repositories will use {} too.",
            account.repo.remote.owner, account.login
        );
    }
    Ok(())
}

fn identity_command(args: &[String]) -> CliResult<()> {
    let host = host_from(args);
    let login = positionals(
        args,
        &[
            "--name",
            "--email",
            "--host",
            "--signing-key",
            "--signing-format",
        ],
    )
    .into_iter()
    .next()
    .ok_or_else(|| CliError::Other(USAGE.to_string()))?;
    let name = option_value(args, "--name");
    let email = option_value(args, "--email");
    let signing_key = option_value(args, "--signing-key");
    let signing_format = option_value(args, "--signing-format");
    if name.is_none() && email.is_none() && signing_key.is_none() && signing_format.is_none() {
        match github_accounts::read_identity(&host, &login) {
            Some((identity, custom)) => println!(
                "{login} commits as {} <{}>{}{}",
                identity.name,
                identity.email,
                identity
                    .signing_key
                    .as_deref()
                    .map(|key| format!(", signed with {key}"))
                    .unwrap_or_default(),
                if custom { " (set by you)" } else { "" }
            ),
            None => match github_accounts::fetch_identity(&host, &login) {
                Some(identity) => {
                    let identity = github_accounts::with_auto_signing(&host, &login, identity);
                    let _ = github_accounts::write_identity(&host, &login, &identity, false);
                    println!("{login} commits as {} <{}>", identity.name, identity.email);
                }
                None => println!("Ghostex could not read {login}'s name and email from GitHub. Set them with --name and --email."),
            },
        }
        return Ok(());
    }
    let current = github_accounts::read_identity(&host, &login).map(|(identity, _)| identity);
    let identity = github_accounts::Identity {
        name: name
            .or_else(|| current.as_ref().map(|identity| identity.name.clone()))
            .unwrap_or_else(|| login.clone()),
        email: email
            .or_else(|| current.as_ref().map(|identity| identity.email.clone()))
            .ok_or_else(|| CliError::Other("Pass --email as well.".to_string()))?,
        signing_key: signing_key
            .filter(|key| !key.is_empty() && key != "none")
            .or_else(|| {
                current
                    .as_ref()
                    .and_then(|identity| identity.signing_key.clone())
            }),
        signing_format: signing_format
            .filter(|format| !format.is_empty() && format != "none")
            .or_else(|| {
                current
                    .as_ref()
                    .and_then(|identity| identity.signing_format.clone())
            }),
    };
    let identity = github_accounts::with_auto_signing(&host, &login, identity);
    github_accounts::write_identity(&host, &login, &identity, true)
        .map_err(|error| CliError::Other(format!("Could not save the identity: {error}")))?;
    println!(
        "✓ Commits made as {login} use {} <{}>",
        identity.name, identity.email
    );
    Ok(())
}

fn ssh_key_command(args: &[String]) -> CliResult<()> {
    let host = host_from(args);
    let mut values = positionals(args, &["--host"]).into_iter();
    let login = values
        .next()
        .ok_or_else(|| CliError::Other(USAGE.to_string()))?;
    if let Some(key) = values.next() {
        let key = PathBuf::from(key);
        if !key.is_file() {
            return Err(CliError::Other(format!("{} is not a file.", key.display())));
        }
        github_accounts::set_account_key(&host, &login, &key)
            .map_err(|error| CliError::Other(format!("Could not save the SSH key: {error}")))?;
        println!(
            "✓ SSH connections to {host} as {login} use {}",
            key.display()
        );
        return Ok(());
    }
    if let Some(key) = github_accounts::account_key(&host, &login) {
        println!("{login} on {host} signs in with {}", key.display());
        return Ok(());
    }
    let accounts = github_accounts::global_hosts()
        .remove(&host)
        .unwrap_or_default();
    github_accounts::discover_keys(&host, &accounts);
    match github_accounts::account_key(&host, &login) {
        Some(key) => println!("✓ Found the SSH key for {login}: {}", key.display()),
        None => println!("None of your SSH keys signs in to {host} as {login}. Pass one: ghostex github-account ssh-key {login} <key file>"),
    }
    Ok(())
}

/// `gh auth switch` run inside a Ghostex session, forwarded by the gh shim.
/// It understands gh's own flags; a switch for another host goes to the real gh.
fn gh_switch(args: &[String]) -> CliResult<()> {
    let user = option_value(args, "--user").or_else(|| option_value(args, "-u"));
    let host = option_value(args, "--hostname").or_else(|| option_value(args, "-h"));
    let cwd = cwd_from(&[])?;
    let worktree_host =
        github_accounts::github_worktree_for_cwd(&cwd).map(|worktree| worktree.repo.remote.host);
    if host
        .as_deref()
        .is_some_and(|host| Some(host.to_ascii_lowercase()) != worktree_host)
    {
        return run_real_gh_switch(args);
    }
    switch_worktree(&cwd, user.as_deref(), true)
}

fn run_real_gh_switch(args: &[String]) -> CliResult<()> {
    let real_gh = std::env::var_os("GHOSTEX_GITHUB_REAL_GH")
        .map(PathBuf::from)
        .or_else(github_accounts::real_gh_path)
        .ok_or_else(|| CliError::Other("gh: command not found".to_string()))?;
    let mut command = std::process::Command::new(real_gh);
    command.args(["auth", "switch"]).args(args);
    remove_token_env(&mut command);
    let status = command
        .status()
        .map_err(|error| CliError::Other(format!("Could not run gh: {error}")))?;
    crate::ghostex_cli::set_exit_code(status.code().unwrap_or(1));
    Ok(())
}

fn remove_token_env(command: &mut std::process::Command) {
    for key in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(key);
    }
}

/// `gh` inside a native Windows session (`gh.cmd` and the Git Bash `gh` call
/// this): the same rules as the POSIX shim in `github_accounts::shim`. It runs
/// the real gh as the worktree's account, sends `gh auth switch` to
/// `gh-switch`, and leaves gh's own sign-in commands alone.
fn gh_shim(args: &[String]) -> CliResult<()> {
    let real_gh = github_accounts::real_gh_for_shim()
        .ok_or_else(|| CliError::Other("gh: command not found".to_string()))?;
    let record = std::env::var_os("GHOSTEX_GITHUB_ACCOUNT_FILE")
        .and_then(|file| github_accounts::read_worktree_record(Path::new(&file)));
    let Some(record) = record else {
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
        _ => match github_accounts::token_for(&record.host, &record.login) {
            Some(token) => run_gh(
                &real_gh,
                args,
                GhToken::Set(github_accounts::token_env_key(&record.host), token),
            ),
            None => {
                eprintln!("Ghostex: the GitHub account {} is not signed in to gh, so gh uses its active account. Sign it in with: gh auth login", record.login);
                run_gh(&real_gh, args, GhToken::Inherit)
            }
        },
    }
}

enum GhToken {
    Inherit,
    Remove,
    Set(&'static str, String),
}

fn run_gh(real_gh: &Path, args: &[String], token: GhToken) -> CliResult<()> {
    let mut command = std::process::Command::new(real_gh);
    command.args(args);
    match token {
        GhToken::Inherit => {}
        GhToken::Remove => remove_token_env(&mut command),
        GhToken::Set(key, token) => {
            command.env(key, token);
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

fn repos_command(args: &[String]) -> CliResult<()> {
    let query = positionals(args, &[]).join(" ");
    let force = args.iter().any(|arg| arg == "--refresh");
    let repositories = github_accounts::search_account_repositories(&query, 1000, force);
    if cli_args_want_json(args) {
        print_json(&json!({
            "ok": true,
            "repositories": repositories.iter().map(|repository| repository.to_json()).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if repositories.is_empty() {
        println!("No repositories found. Sign accounts in with: gh auth login");
        return Ok(());
    }
    for repository in &repositories {
        let host = if repository.host == github_accounts::GITHUB_HOST {
            String::new()
        } else {
            format!("{}/", repository.host)
        };
        println!(
            "{host}{:<48} {}{}",
            repository.name_with_owner,
            repository.accounts.join(", "),
            if repository.private {
                "  (private)"
            } else {
                ""
            }
        );
    }
    Ok(())
}
