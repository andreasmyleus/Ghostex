//! The project menu's GitHub Account rows: switch the worktree to another of the user's gh
//! accounts.
//!
//! SEE-ALSO: packages/gx-core/src/sidebar_menu/project.rs (the rows),
//! server/src/server/github_account_http.rs (the `/api/setProjectGithubAccount` answer),
//! server/src/github_accounts/ (what a worktree's account changes).

use serde_json::Value;

use super::rpc::gxserver_rpc_result_task;
use crate::GhostexGpuiApp;

impl GhostexGpuiApp {
    /// Answers the wrapped `setProjectGithubAccount`. The project's next presentation delta moves
    /// the menu's tick, so success shows a toast naming the account and nothing else.
    pub(crate) fn gx_store_run_github_account(
        &mut self,
        command: &Value,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if command.get("type").and_then(Value::as_str) != Some("command")
            || command["message"].get("type").and_then(Value::as_str)
                != Some("setProjectGithubAccount")
        {
            return false;
        }
        let message = &command["message"];
        let (Some(project_id), Some(login)) = (
            message.get("projectId").and_then(Value::as_str),
            message.get("login").and_then(Value::as_str),
        ) else {
            return true;
        };
        let params = serde_json::json!({ "projectId": project_id, "login": login });
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = gxserver_rpc_result_task(
                &background,
                "/api/setProjectGithubAccount",
                params,
                super::sidebar_lifecycle::rpc_timeout(),
            )
            .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(answer) => {
                    let login = answer
                        .get("login")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let owner = answer
                        .get("owner")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    this.dispatch_gpui_workspace_action_toast(
                        "success",
                        &format!("This worktree now uses {login}"),
                        &format!("gh and git here act as {login}, and new worktrees of {owner} will too."),
                        cx,
                    );
                }
                Err(message) => {
                    this.dispatch_gpui_workspace_action_toast(
                        "error",
                        "Couldn't switch the GitHub account",
                        &message,
                        cx,
                    );
                }
            });
        })
        .detach();
        true
    }
}
