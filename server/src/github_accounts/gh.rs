use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::run::capture;

pub(crate) const GITHUB_HOST: &str = "github.com";

const GH_TOKEN_TIMEOUT: Duration = Duration::from_secs(5);
const TOKEN_CACHE_TTL: Duration = Duration::from_secs(60);

/// Environment variables that make `gh` ignore its own logins. gxserver removes
/// them when it asks gh for an account's token so the answer comes from gh's
/// store, not from a token the daemon happened to inherit.
pub(crate) const GH_TOKEN_ENV_KEYS: [&str; 4] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

/// The accounts signed in to the user's own gh (`gh auth login`) on one host,
/// sorted, and the one gh currently treats as active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GhAccounts {
    pub(crate) logins: Vec<String>,
    pub(crate) active: Option<String>,
}

impl GhAccounts {
    pub(crate) fn find(&self, login: &str) -> Option<&String> {
        self.logins
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(login))
    }
}

/// The variable gh reads a token from for `host`: `GH_TOKEN` on github.com,
/// `GH_ENTERPRISE_TOKEN` on GitHub Enterprise hosts.
pub(crate) fn token_env_key(host: &str) -> &'static str {
    if host.eq_ignore_ascii_case(GITHUB_HOST) {
        "GH_TOKEN"
    } else {
        "GH_ENTERPRISE_TOKEN"
    }
}

/// gh's own config folder, resolved the way gh resolves it.
fn global_gh_config_dir() -> Option<PathBuf> {
    if let Some(dir) = nonempty_env_path("GH_CONFIG_DIR") {
        return Some(dir);
    }
    if let Some(dir) = nonempty_env_path("XDG_CONFIG_HOME") {
        return Some(dir.join("gh"));
    }
    #[cfg(windows)]
    if let Some(dir) = nonempty_env_path("AppData") {
        return Some(dir.join("GitHub CLI"));
    }
    Some(
        ghostex_paths::GhostexPaths::resolve()
            .home_dir
            .join(".config")
            .join("gh"),
    )
}

fn nonempty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Every host in gh's `hosts.yml` (github.com and GitHub Enterprise hosts,
/// lowercased) with its accounts. No network, no subprocess.
///
/// CDXC:GitHubAccounts 2026-10-13 WHY: A daemon that was handed `GH_TOKEN`/`GITHUB_TOKEN` (or `GH_ENTERPRISE_TOKEN` for Enterprise hosts) already has one fixed identity chosen by the user, and every gh call it makes would use that token whatever account Ghostex picked, so those hosts are left out instead of pretending to switch.
pub(crate) fn global_hosts() -> BTreeMap<String, GhAccounts> {
    let env_set = |keys: &[&str]| {
        keys.iter()
            .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    };
    let dotcom_fixed = env_set(&["GH_TOKEN", "GITHUB_TOKEN"]);
    let enterprise_fixed = env_set(&["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]);
    let mut hosts = global_gh_config_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("hosts.yml")).ok())
        .map(|text| parse_hosts_yml(&text))
        .unwrap_or_default();
    hosts.retain(|host, _| {
        if host == GITHUB_HOST {
            !dotcom_fixed
        } else {
            !enterprise_fixed
        }
    });
    hosts
}

