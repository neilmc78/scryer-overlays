use scryer_application::AppUseCase;

pub(crate) const ID: &str = "0017_empty_duplicate_title_folders";

/// Remove the empty title folders an old rename left beside the folder it
/// moved the title into.
///
/// One attempt only. The runner records this migration before it starts, so a
/// failure or an interrupted run is never retried and nothing is resumed.
pub(crate) async fn run(app: &AppUseCase) {
    match app.remove_empty_duplicate_title_folders().await {
        Ok(report) => tracing::info!(
            removed = report.removed.len(),
            kept_with_files = report.kept_with_files.len(),
            failed = report.failed.len(),
            "one-time cleanup of empty leftover title folders finished"
        ),
        Err(error) => tracing::warn!(
            error = %error,
            "one-time cleanup of empty leftover title folders failed and will not be retried"
        ),
    }
}
