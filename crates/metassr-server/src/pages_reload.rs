//! Dev-mode reload of SSR renderers after page rebuilds.
//!
//! Page bundles are rebuilt on disk by the [`Rebuilder`](metassr_build::rebuilder::Rebuilder),
//! but the running server holds MetaCall handles loaded once at startup. This
//! task swaps those handles whenever a page rebuild succeeds, so SSR output
//! reflects edits without restarting the server.

use std::path::PathBuf;

use metassr_build::{rebuilder::RebuildType, server::renderer::page::PageRenderer};
use metassr_utils::routes::page_source_to_route;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tracing::{debug, error, info, warn};

use crate::handler::PageRegistry;

pub async fn watch_pages(
    mut receiver: Receiver<RebuildType>,
    registry: PageRegistry,
    dist_dir: PathBuf,
) {
    loop {
        let rebuild_type = match receiver.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(skipped)) => {
                warn!("page reloader lagged, skipped {skipped} rebuild events");
                continue;
            }
            Err(RecvError::Closed) => break,
        };

        let RebuildType::Page(source_path) = rebuild_type else {
            continue;
        };

        let Some(route) = page_source_to_route(&source_path) else {
            debug!("Cannot map {source_path:?} to a page route; skipping reload");
            continue;
        };

        let Some(renderer) = registry.get(&route).cloned() else {
            debug!("No registered renderer for route '{route}'; skipping reload");
            continue;
        };

        let loaded = {
            let dist_dir = dist_dir.clone();
            let route = route.clone();
            tokio::task::spawn_blocking(move || PageRenderer::from_manifest(&dist_dir, &route))
                .await
        };

        match loaded {
            Ok(Ok(new_renderer)) => {
                *renderer.write().unwrap() = new_renderer;
                info!("Reloaded SSR renderer for route '{route}'");
            }
            // Keep serving the previous renderer if the new bundle cannot be
            // loaded; the next successful rebuild will replace it.
            Ok(Err(e)) => error!("Failed to reload renderer for route '{route}': {e}"),
            Err(e) => error!("Renderer reload task failed for route '{route}': {e}"),
        }
    }
}
