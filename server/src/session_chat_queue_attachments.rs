//! The pictures a queued chat message carries. The queue keeps its own copy of each one, so the
//! message can wait for the agent as long as it takes even when the original was a temporary file,
//! and drops the copies once the row is gone.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::{domain::DomainStateError, paths::GxserverPaths};

/// How long the copies of a delivered row are kept: the agent reads a pasted picture from disk,
/// and some read it only when the turn starts.
const DELIVERED_IMAGES_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// The largest picture a queued row may carry, the size a chat read serves.
const QUEUED_IMAGE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// CDXC:SessionChat 2026-10-11 WHY:
/// A send with pictures used to wait 40 seconds for a starting agent and then refuse, because a queued row held only text (CDXC:SessionChat 2026-10-11 DECISION in session_chat_send_wake.rs: "best self heal for all situations"). The row now carries its pictures, copied under `<app data>/i/queued/<promptId>/`, so a picture send waits like a text send. Files and folders attached in the chat are `[File #N](path)` references inside the text and already waited with it.
fn folder(paths: &GxserverPaths, prompt_id: &str) -> PathBuf {
    paths.app_data_dir.join("i").join("queued").join(prompt_id)
}

/// The `imagePaths` a queue call carries.
pub(crate) fn requested(params: &Map<String, Value>) -> Vec<String> {
    params
        .get("imagePaths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Copies the pictures into the row's folder and answers the copies' paths, in order.
pub(crate) fn store(
    paths: &GxserverPaths,
    prompt_id: &str,
    image_paths: &[String],
) -> Result<Vec<String>, DomainStateError> {
    if image_paths.is_empty() {
        return Ok(Vec::new());
    }
    let directory = folder(paths, prompt_id);
    let copy = || -> Result<Vec<String>, DomainStateError> {
        std::fs::create_dir_all(&directory)
            .map_err(|_| internal("Could not create the folder for the message's pictures."))?;
        image_paths
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let source = Path::new(source);
                let metadata = std::fs::metadata(source)
                    .ok()
                    .filter(|metadata| metadata.is_file() && source.is_absolute())
                    .ok_or_else(|| {
                        DomainStateError::bad_request(format!(
                            "The picture {} is not a file on this machine.",
                            source.display()
                        ))
                    })?;
                if metadata.len() > QUEUED_IMAGE_MAX_BYTES {
                    return Err(DomainStateError::bad_request(format!(
                        "The picture {} is larger than 64 MB.",
                        source.display()
                    )));
                }
                let name = source
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image.png".to_string());
                let target = directory.join(format!("{}-{name}", index + 1));
                std::fs::copy(source, &target)
                    .map_err(|_| internal("Could not keep a copy of the message's picture."))?;
                Ok(target.to_string_lossy().to_string())
            })
            .collect()
    };
    copy().inspect_err(|_| remove(paths, prompt_id))
}

/// The column value for `image_paths`: NULL for a text-only row.
pub(crate) fn encode(image_paths: &[String]) -> Option<String> {
    (!image_paths.is_empty()).then(|| serde_json::to_string(image_paths).unwrap_or_default())
}

pub(crate) fn decode(column: Option<String>) -> Vec<String> {
    column
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

/// Drops the row's copies at once: a row the user deleted never reached the agent.
pub(crate) fn remove(paths: &GxserverPaths, prompt_id: &str) {
    let _ = std::fs::remove_dir_all(folder(paths, prompt_id));
}

/// Starts the delivered row's grace hour: the marker file moves the folder's modified time to now.
pub(crate) fn mark_delivered(paths: &GxserverPaths, prompt_id: &str) {
    let directory = folder(paths, prompt_id);
    if directory.is_dir() {
        let _ = std::fs::write(directory.join(".delivered"), b"");
    }
}

/// Drops the copies of rows that are gone (delivered, or removed while gxserver was down) once
/// they are an hour old, so the agent has read them.
pub(crate) fn sweep(paths: &GxserverPaths, db: &Connection) {
    let Ok(entries) = std::fs::read_dir(paths.app_data_dir.join("i").join("queued")) else {
        return;
    };
    for entry in entries.flatten() {
        let prompt_id = entry.file_name().to_string_lossy().to_string();
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= DELIVERED_IMAGES_GRACE);
        let queued = db
            .query_row(
                "SELECT 1 FROM session_chat_queued_prompts WHERE promptId = ?1",
                [&prompt_id],
                |_| Ok(()),
            )
            .is_ok();
        if old && !queued {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn internal(message: &str) -> DomainStateError {
    DomainStateError {
        code: "internalError",
        message: message.to_string(),
    }
}
