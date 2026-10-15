// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::{Path, PathBuf};

/// Where the app finds what it ships with.
///
/// A release bundle carries `server` and `agent` next to the app's own
/// executable (Tauri's `externalBin`) and everything else under its resource
/// directory. Each location can be overridden by an environment variable, so
/// a development build can point at `target/debug` and locally installed
/// tools instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleLayout {
    /// The `server` binary.
    pub server_binary: PathBuf,
    /// The `agent` binary.
    pub agent_binary: PathBuf,
    /// The `PostgreSQL` installation (contains `bin/`).
    pub postgres_dir: PathBuf,
    /// The built web UI.
    pub static_dir: PathBuf,
    /// The built documentation site, if shipped.
    pub docs_dir: Option<PathBuf>,
    /// The `borg` binary, if shipped; otherwise `borg` is looked up on `PATH`.
    pub borg_binary: Option<PathBuf>,
}

/// Environment variables that override the bundled locations.
pub const SERVER_BIN_VAR: &str = "ASSIMILATE_DESKTOP_SERVER_BIN";
/// See [`SERVER_BIN_VAR`].
pub const AGENT_BIN_VAR: &str = "ASSIMILATE_DESKTOP_AGENT_BIN";
/// See [`SERVER_BIN_VAR`].
pub const POSTGRES_DIR_VAR: &str = "ASSIMILATE_DESKTOP_POSTGRES_DIR";
/// See [`SERVER_BIN_VAR`].
pub const STATIC_DIR_VAR: &str = "ASSIMILATE_DESKTOP_STATIC_DIR";
/// See [`SERVER_BIN_VAR`].
pub const DOCS_DIR_VAR: &str = "ASSIMILATE_DESKTOP_DOCS_DIR";
/// See [`SERVER_BIN_VAR`].
pub const BORG_BIN_VAR: &str = "ASSIMILATE_DESKTOP_BORG_BIN";

impl BundleLayout {
    /// Resolves every location: an override from `env` wins, otherwise the
    /// bundled default. Optional parts are only used if their exact path
    /// exists, so an empty `borg/` directory does not stand in for the
    /// `borg/borg` binary. The checks don't block the runtime.
    pub async fn resolve(
        exe_dir: &Path,
        resource_dir: &Path,
        env: impl Fn(&str) -> Option<PathBuf>,
    ) -> Self {
        Self {
            server_binary: env(SERVER_BIN_VAR).unwrap_or_else(|| exe_dir.join(exe("server"))),
            agent_binary: env(AGENT_BIN_VAR).unwrap_or_else(|| exe_dir.join(exe("agent"))),
            postgres_dir: env(POSTGRES_DIR_VAR).unwrap_or_else(|| resource_dir.join("postgres")),
            static_dir: env(STATIC_DIR_VAR).unwrap_or_else(|| resource_dir.join("web")),
            docs_dir: optional(env(DOCS_DIR_VAR), resource_dir.join("docs")).await,
            borg_binary: optional(
                env(BORG_BIN_VAR),
                resource_dir.join("borg").join(exe("borg")),
            )
            .await,
        }
    }
}

/// `overridden` if set, otherwise `bundled` if it exists.
async fn optional(overridden: Option<PathBuf>, bundled: PathBuf) -> Option<PathBuf> {
    if overridden.is_some() {
        return overridden;
    }
    tokio::fs::try_exists(&bundled)
        .await
        .unwrap_or(false)
        .then_some(bundled)
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tauri::async_runtime::block_on;

    use super::*;
    use crate::test_support::ScratchDir;

    /// A resource directory containing exactly `files` (relative paths) and
    /// `dirs`.
    fn resources(files: &[&str], dirs: &[&str]) -> ScratchDir {
        let res = ScratchDir::new("layout");
        for dir in dirs {
            block_on(tokio::fs::create_dir_all(res.path().join(dir))).unwrap();
        }
        for file in files {
            let file = res.path().join(file);
            block_on(tokio::fs::create_dir_all(file.parent().unwrap())).unwrap();
            block_on(tokio::fs::write(file, b"")).unwrap();
        }
        res
    }

    fn resolve(res: &Path, overrides: &[(&str, &str)]) -> BundleLayout {
        let overrides: HashMap<String, PathBuf> = overrides
            .iter()
            .map(|(key, value)| ((*key).to_owned(), PathBuf::from(value)))
            .collect();
        block_on(BundleLayout::resolve(Path::new("/app/bin"), res, |var| {
            overrides.get(var).cloned()
        }))
    }

    fn borg() -> String {
        format!("borg/{}", exe("borg"))
    }

    #[test]
    fn defaults_to_the_bundle() {
        let borg = borg();
        let res = resources(&[borg.as_str()], &["docs"]);
        let layout = resolve(res.path(), &[]);

        assert_eq!(
            layout.server_binary,
            PathBuf::from(format!("/app/bin/{}", exe("server")))
        );
        assert_eq!(
            layout.agent_binary,
            PathBuf::from(format!("/app/bin/{}", exe("agent")))
        );
        assert_eq!(layout.postgres_dir, res.path().join("postgres"));
        assert_eq!(layout.static_dir, res.path().join("web"));
        assert_eq!(layout.docs_dir, Some(res.path().join("docs")));
        assert_eq!(layout.borg_binary, Some(res.path().join(borg)));
    }

    #[test]
    fn optional_parts_missing_from_the_bundle_are_left_out() {
        let res = resources(&[], &[]);
        let layout = resolve(res.path(), &[]);

        assert_eq!(layout.docs_dir, None);
        assert_eq!(layout.borg_binary, None);
    }

    #[test]
    fn environment_overrides_win() {
        let borg = borg();
        let res = resources(&[borg.as_str()], &["docs"]);
        let layout = resolve(
            res.path(),
            &[
                (SERVER_BIN_VAR, "/dev/server"),
                (AGENT_BIN_VAR, "/dev/agent"),
                (POSTGRES_DIR_VAR, "/opt/pg"),
                (STATIC_DIR_VAR, "/dev/dist"),
                (DOCS_DIR_VAR, "/dev/docs"),
                (BORG_BIN_VAR, "/usr/bin/borg"),
            ],
        );

        assert_eq!(layout.server_binary, PathBuf::from("/dev/server"));
        assert_eq!(layout.agent_binary, PathBuf::from("/dev/agent"));
        assert_eq!(layout.postgres_dir, PathBuf::from("/opt/pg"));
        assert_eq!(layout.static_dir, PathBuf::from("/dev/dist"));
        assert_eq!(layout.docs_dir, Some(PathBuf::from("/dev/docs")));
        assert_eq!(layout.borg_binary, Some(PathBuf::from("/usr/bin/borg")));
    }

    #[test]
    fn a_borg_directory_without_the_binary_is_not_bundled_borg() {
        let res = resources(&[], &["borg", "docs"]);
        let layout = resolve(res.path(), &[]);

        assert_eq!(layout.borg_binary, None);
        assert_eq!(layout.docs_dir, Some(res.path().join("docs")));
    }

    #[test]
    fn the_bundled_borg_binary_is_used_when_present() {
        let borg = borg();
        let res = resources(&[borg.as_str()], &[]);
        let layout = resolve(res.path(), &[]);

        assert_eq!(layout.borg_binary, Some(res.path().join(borg)));
        assert_eq!(layout.docs_dir, None);
    }
}
