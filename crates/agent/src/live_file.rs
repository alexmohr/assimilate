// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Reading one file as it is on the host now, for the server to compare
//! with a backup. Only regular files inside a configured backup source are
//! read: the server can only ask for what the host already backs up.

use std::{
    io::Read as _,
    path::{Component, Path, PathBuf},
};

use shared::{
    live_file::{LIVE_FILE_MAX_BYTES, LiveFile, LiveReadRefusal, LiveText},
    types::AgentConfig,
};

/// Every backup source the host's schedules name, as configured.
pub fn configured_sources(config: Option<&AgentConfig>) -> Vec<PathBuf> {
    config
        .map(|config| {
            config
                .repos
                .iter()
                .flat_map(|repo| &repo.schedules)
                .flat_map(|schedule| &schedule.backup_sources)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The local path an archive path stands for. Borg stores absolute paths
/// without their leading `/`, and on Windows with the drive letter as the
/// first component (`C/Users/...`).
fn local_path(archive_path: &str) -> Option<PathBuf> {
    let relative = archive_path.trim_start_matches('/');
    if relative.is_empty()
        || Path::new(relative)
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    Some(platform_path(relative))
}

#[cfg(not(windows))]
fn platform_path(relative: &str) -> PathBuf {
    Path::new("/").join(relative)
}

#[cfg(windows)]
fn platform_path(relative: &str) -> PathBuf {
    let (first, rest) = relative.split_once('/').unwrap_or((relative, ""));
    let drive = first
        .chars()
        .next()
        .filter(|c| first.len() == 1 && c.is_ascii_alphabetic());
    drive.map_or_else(
        || Path::new(r"\").join(relative.replace('/', r"\")),
        |drive| PathBuf::from(format!(r"{drive}:\{}", rest.replace('/', r"\"))),
    )
}

/// Reads `archive_path` as it is on the host now, if it lies inside one of
/// `sources`. The file's own path and each source are resolved first, so a
/// symbolic link cannot lead out of a source; the file itself is opened
/// without following a link.
pub async fn read(sources: Vec<PathBuf>, archive_path: &str) -> LiveFile {
    let refused = |reason| LiveFile::Refused { reason };
    let Some(path) = local_path(archive_path) else {
        return refused(LiveReadRefusal::NotInSources);
    };
    let path = match tokio::fs::canonicalize(&path).await {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return refused(LiveReadRefusal::NotFound);
        }
        Err(_) => return refused(LiveReadRefusal::Unreadable),
    };
    let mut roots = Vec::with_capacity(sources.len());
    for source in sources {
        if let Ok(root) = tokio::fs::canonicalize(&source).await {
            roots.push(root);
        }
    }
    if !roots.iter().any(|root| path.starts_with(root)) {
        return refused(LiveReadRefusal::NotInSources);
    }
    tokio::task::spawn_blocking(move || read_regular_file(&path))
        .await
        .unwrap_or(LiveFile::Refused {
            reason: LiveReadRefusal::Unreadable,
        })
}

#[allow(
    clippy::disallowed_methods,
    reason = "runs inside spawn_blocking: one bounded read of a single file"
)]
fn read_regular_file(path: &Path) -> LiveFile {
    let refused = |reason| LiveFile::Refused { reason };
    // Checked before opening as well as after: Windows refuses to open a
    // directory as a file at all, which would read as unreadable.
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => return refused(LiveReadRefusal::NotRegularFile),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return refused(LiveReadRefusal::NotFound);
        }
        Err(_) => return refused(LiveReadRefusal::Unreadable),
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits());
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return refused(LiveReadRefusal::NotFound);
        }
        Err(_) => return refused(LiveReadRefusal::Unreadable),
    };
    let Ok(metadata) = file.metadata() else {
        return refused(LiveReadRefusal::Unreadable);
    };
    if !metadata.is_file() {
        return refused(LiveReadRefusal::NotRegularFile);
    }
    if metadata.len() > LIVE_FILE_MAX_BYTES {
        return LiveFile::TooLarge {
            size: metadata.len(),
        };
    }
    let mut bytes = Vec::new();
    // One byte past the limit tells a file that grew since the check.
    if file
        .take(LIVE_FILE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
    {
        return refused(LiveReadRefusal::Unreadable);
    }
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > LIVE_FILE_MAX_BYTES {
        return LiveFile::TooLarge { size };
    }
    if bytes.contains(&0) {
        return LiveFile::Binary;
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return LiveFile::Binary;
    };
    LiveFile::Text {
        text: LiveText(text),
        size,
        modified: metadata.modified().ok().map(chrono::DateTime::from),
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup"
)]
mod tests {
    use super::*;

    /// A backup source holding `etc/app.conf`, and the archive path of it.
    fn source() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("etc/app.conf"), "listen 80\n").unwrap();
        let archive_path = archive_path_of(&root.join("etc/app.conf"));
        (dir, root, archive_path)
    }

    /// The path as borg stores it: no leading `/`, and on Windows the drive
    /// letter as the first component (`C/Users/...`).
    fn archive_path_of(path: &Path) -> String {
        let path = path.to_str().unwrap();
        #[cfg(windows)]
        let path = path
            .trim_start_matches(r"\\?\")
            .replacen(":\\", "/", 1)
            .replace('\\', "/");
        path.trim_start_matches('/').to_owned()
    }

    #[tokio::test]
    async fn a_file_inside_a_source_is_read() {
        let (_dir, root, path) = source();
        let file = read(vec![root], &path).await;
        let LiveFile::Text {
            text,
            size,
            modified,
        } = file
        else {
            panic!("expected text, got {file:?}");
        };
        assert_eq!(text.0, "listen 80\n");
        assert_eq!(size, 10);
        assert!(modified.is_some());
    }

    #[tokio::test]
    async fn a_file_outside_every_source_is_refused() {
        let (_dir, root, path) = source();
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(
            read(vec![elsewhere.path().to_path_buf()], &path).await,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources
            }
        );
        assert_eq!(
            read(Vec::new(), &path).await,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources
            }
        );
        drop(root);
    }

    #[tokio::test]
    async fn a_parent_reference_is_refused() {
        let (_dir, root, path) = source();
        let escaping = format!("{path}/../../../../etc/passwd");
        assert_eq!(
            read(vec![root], &escaping).await,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources
            }
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_out_of_the_source_is_refused() {
        let (_dir, root, _path) = source();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s3cret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.join("link")).unwrap();

        let file = read(vec![root.clone()], &archive_path_of(&root.join("link"))).await;

        assert_eq!(
            file,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources
            }
        );
    }

    #[tokio::test]
    async fn a_missing_file_is_reported() {
        let (_dir, root, _path) = source();
        let missing = archive_path_of(&root.join("etc/gone.conf"));
        assert_eq!(
            read(vec![root], &missing).await,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotFound
            }
        );
    }

    #[tokio::test]
    async fn a_directory_is_not_read() {
        let (_dir, root, _path) = source();
        let dir = archive_path_of(&root.join("etc"));
        assert_eq!(
            read(vec![root], &dir).await,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotRegularFile
            }
        );
    }

    #[tokio::test]
    async fn binary_and_large_files_are_reported_not_read() {
        let (_dir, root, _path) = source();
        std::fs::write(root.join("blob"), [0u8, 159, 146, 150]).unwrap();
        std::fs::write(root.join("latin1"), [b'c', b'a', b'f', 0xE9]).unwrap();
        let large = vec![b'a'; usize::try_from(LIVE_FILE_MAX_BYTES).unwrap() + 1];
        std::fs::write(root.join("large"), &large).unwrap();

        assert_eq!(
            read(vec![root.clone()], &archive_path_of(&root.join("blob"))).await,
            LiveFile::Binary
        );
        assert_eq!(
            read(vec![root.clone()], &archive_path_of(&root.join("latin1"))).await,
            LiveFile::Binary
        );
        assert_eq!(
            read(vec![root.clone()], &archive_path_of(&root.join("large"))).await,
            LiveFile::TooLarge {
                size: LIVE_FILE_MAX_BYTES + 1
            }
        );
    }

    #[test]
    fn archive_paths_map_to_local_paths() {
        assert_eq!(local_path(""), None);
        assert_eq!(local_path("etc/../../root"), None);
        #[cfg(not(windows))]
        {
            assert_eq!(
                local_path("etc/nginx/nginx.conf"),
                Some(PathBuf::from("/etc/nginx/nginx.conf"))
            );
            assert_eq!(local_path("/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        }
        #[cfg(windows)]
        assert_eq!(
            local_path("C/Users/me/notes.txt"),
            Some(PathBuf::from(r"C:\Users\me\notes.txt"))
        );
    }

    #[test]
    fn the_sources_are_every_schedules_sources() {
        let config: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_hostname": "web",
            "repos": [],
        }))
        .unwrap();
        assert_eq!(configured_sources(Some(&config)), Vec::<PathBuf>::new());
        assert_eq!(configured_sources(None), Vec::<PathBuf>::new());
    }
}
