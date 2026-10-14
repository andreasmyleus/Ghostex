use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use super::{gh, run::capture};

const PAGES: u32 = 3;
const PAGE_TIMEOUT: Duration = Duration::from_secs(20);
const LIST_TTL: Duration = Duration::from_secs(5 * 60);

/// One repository a signed-in account can reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountRepository {
    pub(crate) host: String,
    pub(crate) name_with_owner: String,
    pub(crate) url: String,
    pub(crate) ssh_url: String,
    pub(crate) private: bool,
    pub(crate) description: Option<String>,
    pub(crate) pushed_at: String,
    /// The accounts that can reach it, in name order.
    pub(crate) accounts: Vec<String>,
}

impl AccountRepository {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "nameWithOwner": self.name_with_owner,
            "url": self.url,
            "sshUrl": self.ssh_url,
            "private": self.private,
            "description": self.description,
            "pushedAt": self.pushed_at,
            "accounts": self.accounts,
        })
    }
}

/// The repositories every account signed in to gh can reach (its own, the ones it collaborates on
/// and its organizations'), the most recently pushed first, each with the accounts that reach it.
/// At most 300 per account; kept five minutes unless `force`. Blocking.
///
/// CDXC:GitHubAccounts 2026-10-13 DECISION: The user asked for searching and picking repositories to clone to cover every GitHub account, not only gh's active one, so Add Project's GitHub step and `ghostex github-account repos` list all accounts' repositories together and the clone then runs as the account that reaches the picked repository.
pub(crate) fn list_account_repositories(force: bool) -> Vec<AccountRepository> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Vec<AccountRepository>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if !force {
        if let Some((at, list)) = cache.lock().ok().and_then(|cache| cache.clone()) {
            if at.elapsed() < LIST_TTL {
                return list;
            }
        }
    }
    let Some(real_gh) = gh::real_gh_path() else {
        return Vec::new();
    };
    let mut merged: BTreeMap<String, AccountRepository> = BTreeMap::new();
    for (host, accounts) in gh::global_hosts() {
        for login in &accounts.logins {
            let Some(token) = gh::token_for(&host, login) else {
                continue;
            };
            for page in 1..=PAGES {
                let mut command = gh::gh_store_command(&real_gh);
                command.env(gh::token_env_key(&host), &token).args([
                    "api",
                    "--hostname",
                    &host,
                    &format!(
                        "user/repos?per_page=100&page={page}&sort=pushed&affiliation=owner,collaborator,organization_member"
                    ),
                ]);
                let Some(body) = capture(command, PAGE_TIMEOUT)
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                else {
                    break;
                };
                let items = body.as_array().cloned().unwrap_or_default();
                for item in &items {
                    let Some(name) = item.get("full_name").and_then(Value::as_str) else {
                        continue;
                    };
                    let text = |key: &str| {
                        item.get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    let entry = merged
                        .entry(format!("{host}/{name}").to_ascii_lowercase())
                        .or_insert_with(|| AccountRepository {
                            host: host.clone(),
                            name_with_owner: name.to_string(),
                            url: text("html_url"),
                            ssh_url: text("ssh_url"),
                            private: item.get("private").and_then(Value::as_bool) == Some(true),
                            description: item
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .filter(|text| !text.trim().is_empty()),
                            pushed_at: text("pushed_at"),
                            accounts: Vec::new(),
                        });
                    if !entry.accounts.iter().any(|known| known == login) {
                        entry.accounts.push(login.clone());
                    }
                }
                if items.len() < 100 {
                    break;
                }
            }
        }
    }
    let mut list: Vec<AccountRepository> = merged.into_values().collect();
    list.sort_by(|left, right| right.pushed_at.cmp(&left.pushed_at));
    if let Ok(mut cache) = cache.lock() {
        *cache = Some((Instant::now(), list.clone()));
    }
    list
}

/// `list_account_repositories` narrowed to the ones whose `owner/name` or description contains
/// every word of `query` (case-insensitive), at most `limit`.
pub(crate) fn search_account_repositories(
    query: &str,
    limit: usize,
    force: bool,
) -> Vec<AccountRepository> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    list_account_repositories(force)
        .into_iter()
        .filter(|repository| {
            let haystack = format!(
                "{} {}",
                repository.name_with_owner,
                repository.description.as_deref().unwrap_or_default()
            )
            .to_ascii_lowercase();
            words.iter().all(|word| haystack.contains(word))
        })
        .take(limit)
        .collect()
}
