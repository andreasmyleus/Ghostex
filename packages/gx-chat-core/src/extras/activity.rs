//! The terminal activity row and its local clock, ported from
//! `packages/shared/session-chat-controller/activity.ts`.
//!
//! The elapsed reading is the CLI's last sample plus the time since that sample was taken, so it
//! keeps counting smoothly across probes instead of snapping backwards on each one. `now` is
//! always [`ChatContext::now_ms`]; the core reads no clock.

use serde_json::{Map, Value};

use crate::extras::time::parse_iso_millis;
use crate::state::ChatContext;

/// How often the local clock re-publishes between server samples.
pub const ACTIVITY_CLOCK_TICK_MS: u64 = 1_000;

/// `formatSessionChatActivityElapsed`: `1h 2m 3s`, `2m 3s` or `3s`.
pub fn format_activity_elapsed(total_seconds: f64) -> String {
    let seconds = total_seconds.floor().max(0.0) as i64;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let rest = seconds % 60;
    if hours > 0 {
        return format!("{hours}h {minutes}m {rest}s");
    }
    if minutes > 0 {
        return format!("{minutes}m {rest}s");
    }
    format!("{rest}s")
}

/// `sessionChatActivityElapsedSeconds`: the seconds to show now, or `None` when the CLI painted
/// none.
///
/// Takes the two fields rather than the activity so the subagent strip can share it: its clocks
/// anchor on the fleet's `detectedAt` while the seconds come off each row.
pub fn activity_elapsed_seconds(
    elapsed_seconds: Option<f64>,
    detected_at: &str,
    now_ms: f64,
    utc_offset_minutes: i32,
) -> Option<f64> {
    let elapsed = elapsed_seconds?;
    let Some(anchor) = parse_iso_millis(detected_at, utc_offset_minutes) else {
        return Some(elapsed);
    };
    Some(elapsed + ((now_ms - anchor) / 1_000.0).max(0.0))
}

/// `computeSessionChatActivity`: the activity record with its elapsed label, clamped percentage
/// and the two kind flags the renderer draws off.
///
/// Returns `None` for no activity, which the document writes as `null`. The result spreads the
/// activity's own fields first, exactly as the TypeScript's `{...activity, ...}` does, so an
/// unmodelled field survives.
pub fn compute_activity(activity: Option<&Value>, context: &ChatContext) -> Option<Value> {
    compute_activity_at(activity, context.now_ms, context)
}

/// [`compute_activity`] at the clock the strip LATCHED (`const [now, setNow] = useState(...)`),
/// which only the effect and the one-second interval move.
pub fn compute_activity_at(
    activity: Option<&Value>,
    now_ms: f64,
    context: &ChatContext,
) -> Option<Value> {
    let activity = activity?;
    let record = activity.as_object().cloned().unwrap_or_default();
    let detected_at = record
        .get("detectedAt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let elapsed = activity_elapsed_seconds(
        record.get("elapsedSeconds").and_then(Value::as_f64),
        detected_at,
        now_ms,
        context.utc_offset_minutes,
    );
    let percent = record
        .get("percent")
        .and_then(Value::as_f64)
        .map(|value| js_round(value).clamp(0.0, 100.0) as i64);
    let kind = record
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut projected: Map<String, Value> = record.clone();
    projected.insert(
        "elapsedLabel".to_string(),
        match elapsed {
            Some(seconds) => Value::String(format_activity_elapsed(seconds)),
            None => Value::Null,
        },
    );
    projected.insert(
        "percent".to_string(),
        match percent {
            Some(value) => Value::from(value),
            None => Value::Null,
        },
    );
    projected.insert(
        "indeterminate".to_string(),
        Value::Bool(kind == "compacting" && percent.is_none()),
    );
    projected.insert(
        "shellsRunning".to_string(),
        Value::Bool(kind == "shells-running"),
    );
    projected.insert(
        "backgroundTasks".to_string(),
        crate::extras::background_tasks::background_tasks(
            activity,
            now_ms,
            context.utc_offset_minutes,
        )
        .unwrap_or(Value::Null),
    );
    projected.insert(
        "hint".to_string(),
        if kind == "compacting" {
            Value::String(
                "Send or queue a message and it will be posted after compaction".to_string(),
            )
        } else {
            Value::Null
        },
    );
    Some(Value::Object(projected))
}

/// `Math.round`: halves go towards positive infinity, not away from zero.
///
/// CDXC:SessionChat 2026-09-22 WHY:
/// This was `(value + 0.5).floor()`, which is not `Math.round`: the addition rounds first, so
/// `0.49999999999999994 + 0.5` is exactly `1.0` and the answer came back 1 where JavaScript
/// answers 0. `crate::jsnum::js_round` compares against the floor instead and never adds.
pub use crate::jsnum::js_round;
