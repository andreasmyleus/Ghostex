use super::*;

/// `/api/setProjectGithubAccount` `{projectId, login, rememberOwner?}`: the
/// sidebar's GitHub Account menu. Switches the project's worktree to `login`
/// (and, unless `rememberOwner` is false, remembers it for the repository's
/// owner), then republishes the project so the menu's check mark moves.
pub(crate) async fn handle_set_project_github_account_http(
    state: &Arc<AppState>,
    endpoint_path: String,
    request_id: String,
    body: &Value,
) -> RoutedResponse {
    let params = match read_domain_rpc_params(body) {
        Ok(params) => params,
        Err(error) => return domain_error_response(endpoint_path, request_id, error),
    };
    let owned = state.clone();
    let result = tokio::task::spawn_blocking(move || set_project_github_account(&owned, &params))
        .await
        .unwrap_or_else(|error| {
            Err(DomainStateError {
                code: "internalError",
                message: format!("Switching the GitHub account stopped: {error}"),
            })
        });
    match result {
        Ok(result) => routed_json(
            Some(endpoint_path),
            StatusCode::OK,
            rpc_success(request_id, result),
        ),
        Err(error) => domain_error_response(endpoint_path, request_id, error),
    }
}

fn set_project_github_account(
    state: &Arc<AppState>,
    params: &Map<String, Value>,
) -> std::result::Result<Value, DomainStateError> {
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| DomainStateError::bad_request(format!("Pass {key}.")))
    };
    let project_id = text("projectId")?;
    let login = text("login")?;
    let remember_owner = params
        .get("rememberOwner")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let path = {
        let db = open_gxserver_database(&state.paths).map_err(|error| DomainStateError {
            code: "internalError",
            message: format!("SQLite gxserver state error: {error}"),
        })?;
        let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
        repository
            .get_project(&project_id)?
            .and_then(|project| {
                project
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .ok_or_else(|| DomainStateError::bad_request("No project has that id."))?
    };
    let account = crate::github_accounts::choose_account_for_cwd(&path, &login, remember_owner)
        .map_err(DomainStateError::bad_request)?;
    refresh_project_github_account_paths(state, Some(std::slice::from_ref(&path)))?;
    Ok(serde_json::json!({
        "projectId": project_id,
        "login": account.login,
        "accounts": account.accounts.logins,
        "owner": account.repo.remote.owner,
        "rememberedForOwner": remember_owner,
    }))
}
