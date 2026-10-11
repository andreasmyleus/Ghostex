//! What a card for a message between agents says: one another agent sent this session, one this
//! session's agent sent another, and a subagent's reply. Each card is one `card` object (on
//! `interAgentMessage`, on a `sentMessages` entry, or on an `agent-message` system card) that the
//! renderers lay out as is.
//!
//! CDXC:SessionChat 2026-10-12 DECISION:
//! User: the morning's "proposed" agent message cards (a 240px card with a quote box and a footer) were "too large now ... make them smaller like before. i dont like the new style". From the mockup page (docs/2026-10-11/agent-message-cards/smaller-received-cards.html) the user picked D, one line: every message between agents (received, sent, and a subagent's reply alike) is a single row with the agent's own logo (a small arrow badge: prose grey coming in, sky going out), its name ("Codex", "To Claude", "Explore subagent"), one line of text, then the time, the status and a chevron; clicking the row opens the whole message under it. Go to session shows on hover (on the phone, inside the opened message). Delivered is a small check; Queued, Waiting for agent and Not sent say so in their colour. Kept from the first round: "please show the icons of the different agents", "instead of Open api refactor just say 'Go to session'", and no purple for incoming messages ("use color usually used for incoming chat messages"). Supersedes the 2026-10-11 wording that showed the message's first line collapsed and "from <session>" once opened: "keep the text when the message from x agent the same when it's collapsed as the one we show when it's expanded", so `line` is the one text the row shows in both states ("from <session title>", "in <session title>" on a sent row, the first line only when the session's title is not known).
//! SEE-ALSO: agent_message.rs and sent_message.rs read the messages; apps/desktop/src/app/native_chat/inter_agent_message.rs and apps/mobile/app/src/chat/native/transcript/AgentMessageCards.tsx draw the card.

use serde_json::{json, Value};

use crate::extras::agents::{agent_icon_id, default_agent_by_id};
use crate::transcript::jsstr::js_trim;

/// The jump button's label, whichever session it opens.
pub const GO_TO_SESSION: &str = "Go to session";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(js_trim)
        .unwrap_or_default()
}

/// The agent's artwork id: the agent id the header or the send's result named, else its name.
fn agent_icon(agent_id: &str, agent_name: &str) -> Value {
    default_agent_by_id(Some(agent_id))
        .or_else(|| default_agent_by_id(Some(agent_name)))
        .map(|(icon, _)| icon)
        .or_else(|| agent_icon_id(Some(agent_name)))
        .map_or(Value::Null, Value::from)
}

/// `label` is what the row says (only the icon when `iconOnly`), `hint` its tooltip; `tone` is
/// `good`, `waiting` or `failed`; `icon` is `check`, `clock` or `alert`.
fn status(label: &str, hint: &str, tone: &str, icon: &str) -> Value {
    json!({
        "label": label,
        "hint": hint,
        "tone": tone,
        "icon": icon,
        "iconOnly": tone == "good",
    })
}

/// The session a `<machine>:<project>:<session>` reference names, for the jump button.
fn session_target(reference: &str) -> Value {
    let reference = js_trim(reference);
    let parts: Vec<&str> = reference.split(':').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Value::Null;
    }
    json!({ "projectId": parts[1], "sessionId": parts[2], "label": GO_TO_SESSION })
}

/// The body's first line of text, which the row shows after the name.
fn first_line(body: &str) -> String {
    let line = body
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '>', '-', '*']).trim())
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How a row names the other session: "from <session>" on a received card, "in <session>" on a
/// sent one ("To Claude in <session>"). Empty when the session's title is not known.
fn relation(preposition: &str, session: &str) -> String {
    if session.is_empty() {
        String::new()
    } else {
        format!("{preposition} {session}")
    }
}

