//! A chat message sent to a session whose agent is asleep or still starting.

use axum::http::StatusCode;
use serde_json::{json, Map, Value};

use crate::domain::{DomainRepository, DomainStateError};
use crate::protocol::rpc_success;
use crate::server::{domain_error_response, routed_json, AppState, RoutedResponse};
use crate::session_chat_options::SessionChatTerminalDetection;
use crate::session_chat_send::SessionChatSendTarget;
use crate::storage::open_gxserver_database;

/// Refusal code the composer send raises for a session that is not up yet; the HTTP handler turns
/// it into a delivery that waits for the agent instead of an error.
pub(crate) const SESSION_CHAT_SESSION_STARTING: &str = "sessionStarting";

/// The first line the restore script prints before it runs the agent's resume command
/// (`wrap_restored_terminal_resume_command` in agents/helpers.rs).
const RESTORE_BANNER: &str = "Restoring session...";

/// CDXC:SessionChat 2026-09-25 DECISION:
/// User: pressing Enter in the chat must always send, even when the session "wasn't attached, or it wasn't awake". Sends that found no zmx daemon, or found the agent still printing its restore banner, came back as "input box is not accepting input" or "could not be cleared". Now the send wakes a sleeping session itself, and a session that is still starting takes the message as a startup send: the chat shows it at once and the queue types it the moment the input box appears, as a new chat already does (2026-09-09 decision).
/// WHY: only a daemon that `zmx` reports missing is woken. A screen counts as starting when it shows the restore banner or nothing at all with no input box (`screen_shows_agent_starting`); since 2026-10-11 any other missing input box is held too (`holds_until_agent_takes_input`), because the queue types only into an input box it has seen, so a dialog cannot eat the message.
pub(crate) async fn starting_session_refusal(
    state: &AppState,
    target: &SessionChatSendTarget,
    detection: &SessionChatTerminalDetection,
) -> Option<DomainStateError> {
    // A message already held for this session goes first; typing the next one now would put it
    // ahead of the one the user sent earlier.
    if startup_send_pending(state, target) {
        return Some(starting(
            "An earlier message is waiting for the agent to start.",
        ));
    }
    if detection.captured {
        let booting = detection.composer.is_not_ready()
            && !detection.composer.should_dismiss()
            && detection.notice.is_none()
            && screen_shows_agent_starting(&detection.composer.screen_tail);
        if !booting {
            return None;
        }
        crate::session_chat_send_diagnostics::record_send_recovery(
            state,
            "sessionChatSendHeldForStartingAgent",
            &target.project_id,
            &target.session_id,
            detection.composer.reason.as_deref().unwrap_or_default(),
            &detection.composer.screen_tail,
        );
        return Some(starting("The agent is still starting."));
    }
    // No screen could be read: find out whether the daemon is gone before waking anything.
    if !provider_missing(state, target).await {
        return None;
    }
    let body = json!({
        "params": { "projectId": target.project_id, "sessionId": target.session_id }
    });
    let woke = crate::server::handle_zmx_lifecycle_http(
        state,
        "/api/wakeSession".to_string(),
        uuid::Uuid::new_v4().to_string(),
        &body,
    )
    .await
    .response
    .status()
    .is_success();
    crate::session_chat_send_diagnostics::record_send_recovery(
        state,
        if woke {
            "sessionChatSendWokeSession"
        } else {
            "sessionChatSendWakeFailed"
        },
        &target.project_id,
        &target.session_id,
        "The session's zmx daemon was not running when the message was sent.",
        &[],
    );
    Some(if woke {
        starting("The session was asleep and is starting now.")
    } else {
        DomainStateError {
            code: "dependencyUnavailable",
            message: "The session is asleep and could not be woken, so nothing was sent."
                .to_string(),
        }
    })
}

