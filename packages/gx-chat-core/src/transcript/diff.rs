//! Diff lines: the shape a file-change card counts and paints, and the two extractors that read
//! them out of a tool call or a patch.
//!
//! Ported from `packages/shared/session-chat-presentation/diff.ts`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::transcript::jsstr::{utf16_len, utf16_take};

pub const MAX_DIFF_CHARS: usize = 32_000;
pub const DEFAULT_MAX_DIFF_LINES: usize = 120;

/// What one line of a diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffKind {
    Add,
    Del,
    Meta,
    Context,
}

/// One line of a diff, as both renderers paint it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
    /// The line's number in the file before the edit, when the agent recorded where it landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<u32>,
    /// The line's number in the file after the edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
}

impl DiffLine {
    pub fn new(kind: DiffKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            old: None,
            new: None,
        }
    }

    pub fn numbered(mut self, old: Option<u32>, new: Option<u32>) -> Self {
        self.old = old;
        self.new = new;
        self
    }
}

fn truncated_line() -> DiffLine {
    DiffLine::new(DiffKind::Meta, "\u{2026} diff truncated \u{2026}")
}

/// `value.slice(0, MAX_DIFF_CHARS).split('\n', maxLines + 1)` plus the truncation test.
fn to_lines(value: Option<&str>, max_lines: usize) -> (Vec<String>, bool) {
    let Some(value) = value else {
        return (Vec::new(), false);
    };
    let clipped = utf16_take(value, MAX_DIFF_CHARS);
    // `String.prototype.split` with a limit stops filling the array; it does not join the rest.
    let lines: Vec<&str> = clipped.split('\n').take(max_lines + 1).collect();
    let truncated = utf16_len(value) > MAX_DIFF_CHARS || lines.len() > max_lines;
    let mut bounded: Vec<String> = lines
        .into_iter()
        .take(max_lines)
        .map(str::to_string)
        .collect();
    if !truncated && bounded.last().is_some_and(String::is_empty) {
        bounded.pop();
    }
    (bounded, truncated)
}

fn string_field<'a>(record: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let entries = record.as_object()?;
    for key in keys {
        match entries.get(*key) {
            Some(Value::Null) | None => continue,
            Some(value) => return value.as_str(),
        }
    }
    None
}

/// The tool names whose arguments already hold the before and after of an edit.
pub fn is_edit_tool_name(name: &str) -> bool {
    matches!(
        name,
        "Edit" | "MultiEdit" | "Write" | "str_replace" | "apply_patch"
    )
}

/// The diff a tool call's own arguments describe, before any result comes back.
pub fn diff_from_tool_call(name: &str, input: &Value, max_lines: usize) -> Option<Vec<DiffLine>> {
    if !is_edit_tool_name(name) || !input.is_object() {
        return None;
    }
    let old_value = string_field(input, &["old_string", "oldString", "old"]);
    let new_value = string_field(
        input,
        &["new_string", "newString", "new", "content", "file_text"],
    );
    let (old_lines, old_truncated) = to_lines(old_value, max_lines);
    let (new_lines, new_truncated) = to_lines(new_value, max_lines);
    if old_lines.is_empty() && new_lines.is_empty() {
        return None;
    }
    let path = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str);
    // ALL dels first, then ALL adds.
    let mut combined: Vec<DiffLine> = path
        .map(|path| DiffLine::new(DiffKind::Meta, path))
        .into_iter()
        .collect();
    combined.extend(
        old_lines
            .into_iter()
            .map(|text| DiffLine::new(DiffKind::Del, text)),
    );
    combined.extend(
        new_lines
            .into_iter()
            .map(|text| DiffLine::new(DiffKind::Add, text)),
    );
    let truncated = old_truncated || new_truncated || combined.len() > max_lines;
    if truncated {
        combined.truncate(max_lines - 1);
        combined.push(truncated_line());
    }
    Some(combined)
}

/// `^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?: .*)?\r?$`.
fn parse_hunk_header(line: &str) -> Option<(u64, u64)> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let rest = line.strip_prefix("@@ -")?;
    let (old_count, rest) = parse_hunk_range(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_count, rest) = parse_hunk_range(rest)?;
    let rest = rest.strip_prefix(" @@")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((old_count, new_count))
}

fn parse_hunk_range(value: &str) -> Option<(u64, &str)> {
    let start = value.bytes().take_while(u8::is_ascii_digit).count();
    if start == 0 {
        return None;
    }
    let rest = &value[start..];
    let Some(after_comma) = rest.strip_prefix(',') else {
        return Some((1, rest));
    };
    let digits = after_comma.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    Some((after_comma[..digits].parse().ok()?, &after_comma[digits..]))
}

/// CDXC:SessionChat 2026-09-18 WHY:
/// Counting leading plus/minus lines mistook Markdown bullets in command output for file changes.
/// Require unified hunk headers and honor their line counts so surrounding output stays uncolored.
pub fn diff_from_text(text: &str, max_lines: usize) -> Option<Vec<DiffLine>> {
    if text.is_empty() {
        return None;
    }
    let (bounded, truncated) = to_lines(Some(text), max_lines);
    let mut old_remaining = 0u64;
    let mut new_remaining = 0u64;
    let mut has_changes = false;
    let mut lines: Vec<DiffLine> = Vec::with_capacity(bounded.len());
    for line in bounded {
        if let Some((old_count, new_count)) = parse_hunk_header(&line) {
            old_remaining = old_count;
            new_remaining = new_count;
            lines.push(DiffLine::new(DiffKind::Meta, line));
            continue;
        }
        if old_remaining > 0 || new_remaining > 0 {
            if line.starts_with('+') && new_remaining > 0 {
                new_remaining -= 1;
                has_changes = true;
                lines.push(DiffLine::new(DiffKind::Add, &line[1..]));
                continue;
            }
            if line.starts_with('-') && old_remaining > 0 {
                old_remaining -= 1;
                has_changes = true;
                lines.push(DiffLine::new(DiffKind::Del, &line[1..]));
                continue;
            }
            if line.starts_with(' ') && old_remaining > 0 && new_remaining > 0 {
                old_remaining -= 1;
                new_remaining -= 1;
                lines.push(DiffLine::new(DiffKind::Context, line));
                continue;
            }
            if line.strip_suffix('\r').unwrap_or(&line) == "\\ No newline at end of file" {
                lines.push(DiffLine::new(DiffKind::Meta, line));
                continue;
            }
            old_remaining = 0;
            new_remaining = 0;
        }
        let meta = ["diff --git ", "index ", "--- ", "+++ "]
            .iter()
            .any(|prefix| line.starts_with(prefix));
        lines.push(DiffLine::new(
            if meta {
                DiffKind::Meta
            } else {
                DiffKind::Context
            },
            line,
        ));
    }
    if !has_changes {
        return None;
    }
    if truncated {
        lines.truncate(max_lines - 1);
        lines.push(truncated_line());
    }
    Some(lines)
}
