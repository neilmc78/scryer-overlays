use std::collections::BTreeSet;
use std::time::Duration;

use scryer_domain::{DomainEventFilter, DomainEventType};
use tokio_util::sync::CancellationToken;

use crate::AppUseCase;

pub(crate) const POSTER_OVERLAY_SUBSCRIBER: &str = "poster_overlays";
const OVERLAY_EVENT_BATCH_LIMIT: usize = 500;
const OVERLAY_EVENT_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Events after which a title's badges or existence may have changed. Probe
/// data lands with `MediaFileAnalyzed`, not with the import itself.
const OVERLAY_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::MediaFileAnalyzed,
    DomainEventType::MediaFileImported,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRestored,
    DomainEventType::TitleDeleted,
];

/// Rebuild posters on import, analysis, upgrade and delete events, and run a
/// slow library-wide reconcile as a safety net (it is also the only way new
/// upstream artwork is noticed, since image refreshes emit no event).
pub async fn start_poster_overlay_worker(app: AppUseCase, token: CancellationToken) {
    let Some(overlays) = app.poster_overlays().cloned() else {
        return;
    };
    let repository = overlays.repository.clone();
    let events = app.services.events.domain_events.clone();
    let mut event_rx = app.runtime.events.domain_event_broadcast.subscribe();

    let settings = repository.get_settings().await.unwrap_or_default();
    if let Err(error) = overlays.refresh_enabled_flag().await {
        tracing::warn!(%error, "failed to read poster overlay libraries");
    }
    overlays.engine.set_parallelism(settings.parallelism);

    // A fresh install starts at the head of the log: the startup reconcile
    // covers everything that happened before.
    let mut last_sequence = match events
        .get_subscriber_offset(POSTER_OVERLAY_SUBSCRIBER)
        .await
    {
        Ok(0) => match events.latest_sequence().await {
            Ok(latest) => {
                let _ = events
                    .set_subscriber_offset(POSTER_OVERLAY_SUBSCRIBER, latest)
                    .await;
                latest
            }
            Err(_) => 0,
        },
        Ok(sequence) => sequence,
        Err(error) => {
            tracing::warn!(%error, "failed to load poster overlay event offset; starting at 0");
            0
        }
    };

    let mut reconcile_every = Duration::from_secs(settings.reconcile_interval_seconds);
    let mut next_reconcile = tokio::time::Instant::now();
    let mut poll_events = true;

    loop {
        if poll_events {
            match drain_overlay_events(&app, last_sequence).await {
                Ok(sequence) => {
                    last_sequence = sequence;
                    poll_events = false;
                }
                Err(error) => {
                    tracing::warn!(%error, "poster overlay event poll failed; will retry");
                }
            }
        }

        let retry_at = poll_events.then(|| tokio::time::Instant::now() + OVERLAY_EVENT_RETRY_DELAY);
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep_until(next_reconcile) => {
                if let Err(error) = overlays.reconcile().await {
                    tracing::warn!(%error, "poster overlay reconcile failed");
                }
                if let Ok(settings) = repository.get_settings().await {
                    reconcile_every = Duration::from_secs(settings.reconcile_interval_seconds);
                }
                next_reconcile = tokio::time::Instant::now() + reconcile_every;
            }
            _ = overlays.wake.notified() => {
                if let Err(error) = overlays.reconcile().await {
                    tracing::warn!(%error, "poster overlay reconcile failed");
                }
            }
            result = event_rx.recv() => match result {
                Ok(high_water) => poll_events = poll_events || high_water > last_sequence,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => poll_events = true,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            _ = async {
                match retry_at {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {}
        }
    }
}

/// Process every overlay-relevant event after `after_sequence`, advancing
/// the durable offset batch by batch. Returns the new offset.
pub async fn drain_overlay_events(
    app: &AppUseCase,
    mut after_sequence: i64,
) -> crate::AppResult<i64> {
    let events = app.services.events.domain_events.clone();
    loop {
        let batch = events
            .list(&DomainEventFilter {
                event_types: Some(OVERLAY_EVENT_TYPES.to_vec()),
                after_sequence: Some(after_sequence),
                limit: OVERLAY_EVENT_BATCH_LIMIT,
                ..DomainEventFilter::default()
            })
            .await?;
        let Some(last) = batch.last().map(|event| event.sequence) else {
            return Ok(after_sequence);
        };
        let title_ids = batch
            .into_iter()
            .filter_map(|event| event.title_id)
            .collect::<BTreeSet<_>>();
        if !title_ids.is_empty()
            && let Some(overlays) = app.poster_overlays()
        {
            let summary = overlays
                .process_titles(title_ids.into_iter().collect())
                .await;
            if summary.rendered > 0 {
                tracing::info!(
                    rendered = summary.rendered,
                    "rebuilt poster overlays after media changes"
                );
            }
        }
        events
            .set_subscriber_offset(POSTER_OVERLAY_SUBSCRIBER, last)
            .await?;
        after_sequence = last;
    }
}
