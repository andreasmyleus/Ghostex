use std::sync::Arc;

use axum::{body::Body, http::StatusCode};
use serde_json::{json, Value};

use crate::server::{AppState, RoutedResponse};

use super::{
    answers::{reply, ANSWERS_BODY_MAX_BYTES},
    serve::VISUAL_PAGE_ROUTE_PREFIX,
    store::{is_page_id, store_draft, StoreAnswersError},
};

pub(crate) const VISUAL_PAGE_DRAFT_SUFFIX: &str = "/draft";

/// Whether `POST <path>` is a page's draft route, `/visual/<id>/draft`.
pub(crate) fn is_draft_route(path: &str) -> bool {
    path.starts_with(VISUAL_PAGE_ROUTE_PREFIX) && path.ends_with(VISUAL_PAGE_DRAFT_SUFFIX)
}

/// `POST /visual/<id>/draft` `{"draft": <any JSON>}`: what the reader has picked and written so far,
/// saved by the page as it changes and handed back when the page opens again (serve.rs).
///
/// CDXC:SessionChat 2026-10-11 DECISION:
/// User: never lose what the reader picked or wrote in a question page before sending; closing the floating window and opening it again, a reload or a restart brings it all back, without the agent writing anything for it.
/// CDXC:SessionChat 2026-10-11 WHY:
/// A published page has an opaque origin, so it has no storage of its own; the draft lives on the page's record in gxserver, answered before the auth gate like the answers route and limited to that page by its id and content policy.
pub(crate) async fn save(state: Arc<AppState>, path: String, body: Body) -> RoutedResponse {
    let Some(id) = path
        .strip_prefix(VISUAL_PAGE_ROUTE_PREFIX)
        .and_then(|rest| rest.strip_suffix(VISUAL_PAGE_DRAFT_SUFFIX))
        .filter(|id| is_page_id(id))
        .map(str::to_string)
    else {
        return reply(
            StatusCode::NOT_FOUND,
            json!({"ok": false, "error": "No such page."}),
        );
    };
    let Ok(bytes) = axum::body::to_bytes(body, ANSWERS_BODY_MAX_BYTES).await else {
        return reply(
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({"ok": false, "error": "The draft is larger than 64 KiB."}),
        );
    };
    let draft = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(mut body)) if body.contains_key("draft") => {
            body.remove("draft").unwrap_or(Value::Null)
        }
        _ => {
            return reply(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": "Send {\"draft\": …} as JSON."}),
            )
        }
    };
    let state_dir = state.paths.root_dir.clone();
    let stored = tokio::task::spawn_blocking(move || store_draft(&state_dir, &id, &draft)).await;
    match stored {
        Ok(Ok(())) => reply(StatusCode::OK, json!({"ok": true})),
        Ok(Err(StoreAnswersError::NotFound)) => reply(
            StatusCode::NOT_FOUND,
            json!({"ok": false, "error": "No such page."}),
        ),
        Ok(Err(StoreAnswersError::AlreadyAnswered(answered_at))) => reply(
            StatusCode::CONFLICT,
            json!({"ok": false, "error": "These answers were already sent.", "answeredAt": answered_at}),
        ),
        Ok(Err(StoreAnswersError::Io(error))) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"ok": false, "error": format!("The draft could not be saved: {error}")}),
        ),
        Err(_) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"ok": false, "error": "The draft could not be saved."}),
        ),
    }
}