/// CDXC:SessionChat 2026-10-11 WHY:
/// On a Windows machine whose every new PowerShell hung for minutes, a new session's screen stayed blank while the shell loaded, and the send read that as "the agent is gone": it refused the message and tried to restart the session, which would have killed the shell that was still starting. Nothing on screen yet is what a starting agent looks like (an exited agent leaves its shell prompt or its exit text), so a blank screen counts as starting, like the restore banner. `screen_tail` holds only non-blank lines.
pub(crate) fn screen_shows_agent_starting(screen_tail: &[String]) -> bool {
    screen_tail.is_empty() || screen_tail.iter().any(|line| line.trim() == RESTORE_BANNER)
}

/// CDXC:SessionChat 2026-10-11 DECISION:
/// User: "due to the lag due to M drive when i just started a session and tried to send chat message there it goes in chat transcript then back to composer after few seconds. this shouldn't actually happen we should just keep it in chat and keep trying every X seconds until it does get posted. imagine the user sent it then went away. we need to self heal as i said many times before!! … doesn't matter the reason we need to have best self heal for all situations". A chat message refused because the agent's input box is not there yet (still starting, a shell still loading, a resume, a repaint, a panel) is never handed back to the composer: gxserver keeps it as a startup send in the session's queue, the chat draws it in the transcript as waiting, and the queue scheduler types it the moment the input box appears, retrying every 2 s and backing off to every 15 s for as long as the session lives (`session_chat_queue_undelivered.rs`). It survives closed clients and gxserver restarts because the queue is in SQLite. It ends up Not sent (with its reason, Retry and Remove) only when retrying cannot help: a question or dialog only the user can answer, or a refusal that is not about the input box. Pictures wait with the text (session_chat_queue_attachments.rs).
/// SEE-ALSO: packages/gx-chat-core/src/session/startup_sends.rs draws the held row in the transcript; apps/desktop/src/app/native_chat/startup_delivery.rs and message_actions.rs, and apps/mobile/app/src/chat/native/transcript/MessageActions.tsx, show its waiting state with Edit and Delete.
pub(crate) fn holds_until_agent_takes_input(error: &DomainStateError) -> bool {
    error.code == "composerNotReady"
        && !error.message.ends_with("Answer it in chat before sending.")
}

fn startup_send_pending(state: &AppState, target: &SessionChatSendTarget) -> bool {
    open_gxserver_database(&state.paths).is_ok_and(|db| {
        crate::session_chat_queue::read_session_chat_queue_snapshot_with(
            &db,
            &target.project_id,
            &target.session_id,
        )
        .queue
        .iter()
        .any(|prompt| prompt.startup_send && prompt.state != "failed")
    })
}

fn starting(message: &str) -> DomainStateError {
    DomainStateError {
        code: SESSION_CHAT_SESSION_STARTING,
        message: message.to_string(),
    }
}