/// The accounts signed in to gh on `host`.
pub(crate) fn accounts_for_host(host: &str) -> GhAccounts {
    global_hosts()
        .remove(&host.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The accounts signed in to gh on github.com.
pub(crate) fn global_accounts() -> GhAccounts {
    accounts_for_host(GITHUB_HOST)
}

/// The `users` and the active `user` of every host in gh's `hosts.yml`. gh
/// writes the file itself in a small fixed shape, so this reads indentation
/// instead of pulling in a YAML parser. A pre-2.40 file has only `user:`, which
/// counts as a single account.
pub(crate) fn parse_hosts_yml(text: &str) -> BTreeMap<String, GhAccounts> {
    let mut hosts: BTreeMap<String, GhAccounts> = BTreeMap::new();
    let mut host: Option<String> = None;
    let mut child_indent = None;
    let mut in_users = false;
    let mut user_indent = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        let content = line.trim_start();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - content.len();
        let Some((key, value)) = content.split_once(':') else {
            continue;
        };
        let key = unquote(key.trim());
        let value = unquote(value.trim());
        if indent == 0 {
            let name = key.to_ascii_lowercase();
            hosts.entry(name.clone()).or_default();
            host = Some(name);
            child_indent = None;
            in_users = false;
            continue;
        }
        let Some(accounts) = host.as_ref().and_then(|host| hosts.get_mut(host)) else {
            continue;
        };
        let child = *child_indent.get_or_insert(indent);
        if indent <= child {
            in_users = key == "users";
            user_indent = None;
            if key == "user" && !value.is_empty() {
                accounts.active = Some(value.to_string());
            }
            continue;
        }
        if in_users {
            let users = *user_indent.get_or_insert(indent);
            if indent == users && !key.is_empty() {
                accounts.logins.push(key.to_string());
            }
        }
    }
    for accounts in hosts.values_mut() {
        if let Some(active) = accounts.active.clone() {
            if accounts.find(&active).is_none() {
                accounts.logins.push(active);
            }
        }
        accounts
            .logins
            .sort_by_key(|login| login.to_ascii_lowercase());
        accounts.logins.dedup();
    }
    hosts.retain(|_, accounts| !accounts.logins.is_empty());
    hosts
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value)
}

/// The real `gh` executable on the daemon's job PATH (which includes the copy
/// Ghostex installs), never the account shim.
pub(crate) fn real_gh_path() -> Option<PathBuf> {
    let shim_dir = super::shim::shim_dir();
    let executable = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::split_paths(&crate::managed_tools::run::job_path(&[]))
        .filter(|dir| {
            !dir.as_os_str().is_empty() && *dir != shim_dir && !super::shim::is_shim_dir(dir)
        })
        .map(|dir| dir.join(executable))
        .find(|candidate| candidate.is_file())
}

/// The real gh for `ghostex github-account gh-shim`: the first one on this
/// process's PATH that is not the shim, then the one the session was started
/// with, then the daemon's.
pub(crate) fn real_gh_for_shim() -> Option<PathBuf> {
    let executable = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .filter(|dir| !dir.as_os_str().is_empty() && !super::shim::is_shim_dir(dir))
        .map(|dir| dir.join(executable))
        .find(|candidate| candidate.is_file())
        .or_else(|| {
            std::env::var_os("GHOSTEX_GITHUB_REAL_GH")
                .map(PathBuf::from)
                .filter(|path| path.is_file())
        })
        .or_else(real_gh_path)
}

/// A `gh` command that answers from the user's own gh store: no inherited
/// token or host, no update notice, no prompt.
pub(crate) fn gh_store_command(real_gh: &PathBuf) -> Command {
    let mut command = Command::new(real_gh);
    for key in GH_TOKEN_ENV_KEYS {
        command.env_remove(key);
    }
    command.env_remove("GH_HOST");
    command
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1");
    command
}

/// `gh auth token --user <login>`: the token gh already holds for that account.
/// Nothing is written or stored by Ghostex; the answer is kept in memory for a
/// minute so periodic status probes do not start a gh process each time.
pub(crate) fn token_for(host: &str, login: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{host}/{login}").to_ascii_lowercase();
    if let Ok(cache) = cache.lock() {
        if let Some((at, token)) = cache.get(&key) {
            if at.elapsed() < TOKEN_CACHE_TTL {
                return Some(token.clone());
            }
        }
    }
    let real_gh = real_gh_path()?;
    let mut command = gh_store_command(&real_gh);
    command.args(["auth", "token", "--hostname", host, "--user", login]);
    let token = capture(command, GH_TOKEN_TIMEOUT).filter(|token| !token.is_empty())?;
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, (Instant::now(), token.clone()));
    }
    Some(token)
}
