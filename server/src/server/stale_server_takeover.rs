//! A gxserver of another build that is in the way of this one: it gets out of the way on its own (superseded), or the starting build ends it.

use std::{sync::Arc, time::Duration};

use serde_json::json;

use crate::{
    logging::{GxserverLogInput, LogLevel},
    paths::GxserverPaths,
    runtime::read_runtime_metadata,
};

use super::*;

/// How often a running daemon checks whether a newer build replaced its executable.
#[cfg(windows)]
const SUPERSEDED_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long a starting gxserver keeps trying to take the port from a daemon that holds it without answering.
const PORT_TAKEOVER_PATIENCE: Duration = Duration::from_secs(20);

/// CDXC:ServerDaemon 2026-10-12 WHY:
/// After `cargo xtask start` stopped gxserver, a `gxserver start` from the SSH side (Task Scheduler hand-off at 03:59:33) started the still-unrefreshed package binary; 7 s later the app renamed that image into `.retired-native`, so the daemon only reported `…:superseded` and kept the port. It then spent ~150 s in startup with the listener bound but not served (connect timeouts), and the app's own gxserver failed with "Port 58746 is already in use" while the app showed the startup diagnostics dialog. A superseded daemon therefore stops by itself as soon as it notices, the same way `/api/control/stop` stops it, so whoever starts next runs the installed build. Windows only: there the app and the desktop-session hand-off start a replacement on demand; a Linux remote has nobody to restart it.
/// SEE-ALSO: `take_over_port_from_stale_gxserver` below; `running_image_superseded` in server/src/platform/process.rs; the desktop's post-spawn takeover in apps/desktop/src/app/os_integration/gxserver_bootstrap.rs.
pub(crate) fn spawn_superseded_retire_task(
    state: &Arc<AppState>,
) -> Option<tokio::task::JoinHandle<()>> {
    #[cfg(windows)]
    {
        let state = state.clone();
        Some(tokio::spawn(async move {
            let mut shutdown_rx = state.shutdown_tx.subscribe();
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => return,
                    _ = tokio::time::sleep(SUPERSEDED_CHECK_INTERVAL) => {}
                }
                let superseded =
                    tokio::task::spawn_blocking(crate::platform::process::running_image_superseded)
                        .await
                        .unwrap_or(false);
                if !superseded {
                    continue;
                }
                let _ = state.logger.log(GxserverLogInput {
                    level: LogLevel::Warn,
                    event: "serverSupersededRetiring".to_string(),
                    server_id: Some(state.metadata.server_id.clone()),
                    request_id: None,
                    client: None,
                    duration_ms: None,
                    error: Some(
                        "A newer gxserver build replaced this daemon's executable, so it stopped to let the installed build take over."
                            .to_string(),
                    ),
                    details: Some(json!({ "buildIdentity": state.build_identity })),
                });
                broadcast_server_stopping(&state);
                let _ = state.shutdown_tx.send(());
                return;
            }
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        None
    }
}

/// Ends a daemon of another build that answered health but did not stop when asked. `true` once it is gone.
pub(crate) fn end_stale_gxserver(pid: u32) -> bool {
    #[cfg(windows)]
    {
        crate::platform::process::process_runs_gxserver(pid)
            && crate::platform::process::terminate_process(pid, Duration::from_secs(5))
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// Binds the local listener, taking the port from a gxserver of another build that holds it without answering health (a superseded daemon still in its startup, or one hung on its way out). A holder that recorded this same build is left alone: it is this build starting, and `Ok(None)` says so.
pub(crate) async fn bind_local_listener_taking_over_stale_holder(
    address: std::net::SocketAddr,
    paths: &GxserverPaths,
    build_identity: &str,
    logger: &crate::logging::GxserverLogger,
) -> anyhow::Result<Option<tokio::net::TcpListener>> {
    let started = std::time::Instant::now();
    let mut ended_pid = None;
    loop {
        let error = match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => return Ok(Some(listener)),
            Err(error) => error,
        };
        if error.kind() != std::io::ErrorKind::AddrInUse || !cfg!(windows) {
            return Err(port_in_use_error(address.port(), error));
        }
        // The holder writes its runtime record right after binding, so a missing or foreign record is a holder still taking the port: wait for it.
        let holder = read_runtime_metadata(paths)
            .ok()
            .flatten()
            .filter(|holder| {
                holder.port == address.port()
                    && holder.pid != std::process::id()
                    && crate::platform::process::process_is_alive(holder.pid)
            });
        if let Some(holder) = holder {
            if holder.build_identity == build_identity {
                return Ok(None);
            }
            if ended_pid != Some(holder.pid) {
                ended_pid = Some(holder.pid);
                let ended = end_stale_gxserver(holder.pid);
                let _ = logger.log(GxserverLogInput {
                    level: LogLevel::Warn,
                    event: "staleGxserverPortTakeover".to_string(),
                    server_id: Some(holder.server_id.clone()),
                    request_id: None,
                    client: None,
                    duration_ms: Some(started.elapsed().as_millis()),
                    error: (!ended)
                        .then(|| "The stale gxserver did not exit when ended.".to_string()),
                    details: Some(json!({
                        "pid": holder.pid,
                        "heldBuildIdentity": holder.build_identity,
                        "buildIdentity": build_identity,
                    })),
                });
            }
        }
        if started.elapsed() >= PORT_TAKEOVER_PATIENCE {
            return Err(port_in_use_error(address.port(), error));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn port_in_use_error(port: u16, error: std::io::Error) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        anyhow::anyhow!("Port {port} is already in use and did not respond as a compatible gxserver. Stop the conflicting process or update Ghostex/gxserver so their protocol versions match.")
    } else {
        anyhow::anyhow!(error)
    }
}