/// One card. `name` heads the row and `title` is the sentence an assistive reader hears; `detail`
/// names the other session (empty when its title is not known) and `preview` is the body's first
/// line. `line` is the one text the row shows after the name, collapsed and opened alike: `detail`,
/// else `preview`.
#[allow(clippy::too_many_arguments)]
fn card(
    direction: &str,
    agent_icon: Value,
    name: String,
    title: String,
    detail: String,
    body: &str,
    time: &str,
    status: Value,
    failure: &str,
    retry_prompt_id: Value,
    session: Value,
) -> Value {
    let preview = first_line(body);
    let line = if detail.is_empty() {
        preview.clone()
    } else {
        detail.clone()
    };
    json!({
        "direction": direction,
        "agentIcon": agent_icon,
        "name": name,
        "title": title,
        "detail": detail,
        "preview": preview,
        "line": line,
        "time": time,
        "status": status,
        "failure": failure,
        "retryPromptId": retry_prompt_id,
        "session": session,
    })
}

/// A subagent's reply: the sparkle in place of a logo.
pub fn subagent_card(name: &str, body: &str) -> Value {
    let name = js_trim(name);
    let label = if name.is_empty() || name.eq_ignore_ascii_case("subagent") {
        "Subagent".to_string()
    } else {
        format!("{name} subagent")
    };
    card(
        "subagent",
        Value::Null,
        label.clone(),
        format!("Reply from {label}"),
        String::new(),
        body,
        "",
        Value::Null,
        "",
        Value::Null,
        Value::Null,
    )
}

/// Writes `interAgentMessage.card` for a projected message from another agent. Run again whenever
/// the transcript names the sender (`sent_message::resolve_recipients`).
pub fn dress_received(message: &mut Value) {
    let inter = &message["interAgentMessage"];
    if !inter.is_object() {
        return;
    }
    if inter["subagent"] == true {
        let card = subagent_card(text(inter, "agentName"), text(inter, "body"));
        message["interAgentMessage"]["card"] = card;
        return;
    }
    let agent = match text(inter, "agentName") {
        "" => "An agent",
        name => name,
    };
    let delivery = &message["startupDelivery"];
    let failed = delivery.is_object() && delivery["state"] == "failed";
    let status = if failed {
        status("Not sent", "Not sent", "failed", "alert")
    } else if delivery.is_object() {
        status("Waiting for agent", "Waiting for agent", "waiting", "clock")
    } else if message["queued"] == true {
        status("Queued", "Queued until this turn ends", "waiting", "clock")
    } else {
        status("Delivered", "Delivered", "good", "check")
    };
    let failure = if !failed {
        ""
    } else {
        match text(delivery, "errorMessage") {
            "" => "Message could not be delivered.",
            error => error,
        }
    };
    let card = card(
        "received",
        agent_icon(text(inter, "agentId"), agent),
        agent.to_string(),
        format!("Message from {agent}"),
        relation("from", text(inter, "sessionTitle")),
        text(inter, "body"),
        text(&message["time"], "label"),
        status,
        failure,
        if failed {
            delivery["promptId"].clone()
        } else {
            Value::Null
        },
        session_target(text(inter, "replyTo")),
    );
    message["interAgentMessage"]["card"] = card;
}

/// Writes a sent message's `card`, once its recipient's name and session are known.
pub fn dress_sent(sent: &mut Value, agent: &str, session: &str) {
    let failed = sent["failed"] == true;
    let status = if failed {
        status("Not sent", "Not sent", "failed", "alert")
    } else {
        match text(sent, "status") {
            "delivered" => status("Delivered", "Delivered", "good", "check"),
            "queued" => status("Queued", "Queued until its turn ends", "waiting", "clock"),
            "pending" => status("Waiting for agent", "Waiting for agent", "waiting", "clock"),
            "accepted" => status("Sent", "Sent", "good", "check"),
            _ => Value::Null,
        }
    };
    let target = match session_target(text(sent, "globalRef")) {
        Value::Null => session_target(text(sent, "target")),
        target => target,
    };
    let failure = if failed { text(sent, "failure") } else { "" }.to_string();
    let body = text(sent, "body").to_string();
    sent["card"] = card(
        "sent",
        agent_icon(text(sent, "agentId"), agent),
        format!("To {agent}"),
        format!("Message to {agent}"),
        relation("in", session),
        &body,
        "",
        status,
        &failure,
        Value::Null,
        target,
    );
}