async fn provider_missing(state: &AppState, target: &SessionChatSendTarget) -> bool {
    let paths = state.paths.clone();
    let server_id = state.metadata.server_id.clone();
    let lifecycle = crate::zmx::LifecycleParams {
        project_id: target.project_id.clone(),
        session_id: target.session_id.clone(),
    };
    tokio::task::spawn_blocking(move || {
        let db = open_gxserver_database(&paths).ok()?;
        let repository = DomainRepository::new(&db, server_id.as_str());
        crate::zmx::probe_and_cache_session_provider(&repository, &lifecycle)
            .ok()
            .map(|(probe, ..)| probe.lifecycle_state == "missing")
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// CDXC:SessionTitles 2026-09-27 WHY:
/// An agent rename is typed into the agent's own input box, and a sleeping session has none: the direct write to its missing zmx daemon was lost after the new title was already saved, so the agent's older title replaced it on the next wake. A command for a session whose daemon is gone waits in the queue as a startup send and wakes the session, the same way a chat message sent to a sleeping session does (2026-09-25 decision above).
pub(crate) fn queue_startup_command_and_wake(
    state: &AppState,
    project_id: &str,
    session_id: &str,
    command: &str,
) -> Result<(), DomainStateError> {
    let mut params = Map::new();
    params.insert("projectId".to_string(), json!(project_id));
    params.insert("sessionId".to_string(), json!(session_id));
    params.insert("text".to_string(), json!(command));
    params.insert("startupSend".to_string(), json!(true));
    crate::session_chat_queue::handle_session_chat_queue_endpoint(
        &state.paths,
        state.metadata.server_id.as_str(),
        "/api/queueSessionChatPrompt",
        &params,
    )?;
    crate::session_chat_queue_runtime::broadcast_session_chat_queue_state(
        state, project_id, session_id,
    );
    let state = state.clone();
    let project_id = project_id.to_string();
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        let body = json!({ "params": { "projectId": project_id, "sessionId": session_id } });
        let woke = crate::server::handle_zmx_lifecycle_http(
            &state,
            "/api/wakeSession".to_string(),
            uuid::Uuid::new_v4().to_string(),
            &body,
        )
        .await
        .response
        .status()
        .is_success();
        crate::session_chat_send_diagnostics::record_send_recovery(
            &state,
            if woke {
                "sessionChatSendWokeSession"
            } else {
                "sessionChatSendWakeFailed"
            },
            &project_id,
            &session_id,
            "The session's zmx daemon was not running when a command was queued for it.",
            &[],
        );
    });
    Ok(())
}

/// CDXC:Drafts 2026-10-06 WHY:
/// A draft's agent is started by the client that opens it (the desktop's attach, the phone's background attach), so a draft created with `ghostex create-agent --defer-start` that no client opened had no zmx daemon, and a chat message sent to it waited in the queue forever. The first message to such a draft wakes it, as a send to a sleeping session does (2026-09-25 decision); a draft whose daemon is already up is left alone.
pub(crate) fn start_draft_agent_if_missing(state: &AppState, target: &SessionChatSendTarget) {
    let state = state.clone();
    let target = SessionChatSendTarget {
        project_id: target.project_id.clone(),
        session_id: target.session_id.clone(),
        zmx_name: target.zmx_name.clone(),
        session: Value::Null,
    };
    tokio::spawn(async move {
        if !provider_missing(&state, &target).await {
            return;
        }
        let body = json!({
            "params": { "projectId": target.project_id, "sessionId": target.session_id }
        });
        let woke = crate::server::handle_zmx_lifecycle_http(
            &state,
            "/api/wakeSession".to_string(),
            uuid::Uuid::new_v4().to_string(),
            &body,
        )
        .await
        .response
        .status()
        .is_success();
        crate::session_chat_send_diagnostics::record_send_recovery(
            &state,
            if woke {
                "sessionChatSendWokeSession"
            } else {
                "sessionChatSendWakeFailed"
            },
            &target.project_id,
            &target.session_id,
            "The draft's agent was not running when its first message was queued.",
            &[],
        );
    });
}

/// A send accepted now and typed by the queue once the agent's input box appears; the chat draws
/// it in the transcript, not in the queue strip. The row keeps the send's `imagePaths` too.
pub(crate) fn queue_startup_send(
    state: &AppState,
    endpoint_path: String,
    request_id: String,
    params: &Map<String, Value>,
    target: &SessionChatSendTarget,
    text: &str,
) -> RoutedResponse {
    let mut startup_params = params.clone();
    startup_params.insert("startupSend".to_string(), json!(true));
    match crate::session_chat_queue::handle_session_chat_queue_endpoint(
        &state.paths,
        state.metadata.server_id.as_str(),
        "/api/queueSessionChatPrompt",
        &startup_params,
    ) {
        Ok(result) => {
            crate::session_chat_queue_runtime::broadcast_session_chat_queue_state(
                state,
                &target.project_id,
                &target.session_id,
            );
            routed_json(
                Some(endpoint_path),
                StatusCode::OK,
                rpc_success(
                    request_id,
                    json!({
                        "queued": true,
                        "textBytes": text.len(),
                        "queuedPromptId": result.value.pointer("/prompt/id"),
                    }),
                ),
            )
        }
        Err(error) => domain_error_response(endpoint_path, request_id, error),
    }
}
