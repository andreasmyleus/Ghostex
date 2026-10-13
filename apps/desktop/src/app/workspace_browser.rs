//! Which Browser profile a workspace's tabs use, and the wait for its disk-backed context.
use crate::cef::WorkspaceBrowserContextState;
use crate::*;
use futures::FutureExt as _;

const WORKSPACE_BROWSER_CONTEXT_WAIT: Duration = Duration::from_secs(10);

thread_local! {
    static WORKSPACE_BROWSER_CONTEXT_WAITS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// CDXC:Browser 2026-10-09 DECISION:
/// User: each workspace has its own browser sign-ins (cookies), so a user can be signed in to github.com or linear.app as their work account in the Work workspace and as themselves in Personal. A workspace's Browser profile id is `workspace-<workspaceId>`; the default (Personal) workspace, passed as `None`, keeps today's Default profile so existing sign-ins stay where they are. Ids that are not already lowercase letters, digits and dashes (or too long for a cache folder name) are hashed so two workspaces never share a folder.
pub(crate) fn workspace_browser_profile(workspace_id: Option<&str>) -> String {
    let Some(workspace_id) = workspace_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return BrowserProfileId::default_profile().cef_profile_string();
    };
    let profile = format!("workspace-{workspace_id}");
    if cef::cef_profile_cache_segment(&profile).is_some() {
        return profile;
    }
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(workspace_id.as_bytes()));
    format!("workspace-h{}", &digest[..32])
}

/// CDXC:Browser 2026-10-09 WHY:
/// Only the Default Browser profile follows the workspace. Generated "Profile N" ids are app-global and memory-backed, and stay that way: they are reused across windows and workspaces, so making them disk-backed would carry one workspace's sign-ins into another's "Profile 2".
pub(crate) fn browser_cef_profile_for_workspace(
    profile_id: BrowserProfileId,
    workspace_id: Option<&str>,
) -> String {
    if profile_id == BrowserProfileId::default_profile() {
        workspace_browser_profile(workspace_id)
    } else {
        profile_id.cef_profile_string()
    }
}

/// The Browser profile a project whose worktree runs as GitHub account `login` on `host` uses in
/// a workspace (`None` = the default one): `workspace[-<id>]-gh-<login>`, disk-backed and cleared
/// with the workspace's sign-ins, hashed like `workspace_browser_profile` when it would not be a
/// valid folder name.
///
/// CDXC:GitHubAccounts 2026-10-13 DECISION: The user asked for the in-app browser to follow the worktree's GitHub account too, so a PR opened from a project is shown signed in as that project's account. Each account gets its own Browser profile per workspace (signed in once per account); projects without an account choice keep the workspace's profile, so nothing changes for one gh account.
pub(crate) fn github_account_browser_profile(
    workspace_id: Option<&str>,
    host: &str,
    login: &str,
) -> String {
    let workspace = workspace_id.map(str::trim).filter(|id| !id.is_empty());
    let mut profile = match workspace {
        Some(id) => format!("workspace-{id}-gh-"),
        None => "workspace-gh-".to_string(),
    };
    if !host.eq_ignore_ascii_case("github.com") {
        profile.push_str(&host.to_ascii_lowercase().replace('.', "-"));
        profile.push('-');
    }
    profile.push_str(&login.to_ascii_lowercase());
    if cef::cef_profile_cache_segment(&profile).is_some() {
        return profile;
    }
    use sha2::{Digest, Sha256};
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{}\u{1f}{host}\u{1f}{login}", workspace.unwrap_or("")).as_bytes())
    );
    format!("workspace-g{}", &digest[..32])
}

/// Signs a workspace's Browser profile out of everything (see `cef::clear_workspace_browser_profile`).
/// The Workspaces settings page's "Sign out of all sites" (`clearWorkspaceBrowserSignins`).
pub(crate) fn clear_workspace_browser_signins(workspace_id: &str) -> Result<(), String> {
    let profile = workspace_browser_profile(Some(workspace_id));
    if !cef::cef_profile_is_workspace(&profile) {
        return Err("The default workspace uses the Default Browser profile".into());
    }
    cef::clear_workspace_browser_profile(&profile).map_err(|error| error.to_string())
}

/// Starts a workspace profile's context with no page on it, for work that needs only the context
/// (Settings' Forget all answers), and resolves true once it is ready. False for other profiles,
/// when it fails or takes too long, and when the CEF runtime is not running: that stays deferred
/// until a web view needs it (CDXC:CefRuntime 2026-09-19).
pub(crate) fn start_workspace_browser_context(
    profile: &str,
    cx: &App,
) -> impl std::future::Future<Output = bool> + 'static {
    let state = (cef::cef_profile_is_workspace(profile) && cef::context_initialized())
        .then(|| cef::prepare_workspace_browser_context(profile).ok())
        .flatten();
    let timer = cx
        .background_executor()
        .timer(WORKSPACE_BROWSER_CONTEXT_WAIT);
    async move {
        let receiver = match state {
            Some(WorkspaceBrowserContextState::Ready) => return true,
            Some(WorkspaceBrowserContextState::Pending(receiver)) => receiver,
            _ => return false,
        };
        let timer = timer.fuse();
        let mut receiver = receiver.fuse();
        futures::pin_mut!(timer);
        futures::select! {
            result = receiver => result.unwrap_or(false),
            _ = timer => false,
        }
    }
}

