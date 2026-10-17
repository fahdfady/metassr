//! Dev-mode hot-reload of API route scripts.
//!
//! API scripts are executed straight from source, so a change only requires
//! reloading the MetaCall handle. Failures are pushed to the browser error
//! overlay through the rebuilder.

use std::sync::Arc;

use metassr_api_handler::ApiRoutes;
use metassr_build::rebuilder::{RebuildType, Rebuilder};
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tracing::{debug, error, info, warn};

pub async fn watch_api_routes(
    mut receiver: Receiver<RebuildType>,
    rebuilder: Arc<Rebuilder>,
    api_routes: Arc<ApiRoutes>,
) {
    // Tracks whether the last API reload failed, so a later success only
    // clears errors we own.
    let mut reported_error = false;

    loop {
        let rebuild_type = match receiver.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(skipped)) => {
                warn!("API reloader lagged, skipped {skipped} rebuild events");
                continue;
            }
            Err(RecvError::Closed) => break,
        };

        let RebuildType::Api(relative_path) = rebuild_type else {
            continue;
        };

        let entry = api_routes
            .routes()
            .iter()
            .find(|route| route.file_path.ends_with(&relative_path))
            .cloned();

        let Some(entry) = entry else {
            debug!("No API route registered for {relative_path:?}; skipping reload");
            continue;
        };

        let result = {
            let api_routes = Arc::clone(&api_routes);
            let file_path = entry.file_path.clone();
            tokio::task::spawn_blocking(move || api_routes.reload_script(&file_path, entry.tag))
                .await
        };

        match result {
            Ok(Ok(())) => {
                if reported_error {
                    rebuilder.clear_last_errors();
                    reported_error = false;
                }
                info!("Reloaded API route: {relative_path:?}");
            }
            Ok(Err(e)) => {
                let message = format!("Failed to reload API route {relative_path:?}: {e}");
                error!("{message}");
                rebuilder.notify_build_error(vec![message]);
                reported_error = true;
            }
            Err(e) => {
                let message = format!("API reload task failed for {relative_path:?}: {e}");
                error!("{message}");
                rebuilder.notify_build_error(vec![message]);
                reported_error = true;
            }
        }
    }
}
