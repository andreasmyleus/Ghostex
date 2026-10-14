use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Runs `command` to completion within `timeout` and returns its trimmed stdout
/// when it exits zero. A timeout kills it and reads as `None`, like any failure;
/// stdin is closed and stderr dropped so a prompt can never wedge the caller.
pub(super) fn capture(mut command: Command, timeout: Duration) -> Option<String> {
    crate::platform::process::NoConsoleWindow::no_console_window(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = reader.join().unwrap_or_default();
    if !status?.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output).trim().to_string())
}