impl GhostexGpuiApp {
    /// The CEF profile a Browser tab or website view opens with in this window.
    pub(crate) fn browser_tab_cef_profile(&self, profile_id: BrowserProfileId) -> String {
        // The window's workspace (`None` = the default workspace, gx_store/workspaces.rs).
        let workspace_id = self.gx_store_window_non_default_workspace_id();
        if profile_id == BrowserProfileId::default_profile() {
            if let Some((host, login)) = self.active_project_github_account() {
                return github_account_browser_profile(workspace_id.as_deref(), &host, &login);
            }
        }
        browser_cef_profile_for_workspace(profile_id, workspace_id.as_deref())
    }

    /// `(host, login)` of the GitHub account the active project's worktree runs as, from the
    /// sidebar's project context (absent unless gh has several accounts on its host).
    fn active_project_github_account(&self) -> Option<(String, String)> {
        let project_id = self.active_project_id_for_view_scope()?;
        let snapshot = self.native_sidebar.snapshot.as_ref()?;
        snapshot.groups.iter().find_map(|group| {
            let context = group.project_context.as_ref()?;
            if context.pointer("/editor/projectId")?.as_str()? != project_id {
                return None;
            }
            let account = context.get("githubAccount")?;
            Some((
                account
                    .get("host")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("github.com")
                    .to_string(),
                account.get("login")?.as_str()?.to_string(),
            ))
        })
    }

    /// Clears the per-GitHub-account Browser profiles of a workspace for every account the
    /// sidebar's projects can use, with its "Sign out of all sites".
    pub(crate) fn clear_workspace_github_browser_profiles(&self, workspace_id: &str) {
        let Some(snapshot) = self.native_sidebar.snapshot.as_ref() else {
            return;
        };
        let mut profiles: Vec<String> = snapshot
            .groups
            .iter()
            .filter_map(|group| group.project_context.as_ref()?.get("githubAccount"))
            .flat_map(|account| {
                let host = account
                    .get("host")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("github.com")
                    .to_string();
                account
                    .get("accounts")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(move |login| {
                        login
                            .as_str()
                            .map(|login| github_account_browser_profile(Some(workspace_id), &host, login))
                    })
            })
            .collect();
        profiles.sort();
        profiles.dedup();
        for profile in profiles {
            let _ = cef::clear_workspace_browser_profile(&profile);
        }
    }

    /// True when `profile` can create a browser now. A workspace profile whose
    /// disk-backed context is still initializing returns false and re-runs the
    /// Browser and website-view reconcile when CEF reports it initialized.
    pub(crate) fn prepare_workspace_browser_context(
        &mut self,
        profile: &str,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !cef::cef_profile_is_workspace(profile) {
            return true;
        }
        let receiver = match cef::prepare_workspace_browser_context(profile) {
            Ok(WorkspaceBrowserContextState::Ready) => return true,
            Ok(WorkspaceBrowserContextState::RuntimeNotReady) => return false,
            Ok(WorkspaceBrowserContextState::Pending(receiver)) => receiver,
            Err(error) => {
                support_logs::append(
                    support_logs::GpuiSupportLog::CrashReports,
                    "gpui.workspaceBrowserContext.failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return false;
            }
        };
        if !WORKSPACE_BROWSER_CONTEXT_WAITS
            .with(|waits| waits.borrow_mut().insert(profile.to_string()))
        {
            return false;
        }
        let profile = profile.to_string();
        cx.spawn(async move |this, cx| {
            let timer = cx
                .background_executor()
                .timer(WORKSPACE_BROWSER_CONTEXT_WAIT)
                .fuse();
            let mut receiver = receiver.fuse();
            futures::pin_mut!(timer);
            let outcome = futures::select! {
                result = receiver => Some(result.unwrap_or(false)),
                _ = timer => None,
            };
            WORKSPACE_BROWSER_CONTEXT_WAITS.with(|waits| waits.borrow_mut().remove(&profile));
            if outcome != Some(true) {
                // Not ready in time or failed: the next reconcile asks again and
                // waits on the same context; no memory-backed context is used.
                support_logs::append(
                    support_logs::GpuiSupportLog::CrashReports,
                    "gpui.workspaceBrowserContext.notReady",
                    serde_json::json!({ "timedOut": outcome.is_none() }),
                );
                return;
            }
            let _ = this.update_in(cx, |app, window, cx| {
                app.sync_active_browser_tab_to_surface(window, cx);
                app.ensure_project_workarea_runtime_cef_surfaces_for_current_context(cx);
                cx.notify();
            });
        })
        .detach();
        false
    }
}
