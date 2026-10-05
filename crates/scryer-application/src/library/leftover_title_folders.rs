//! One-time removal of the empty title folders an old rename left behind.
//!
//! A rename that changed a title's folder name moved the files into the new
//! folder and, when the title's folder record was wrong, left the old folder
//! standing with nothing but empty season folders in it. This removes those
//! folders, and only those:
//!
//! - the folder sits directly under a configured library root,
//! - no title records it or a folder inside it, and the titles it is named
//!   after track no file in it,
//! - no library root is the folder or sits inside it,
//! - its name starts with the name of a title whose recorded folder is another
//!   folder directly under the same root,
//! - it holds no file of any kind at any depth, only directories.
//!
//! Directories are removed with `remove_dir`, which refuses a directory that
//! still holds anything, so a file can never be deleted from here.

use std::path::{Path, PathBuf};

use scryer_domain::Title;
use tracing::{info, warn};

use crate::library::rename::{build_title_folder_tokens, sanitize_filesystem_component};
use crate::stored_paths::{folder_paths_match, path_to_stored_string, stored_path_to_path_buf};
use crate::title_folder_rules::{path_is_strictly_within, stored_path_is_inside_folder};
use crate::{AppResult, AppUseCase};

/// What the one-time cleanup did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmptyDuplicateTitleFolderReport {
    /// Folders removed.
    pub removed: Vec<String>,
    /// Folders that matched a title by name but still hold a file, so were kept.
    pub kept_with_files: Vec<String>,
    /// Folders that could not be inspected or removed.
    pub failed: Vec<String>,
}

enum FolderOutcome {
    Removed,
    HoldsFiles,
    Failed(std::io::Error),
}

impl AppUseCase {
    /// Remove the empty leftover title folders described in the module docs.
    pub async fn remove_empty_duplicate_title_folders(
        &self,
    ) -> AppResult<EmptyDuplicateTitleFolderReport> {
        let roots = self
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| root.path)
            .collect::<Vec<_>>();
        let titles = self.services.catalog.titles.list(None, None).await?;
        let recorded = titles
            .iter()
            .filter_map(|title| {
                let folder = title.folder_path.as_deref().map(str::trim)?;
                (!folder.is_empty()).then_some((title, folder))
            })
            .collect::<Vec<_>>();

        let mut report = EmptyDuplicateTitleFolderReport::default();
        let mut visited_roots = Vec::<&str>::new();
        for root in &roots {
            if visited_roots
                .iter()
                .any(|visited| folder_paths_match(visited, root))
            {
                continue;
            }
            visited_roots.push(root);

            // Titles whose recorded folder is directly under this root.
            let siblings = recorded
                .iter()
                .filter(|(_, folder)| {
                    stored_path_to_path_buf(folder)
                        .parent()
                        .is_some_and(|parent| {
                            folder_paths_match(&path_to_stored_string(parent), root)
                        })
                })
                .copied()
                .collect::<Vec<_>>();
            if siblings.is_empty() {
                continue;
            }

            let root_path = stored_path_to_path_buf(root);
            let directories = match run_blocking(move || child_directories(&root_path)).await {
                Ok(directories) => directories,
                Err(error) => {
                    warn!(root = %root, error = %error, "skipping leftover title folder cleanup for a root that could not be listed");
                    continue;
                }
            };

            for directory in directories {
                let stored = path_to_stored_string(&directory);
                let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if recorded.iter().any(|(_, folder)| {
                    folder_paths_match(folder, &stored) || path_is_strictly_within(folder, &stored)
                }) || roots.iter().any(|other_root| {
                    folder_paths_match(other_root, &stored)
                        || path_is_strictly_within(other_root, &stored)
                }) {
                    continue;
                }
                let owners = siblings
                    .iter()
                    .filter(|(title, folder)| {
                        let recorded_name = stored_path_to_path_buf(folder)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(str::to_string)
                            .unwrap_or_default();
                        folder_name_starts_with_title(name, title)
                            && folder_name_starts_with_title(&recorded_name, title)
                    })
                    .map(|(title, _)| *title)
                    .collect::<Vec<_>>();
                if owners.is_empty() {
                    continue;
                }
                if self.any_tracked_file_inside(&owners, &stored).await? {
                    continue;
                }

                let target = directory.clone();
                match run_blocking(move || Ok(remove_file_free_tree(&target))).await {
                    Ok(FolderOutcome::Removed) => {
                        info!(folder = %stored, "removed an empty title folder a rename left behind");
                        report.removed.push(stored);
                    }
                    Ok(FolderOutcome::HoldsFiles) => {
                        info!(folder = %stored, "kept a leftover title folder that still holds files");
                        report.kept_with_files.push(stored);
                    }
                    Ok(FolderOutcome::Failed(error)) | Err(error) => {
                        warn!(folder = %stored, error = %error, "could not remove an empty leftover title folder");
                        report.failed.push(stored);
                    }
                }
            }
        }
        Ok(report)
    }

    async fn any_tracked_file_inside(&self, titles: &[&Title], folder: &str) -> AppResult<bool> {
        for title in titles {
            let files = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await?;
            if files
                .iter()
                .any(|file| stored_path_is_inside_folder(folder, &file.file_path))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

async fn run_blocking<T, F>(work: F) -> std::io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(std::io::Error::other)?
}

/// The directories directly inside `root`. A symlink is not a directory here.
fn child_directories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

/// Whether `directory` holds nothing but directories, at every depth. A
/// symlink or any other non-directory entry counts as a file.
fn holds_no_files(directory: &Path) -> std::io::Result<bool> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !holds_no_files(&entry.path())? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Remove `directory` and the directories inside it, deepest first.
/// `remove_dir` fails on a directory that is not empty, so anything that
/// appeared after the check stops the removal instead of being deleted.
fn remove_empty_tree(directory: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            remove_empty_tree(&entry.path())?;
        }
    }
    std::fs::remove_dir(directory)
}

