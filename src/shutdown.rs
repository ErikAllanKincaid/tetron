//! Graceful shutdown via SIGINT/SIGTERM.
//!
//! Returns a [`CancellationToken`] that fires when a shutdown signal is received.
//! All long-running tasks select on this token to exit cleanly.

use tokio_util::sync::CancellationToken;

/// Creates a cancellation token that fires on SIGINT or SIGTERM.
pub fn token() -> CancellationToken {
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        signal_listener().await;
        tracing::info!("shutdown signal received");
        t.cancel();
    });
    token
}

async fn signal_listener() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}
