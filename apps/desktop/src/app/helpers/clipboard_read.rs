//! Clipboard reads for paste that never block the UI thread, and the slow-read diagnostic.
//!
//! CDXC:Clipboard 2026-10-11 DECISION: User: Ctrl+V sometimes froze the sidebar and the chat on Windows; every Windows paste path reads the clipboard in the background, and a read slower than 200 ms is logged with the formats involved under the Chat diagnostic logs area, only while that area is on. Windows' `GetClipboardData` waits for the source app to render a format it published on demand (Office pictures, Remote Desktop, screenshot tools), up to the system's 30 second timeout when that app is busy, so a read on the UI thread froze every window. Text fields read through GPUI Kit's `Paste` (gpui-base `InputBaseState::paste`), which already reads in the background; only the app's own reads come here. The log names entry kinds and sizes, never clipboard contents.

use std::future::Future;
use std::time::Duration;

use gpui::{App, ClipboardEntry, ClipboardItem};
use web_time::Instant;

use crate::support_logs::{GpuiSupportLog, append};

const SLOW_CLIPBOARD_READ: Duration = Duration::from_millis(200);

/// Reads the clipboard off the UI thread (a ready read where the platform's own read is quick)
/// and resolves on the foreground. `source` names the paste path in the slow-read log.
pub(crate) fn gpui_read_clipboard(
    cx: &App,
    source: &'static str,
) -> impl Future<Output = Option<ClipboardItem>> + 'static {
    let started = Instant::now();
    let read = cx.read_from_clipboard_async();
    async move {
        let item = match read.await {
            Ok(item) => item,
            Err(error) => {
                eprintln!("Could not read the clipboard for {source}: {error}");
                None
            }
        };
        gpui_note_clipboard_read(source, started, item.as_ref());
        item
    }
}

/// Logs a paste whose clipboard read took longer than 200 ms, for reads made elsewhere (the chat
/// composer's field reads through GPUI Kit and hands its paste hook the result).
pub(crate) fn gpui_note_clipboard_read(
    source: &'static str,
    started: Instant,
    item: Option<&ClipboardItem>,
) {
    let elapsed = started.elapsed();
    if elapsed < SLOW_CLIPBOARD_READ {
        return;
    }
    append(
        GpuiSupportLog::SessionChat,
        "clipboard.slowRead",
        serde_json::json!({
            "source": source,
            "elapsedMs": elapsed.as_millis() as u64,
            "formats": item.map(clipboard_formats).unwrap_or_default(),
        }),
    );
}

/// What the clipboard held, by kind and size only.
fn clipboard_formats(item: &ClipboardItem) -> Vec<String> {
    item.entries()
        .iter()
        .map(|entry| match entry {
            ClipboardEntry::String(string) => format!("text ({} bytes)", string.text().len()),
            ClipboardEntry::Image(image) => {
                format!("image {:?} ({} bytes)", image.format, image.bytes.len())
            }
            ClipboardEntry::ExternalPaths(paths) => format!("files ({})", paths.paths().len()),
        })
        .collect()
}
