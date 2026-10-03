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
    /// bundled default. Optional parts are only used if they exist.
    pub fn resolve(
        exe_dir: &Path,
        resource_dir: &Path,
        env: impl Fn(&str) -> Option<PathBuf>,
        exists: impl Fn(&Path) -> bool,
    ) -> Self {
        let optional =
            |var: &str, default: PathBuf| env(var).or_else(|| exists(&default).then_some(default));
        Self {
            server_binary: env(SERVER_BIN_VAR).unwrap_or_else(|| exe_dir.join(exe("server"))),
            agent_binary: env(AGENT_BIN_VAR).unwrap_or_else(|| exe_dir.join(exe("agent"))),
            postgres_dir: env(POSTGRES_DIR_VAR).unwrap_or_else(|| resource_dir.join("postgres")),
            static_dir: env(STATIC_DIR_VAR).unwrap_or_else(|| resource_dir.join("web")),
            docs_dir: optional(DOCS_DIR_VAR, resource_dir.join("docs")),
            borg_binary: optional(BORG_BIN_VAR, resource_dir.join("borg").join(exe("borg"))),
        }
    }
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn resolve(overrides: &[(&str, &str)], existing: &[&str]) -> BundleLayout {
        let overrides: HashMap<String, PathBuf> = overrides
            .iter()
            .map(|(key, value)| ((*key).to_owned(), PathBuf::from(value)))
            .collect();
        let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
        BundleLayout::resolve(
            Path::new("/app/bin"),
            Path::new("/app/res"),
            |var| overrides.get(var).cloned(),
            |path| existing.iter().any(|candidate| candidate == path),
        )
    }

    #[test]
    fn defaults_to_the_bundle() {
        let layout = resolve(&[], &["/app/res/docs", "/app/res/borg/borg"]);

        assert_eq!(layout.server_binary, PathBuf::from("/app/bin/server"));
        assert_eq!(layout.agent_binary, PathBuf::from("/app/bin/agent"));
        assert_eq!(layout.postgres_dir, PathBuf::from("/app/res/postgres"));
        assert_eq!(layout.static_dir, PathBuf::from("/app/res/web"));
        assert_eq!(layout.docs_dir, Some(PathBuf::from("/app/res/docs")));
        assert_eq!(
            layout.borg_binary,
            Some(PathBuf::from("/app/res/borg/borg"))
        );
    }

    #[test]
    fn optional_parts_missing_from_the_bundle_are_left_out() {
        let layout = resolve(&[], &[]);

        assert_eq!(layout.docs_dir, None);
        assert_eq!(layout.borg_binary, None);
    }

    #[test]
    fn environment_overrides_win() {
        let layout = resolve(
            &[
                (SERVER_BIN_VAR, "/dev/server"),
                (AGENT_BIN_VAR, "/dev/agent"),
                (POSTGRES_DIR_VAR, "/opt/pg"),
                (STATIC_DIR_VAR, "/dev/dist"),
                (DOCS_DIR_VAR, "/dev/docs"),
                (BORG_BIN_VAR, "/usr/bin/borg"),
            ],
            &[],
        );

        assert_eq!(layout.server_binary, PathBuf::from("/dev/server"));
        assert_eq!(layout.agent_binary, PathBuf::from("/dev/agent"));
        assert_eq!(layout.postgres_dir, PathBuf::from("/opt/pg"));
        assert_eq!(layout.static_dir, PathBuf::from("/dev/dist"));
        assert_eq!(layout.docs_dir, Some(PathBuf::from("/dev/docs")));
        assert_eq!(layout.borg_binary, Some(PathBuf::from("/usr/bin/borg")));
    }
}