fn remove_file_free_tree(directory: &Path) -> FolderOutcome {
    match holds_no_files(directory) {
        Ok(true) => match remove_empty_tree(directory) {
            Ok(()) => FolderOutcome::Removed,
            Err(error) => FolderOutcome::Failed(error),
        },
        Ok(false) => FolderOutcome::HoldsFiles,
        Err(error) => FolderOutcome::Failed(error),
    }
}

/// Whether `folder_name` starts with the title's name as a folder template
/// renders it, ending at a word boundary so one title's name is not matched
/// inside a longer one.
fn folder_name_starts_with_title(folder_name: &str, title: &Title) -> bool {
    let tokens = build_title_folder_tokens(title, title.year);
    let title_name =
        sanitize_filesystem_component(tokens.get("title").map(String::as_str).unwrap_or_default());
    let title_name = title_name.trim().to_lowercase();
    if title_name.is_empty() {
        return false;
    }
    let folder_name = folder_name.to_lowercase();
    folder_name
        .strip_prefix(&title_name)
        .is_some_and(|rest| !rest.chars().next().is_some_and(char::is_alphanumeric))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_of_empty_directories_is_removed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = temp.path().join("Synthetic Show (2024)");
        std::fs::create_dir_all(folder.join("Season 1")).expect("season");
        std::fs::create_dir_all(folder.join("Specials").join("Nested")).expect("nested");
        assert!(matches!(
            remove_file_free_tree(&folder),
            FolderOutcome::Removed
        ));
        assert!(!folder.exists());
        assert!(temp.path().is_dir());
    }

    #[test]
    fn a_tree_holding_one_file_is_left_whole() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = temp.path().join("Synthetic Show (2024)");
        std::fs::create_dir_all(folder.join("Season 1")).expect("season");
        std::fs::create_dir_all(folder.join("Specials")).expect("specials");
        let kept = folder.join("Specials").join("poster.jpg");
        std::fs::write(&kept, b"x").expect("file");
        assert!(matches!(
            remove_file_free_tree(&folder),
            FolderOutcome::HoldsFiles
        ));
        assert!(kept.is_file());
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_counts_as_a_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let folder = temp.path().join("Synthetic Show (2024)");
        std::fs::create_dir_all(&folder).expect("folder");
        std::os::unix::fs::symlink(&elsewhere, folder.join("link")).expect("symlink");
        assert!(matches!(
            remove_file_free_tree(&folder),
            FolderOutcome::HoldsFiles
        ));
        assert!(elsewhere.is_dir());
    }
}
