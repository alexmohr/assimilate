// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Reading one file as it is on the host now, for the server to compare
//! with a backup. Only regular files inside a configured backup source are
//! read: the server can only ask for what the host already backs up. No
//! symbolic link below a source is followed (see [`open`]).

mod open;

use std::path::{Component, Path, PathBuf};

use shared::{
    live_file::{LIVE_FILE_MAX_BYTES, LiveFile, LiveReadRefusal, LiveText},
    time::Timestamp,
    types::AgentConfig,
};
use tokio::io::AsyncReadExt as _;

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
/// `sources`. The path must name a regular file below a source, and no name
/// on the way from the source to it may be a symbolic link: borg stores a
/// link as a link, so the file at that path now is the link itself, and
/// reading through it could lead out of every source.
pub async fn read(sources: Vec<PathBuf>, archive_path: &str) -> LiveFile {
    read_within(&sources, archive_path)
        .await
        .unwrap_or_else(|reason| LiveFile::Refused { reason })
}

async fn read_within(sources: &[PathBuf], archive_path: &str) -> Result<LiveFile, LiveReadRefusal> {
    let path = local_path(archive_path).ok_or(LiveReadRefusal::NotInSources)?;
    let (root, names) = open::within_sources(sources, &path)
        .await
        .ok_or(LiveReadRefusal::NotInSources)?;
    read_regular_file(open::beneath(&root, &names).await?).await
}

fn regular_file(metadata: &std::fs::Metadata) -> Result<(), LiveReadRefusal> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(LiveReadRefusal::NotRegularFile)
    }
}

