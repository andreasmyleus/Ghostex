//! Windows: deleting the release artifacts cargo leaves behind in the start's target folders.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::Start;
use crate::util::root;

/// Written by build-windows-app.ps1 beside each release build: cargo's JSON messages for every unit that build used, fresh or rebuilt.
const UNITS_FILE: &str = "ghostex-build-units.jsonl";
const STAMP_FILE: &str = "ghostex-last-sweep";
const SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// Units the start did not use but another release build touched this recently (an agent's `cargo test --release`) are kept so they are not rebuilt every day.
const KEEP_RECENT: Duration = Duration::from_secs(3 * 24 * 3600);
const TARGETS: [&str; 4] = [
    "apps/desktop/target/release",
    "server/target/release",
    ".dependencies/wmx/target/release",
    "apps/editor/desktop/target/release",
];

impl Start {
    /// CDXC:Build 2026-10-11 DECISION:
    /// User asked for a Windows cleanup step in the start that sweeps stale release artifacts, behind a cheap check so an unchanged start stays fast.
    /// Cargo never deletes old units: every dependency bump, feature change or flag change leaves another copy in deps/ and build/, and the desktop's release folder had grown to 2,090 rlibs for 579 crates, with four 0.85 GB copies of the CEF build. At most once a day per target folder, and only when this start's build wrote the folder's unit list and no cargo holds its lock, delete every deps/ file and build/ folder whose unit hash that build did not use and nothing touched for three days.
    /// .fingerprint/ is left alone: a binary's fingerprint hash never appears in cargo's messages, and the folder is small.
    pub(super) fn sweep_stale_release_artifacts(&self, build_started: SystemTime) {
        let mut removed_files = 0usize;
        let mut removed_bytes = 0u64;
        for target in TARGETS {
            let release = root().join(target);
            if let Some((files, bytes)) = sweep_target(&release, build_started) {
                removed_files += files;
                removed_bytes += bytes;
            }
        }
        if removed_files > 0 {
            self.log.detail(&format!(
                "Removed {removed_files} stale release build file{} ({:.1} GB).",
                if removed_files == 1 { "" } else { "s" },
                removed_bytes as f64 / (1u64 << 30) as f64
            ));
        }
    }
}

fn sweep_target(release: &Path, build_started: SystemTime) -> Option<(usize, u64)> {
    let units_written = fs::metadata(release.join(UNITS_FILE))
        .ok()?
        .modified()
        .ok()?;
    if units_written < build_started {
        return None;
    }
    let now = SystemTime::now();
    let stamp = release.join(STAMP_FILE);
    if let Some(last) = fs::metadata(&stamp).ok().and_then(|m| m.modified().ok()) {
        if now.duration_since(last).unwrap_or_default() < SWEEP_INTERVAL {
            return None;
        }
    }
    // Holding cargo's own build lock keeps a build running in another session from losing files mid-compile.
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(release.join(".cargo-lock"))
        .ok()?;
    lock.try_lock().ok()?;
    let live = live_unit_hashes(&release.join(UNITS_FILE))?;
    let keep_after = now - KEEP_RECENT;
    let mut removed = (0usize, 0u64);
    for entry in fs::read_dir(release.join("deps")).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || !is_stale(&name, &metadata, &live, keep_after) {
            continue;
        }
        if fs::remove_file(entry.path()).is_ok() {
            removed.0 += 1;
            removed.1 += metadata.len();
        }
    }
    if let Ok(entries) = fs::read_dir(release.join("build")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_dir() || !is_stale(&name, &metadata, &live, keep_after) {
                continue;
            }
            let (files, bytes) = tree_size(&entry.path());
            if fs::remove_dir_all(entry.path()).is_ok() {
                removed.0 += files;
                removed.1 += bytes;
            }
        }
    }
    let _ = fs::write(&stamp, b"");
    Some(removed)
}

/// Every unit hash the recorded build used. None when the list is empty or unreadable, so a broken list never deletes anything.
fn live_unit_hashes(units: &Path) -> Option<HashSet<String>> {
    let text = fs::read_to_string(units).ok()?;
    let mut live = HashSet::new();
    for line in text.lines() {
        let Ok(message) =
            serde_json::from_str::<serde_json::Value>(line.trim_start_matches('\u{feff}'))
        else {
            continue;
        };
        let paths = message
            .get("filenames")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
            .chain(message.get("executable"))
            .chain(message.get("out_dir"));
        for path in paths.filter_map(|p| p.as_str()) {
            for component in path.split(['/', '\\']) {
                if let Some(hash) = unit_hash(component) {
                    live.insert(hash.to_string());
                }
            }
        }
    }
    (!live.is_empty()).then_some(live)
}

fn is_stale(
    name: &str,
    metadata: &fs::Metadata,
    live: &HashSet<String>,
    keep_after: SystemTime,
) -> bool {
    let Some(hash) = unit_hash(name) else {
        return false;
    };
    !live.contains(hash)
        && metadata
            .modified()
            .is_ok_and(|modified| modified < keep_after)
}

/// The 16-hex-digit unit hash cargo puts in artifact names: `libfoo-0123456789abcdef.rlib`, `build/cef-dll-sys-0123456789abcdef`.
fn unit_hash(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    (0..bytes.len()).find_map(|dash| {
        let end = dash + 17;
        let hash_ends = end == bytes.len() || bytes.get(end) == Some(&b'.');
        (bytes[dash] == b'-'
            && end <= bytes.len()
            && hash_ends
            && bytes[dash + 1..end]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)))
        .then(|| &name[dash + 1..end])
    })
}

fn tree_size(dir: &Path) -> (usize, u64) {
    let mut total = (0usize, 0u64);
    let Ok(entries) = fs::read_dir(dir) else {
        return total;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let (files, bytes) = tree_size(&entry.path());
            total.0 += files;
            total.1 += bytes;
        } else {
            total.0 += 1;
            total.1 += metadata.len();
        }
    }
    total
}