/// Reads the file [`open::beneath`] opened, checking again that it is a
/// regular file.
async fn read_regular_file(file: tokio::fs::File) -> Result<LiveFile, LiveReadRefusal> {
    let metadata = file.metadata().await?;
    regular_file(&metadata)?;
    if metadata.len() > LIVE_FILE_MAX_BYTES {
        return Ok(LiveFile::TooLarge {
            size: metadata.len(),
        });
    }
    let mut bytes = Vec::new();
    // One byte past the limit tells a file that grew since the check.
    file.take(LIVE_FILE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .await?;
    Ok(contents(
        bytes,
        metadata
            .modified()
            .ok()
            .and_then(|modified| Timestamp::try_from(modified).ok()),
    ))
}

/// What a file holding `bytes` is to the comparison.
fn contents(bytes: Vec<u8>, modified: Option<Timestamp>) -> LiveFile {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > LIVE_FILE_MAX_BYTES {
        return LiveFile::TooLarge { size };
    }
    if bytes.contains(&0) {
        return LiveFile::Binary;
    }
    String::from_utf8(bytes).map_or(LiveFile::Binary, |text| LiveFile::Text {
        text: LiveText(text),
        size,
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backup source holding `etc/app.conf`, and the archive path of it.
    async fn source() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = tokio::fs::canonicalize(dir.path()).await.unwrap();
        tokio::fs::create_dir_all(root.join("etc")).await.unwrap();
        tokio::fs::write(root.join("etc/app.conf"), "listen 80\n")
            .await
            .unwrap();
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
        let (_dir, root, path) = source().await;
        let file = read(vec![root], &path).await;
        assert!(
            matches!(
                &file,
                LiveFile::Text { text, size: 10, modified: Some(_) } if text.0 == "listen 80\n"
            ),
            "{file:?}"
        );
    }

    #[test]
    fn a_file_that_grew_past_the_limit_while_read_is_too_large() {
        let bytes = vec![b'a'; usize::try_from(LIVE_FILE_MAX_BYTES).unwrap() + 1];
        assert_eq!(
            contents(bytes, None),
            LiveFile::TooLarge {
                size: LIVE_FILE_MAX_BYTES + 1
            }
        );
    }

    #[tokio::test]
    async fn a_file_outside_every_source_is_refused() {
        let (_dir, root, path) = source().await;
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
        let (_dir, root, path) = source().await;
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
        let (_dir, root, _path) = source().await;
        let outside = tempfile::tempdir().unwrap();
        tokio::fs::write(outside.path().join("secret"), "s3cret")
            .await
            .unwrap();
        tokio::fs::symlink(outside.path().join("secret"), root.join("link"))
            .await
            .unwrap();

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
        let (_dir, root, _path) = source().await;
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
        let (_dir, root, _path) = source().await;
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
        let (_dir, root, _path) = source().await;
        tokio::fs::write(root.join("blob"), [0u8, 159, 146, 150])
            .await
            .unwrap();
        tokio::fs::write(root.join("latin1"), [b'c', b'a', b'f', 0xE9])
            .await
            .unwrap();
        let large = vec![b'a'; usize::try_from(LIVE_FILE_MAX_BYTES).unwrap() + 1];
        tokio::fs::write(root.join("large"), &large).await.unwrap();

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

    #[cfg(unix)]
    fn refused(reason: LiveReadRefusal) -> LiveFile {
        LiveFile::Refused { reason }
    }

    /// A directory outside every source holding `secret`.
    #[cfg(unix)]
    async fn outside() -> tempfile::TempDir {
        let outside = tempfile::tempdir().unwrap();
        tokio::fs::write(outside.path().join("secret"), "s3cret")
            .await
            .unwrap();
        outside
    }

    #[tokio::test]
    async fn a_current_directory_reference_reads_the_same_file() {
        let root = PathBuf::from("/srv/app");
        assert_eq!(
            open::within_sources(
                std::slice::from_ref(&root),
                Path::new("/srv/app/./etc/app.conf")
            )
            .await,
            Some((root, vec!["etc".into(), "app.conf".into()]))
        );
    }

    #[tokio::test]
    async fn a_path_matches_a_source_by_its_names_only() {
        let sources = [PathBuf::from("/srv/app")];
        assert_eq!(
            open::within_sources(&sources, Path::new("/srv/application/x")).await,
            None
        );
        assert_eq!(
            open::within_sources(&sources, Path::new("/srv/app")).await,
            Some((PathBuf::from("/srv/app"), Vec::new()))
        );
        assert_eq!(
            open::within_sources(&[PathBuf::from("srv/app")], Path::new("/srv/app/x")).await,
            None,
            "a relative source matches nothing"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_to_a_directory_outside_the_source_is_refused() {
        let (_dir, root, _path) = source().await;
        let outside = outside().await;
        tokio::fs::symlink(outside.path(), root.join("etc/linked"))
            .await
            .unwrap();

        let secret = archive_path_of(&root.join("etc/linked/secret"));

        assert_eq!(
            read(vec![root], &secret).await,
            refused(LiveReadRefusal::NotInSources)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_that_stays_inside_the_source_is_not_followed() {
        let (_dir, root, _path) = source().await;
        tokio::fs::symlink("app.conf", root.join("etc/alias.conf"))
            .await
            .unwrap();
        tokio::fs::symlink("etc", root.join("conf")).await.unwrap();

        assert_eq!(
            read(
                vec![root.clone()],
                &archive_path_of(&root.join("etc/alias.conf"))
            )
            .await,
            refused(LiveReadRefusal::NotRegularFile)
        );
        assert_eq!(
            read(
                vec![root.clone()],
                &archive_path_of(&root.join("conf/app.conf"))
            )
            .await,
            refused(LiveReadRefusal::NotRegularFile)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_dangling_link_is_not_a_regular_file() {
        let (_dir, root, _path) = source().await;
        tokio::fs::symlink(root.join("gone"), root.join("dangling"))
            .await
            .unwrap();
        assert_eq!(
            read(vec![root.clone()], &archive_path_of(&root.join("dangling"))).await,
            refused(LiveReadRefusal::NotRegularFile)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_path_through_a_file_is_not_a_regular_file() {
        let (_dir, root, _path) = source().await;
        let through = archive_path_of(&root.join("etc/app.conf/x"));
        assert_eq!(
            read(vec![root], &through).await,
            refused(LiveReadRefusal::NotRegularFile)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pipe_is_not_opened() {
        let (_dir, root, _path) = source().await;
        nix::unistd::mkfifo(&root.join("pipe"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        assert_eq!(
            read(vec![root.clone()], &archive_path_of(&root.join("pipe"))).await,
            refused(LiveReadRefusal::NotRegularFile)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_source_that_is_a_file_is_read_but_not_through_a_link() {
        let (_dir, root, path) = source().await;
        let outside = outside().await;
        tokio::fs::symlink(outside.path().join("secret"), root.join("secret.conf"))
            .await
            .unwrap();

        let file = read(vec![root.join("etc/app.conf")], &path).await;
        assert!(matches!(file, LiveFile::Text { size: 10, .. }), "{file:?}");

        let secret = root.join("secret.conf");
        assert_eq!(
            read(vec![secret.clone()], &archive_path_of(&secret)).await,
            refused(LiveReadRefusal::NotInSources)
        );
        assert_eq!(
            read(vec![PathBuf::from("/")], "").await,
            refused(LiveReadRefusal::NotInSources)
        );
    }

    /// The source itself may run through a link: an administrator chose it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_source_configured_through_a_link_is_read_by_either_path() {
        let (_dir, root, path) = source().await;
        let links = tempfile::tempdir().unwrap();
        let linked_root = tokio::fs::canonicalize(links.path())
            .await
            .unwrap()
            .join("app");
        tokio::fs::symlink(&root, &linked_root).await.unwrap();

        let by_link = archive_path_of(&linked_root.join("etc/app.conf"));
        for archive_path in [by_link, path] {
            let file = read(vec![linked_root.clone()], &archive_path).await;
            assert!(matches!(file, LiveFile::Text { size: 10, .. }), "{file:?}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_in_a_missing_source_is_not_found() {
        let (_dir, root, _path) = source().await;
        let missing_root = root.join("missing");
        assert_eq!(
            read(
                vec![missing_root.clone()],
                &archive_path_of(&missing_root.join("x"))
            )
            .await,
            refused(LiveReadRefusal::NotFound)
        );
    }
}
