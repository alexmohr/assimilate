// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Editing a repository host's `authorized_keys` for append-only access.
//!
//! The host enforces append-only, not borg: the key the agents borrow from the
//! server is restricted to `borg serve --append-only`, and the server's second,
//! maintenance key is a plain line that keeps shell and SFTP access. These
//! functions take the file's current text and return the new text, so the
//! edits can be tested without a host. Lines Assimilate does not manage,
//! comments and blank lines included, are kept as they are.

use ssh_key::PublicKey;

use super::shell_escape;

/// The comment on the maintenance key's line, which marks it as Assimilate's.
pub const MAINTENANCE_COMMENT: &str = "assimilate-maintenance";

/// Why a line for the host could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthorizedKeysError {
    /// A repository path would break out of the forced command.
    #[error("repository path {0:?} contains a character that cannot be used in a forced command")]
    UnsafePath(String),
    /// There is no repository to restrict the key to.
    #[error("no repository paths to restrict the key to")]
    NoRepositories,
}

/// What the host's `authorized_keys` says about the key the agents use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restriction {
    /// The key is not in the file.
    Absent,
    /// The key is in the file with no forced command.
    Unrestricted,
    /// The key is forced to `borg serve --append-only`.
    AppendOnly,
    /// The key has a forced command that is not append-only borg.
    Other,
}

/// The absolute path borg sees for a repository: the URL Assimilate builds
/// drops any leading slash and borg reads the rest as absolute.
#[must_use]
pub fn borg_repo_path(repo_path: &str) -> String {
    format!("/{}", repo_path.trim_start_matches('/'))
}

/// The forced command that makes a connection append-only and limits it to
/// `repo_paths`.
///
/// # Errors
///
/// Returns [`AuthorizedKeysError::NoRepositories`] for an empty list, and
/// [`AuthorizedKeysError::UnsafePath`] for a path with a quote, backslash or
/// control character, which would end the option's value early.
pub fn forced_command(repo_paths: &[String]) -> Result<String, AuthorizedKeysError> {
    if repo_paths.is_empty() {
        return Err(AuthorizedKeysError::NoRepositories);
    }
    let restrictions = repo_paths
        .iter()
        .map(|path| {
            let path = borg_repo_path(path);
            if path
                .chars()
                .any(|c| c == '"' || c == '\\' || c.is_control())
            {
                return Err(AuthorizedKeysError::UnsafePath(path));
            }
            Ok(format!("--restrict-to-repository {}", shell_escape(&path)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "borg serve --append-only {}",
        restrictions.join(" ")
    ))
}

fn with_comment(key: &PublicKey, comment: &str) -> PublicKey {
    let mut key = key.clone();
    key.set_comment(comment);
    key
}

fn key_line(key: &PublicKey) -> String {
    // A key that cannot be written out has no sane line; the empty string is
    // never a valid replacement, and `to_openssh` only fails on I/O.
    key.to_openssh().unwrap_or_default()
}

/// The line that restricts the agents' key to append-only access to
/// `repo_paths`, keeping the key's own comment.
///
/// # Errors
///
/// See [`forced_command`].
pub fn restricted_line(
    backup: &PublicKey,
    repo_paths: &[String],
) -> Result<String, AuthorizedKeysError> {
    let command = forced_command(repo_paths)?;
    Ok(format!(
        "command=\"{command}\",restrict {}",
        key_line(backup)
    ))
}

/// The maintenance key's line: unrestricted, so the server keeps its shell
/// and SFTP access to the host.
#[must_use]
pub fn maintenance_line(maintenance: &PublicKey) -> String {
    key_line(&with_comment(maintenance, MAINTENANCE_COMMENT))
}

/// A line of the file that holds a key: its options, and the key.
struct KeyLine<'a> {
    options: &'a str,
    key: PublicKey,
}

/// Reads a line as sshd does: optional comma-separated options, which may
/// hold spaces inside double quotes, then the key. Anything else - a comment,
/// a blank line, text that is not a key - is `None`.
///
/// `ssh_key`'s own parser cuts the options at the first space, so it cannot
/// read a `command="borg serve ..."` option.
fn parse_line(line: &str) -> Option<KeyLine<'_>> {
    let line = line.trim();
    if let Ok(key) = PublicKey::from_openssh(line) {
        return Some(KeyLine { options: "", key });
    }
    let mut in_quotes = false;
    let mut escaped = false;
    let end = line.char_indices().find_map(|(index, c)| {
        if escaped {
            escaped = false;
            return None;
        }
        match c {
            '\\' if in_quotes => {
                escaped = true;
                None
            }
            '"' => {
                in_quotes = !in_quotes;
                None
            }
            c if c.is_whitespace() && !in_quotes => Some(index),
            _ => None,
        }
    })?;
    let (options, key) = line.split_at(end);
    Some(KeyLine {
        options,
        key: PublicKey::from_openssh(key.trim()).ok()?,
    })
}

fn holds_key(line: &str, key: &PublicKey) -> bool {
    parse_line(line).is_some_and(|parsed| parsed.key.key_data() == key.key_data())
}

fn join(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn lines_of(existing: &str) -> Vec<String> {
    existing.lines().map(str::to_owned).collect()
}

/// Adds the maintenance key's line unless the file already holds the key.
#[must_use]
pub fn with_maintenance_key(existing: &str, maintenance: &PublicKey) -> String {
    let mut lines = lines_of(existing);
    if !lines.iter().any(|line| holds_key(line, maintenance)) {
        lines.push(maintenance_line(maintenance));
    }
    join(&lines)
}

/// Replaces the agents' key's line with the restricted one for `repo_paths`,
/// or adds it if the file does not hold the key.
///
/// # Errors
///
/// See [`forced_command`].
pub fn with_restricted_backup_key(
    existing: &str,
    backup: &PublicKey,
    repo_paths: &[String],
) -> Result<String, AuthorizedKeysError> {
    let restricted = restricted_line(backup, repo_paths)?;
    Ok(replace_key_lines(existing, backup, &restricted))
}

/// Turns append-only off: the agents' key becomes a plain line again and the
/// maintenance key's line is removed.
#[must_use]
pub fn without_append_only(existing: &str, backup: &PublicKey, maintenance: &PublicKey) -> String {
    let plain = replace_key_lines(existing, backup, &key_line(backup));
    let kept: Vec<String> = plain
        .lines()
        .filter(|line| !holds_key(line, maintenance))
        .map(str::to_owned)
        .collect();
    join(&kept)
}

/// Replaces every line that holds `key` with `replacement` (once), or appends
/// it when none does.
fn replace_key_lines(existing: &str, key: &PublicKey, replacement: &str) -> String {
    let mut replaced = false;
    let mut lines: Vec<String> = lines_of(existing)
        .into_iter()
        .filter_map(|line| {
            if !holds_key(&line, key) {
                return Some(line);
            }
            if replaced {
                return None;
            }
            replaced = true;
            Some(replacement.to_owned())
        })
        .collect();
    if !replaced {
        lines.push(replacement.to_owned());
    }
    join(&lines)
}

/// What the file says about the agents' key.
#[must_use]
pub fn restriction_of(existing: &str, backup: &PublicKey) -> Restriction {
    existing
        .lines()
        .filter_map(parse_line)
        .find(|parsed| parsed.key.key_data() == backup.key_data())
        .map_or(Restriction::Absent, |parsed| {
            let options = parsed.options;
            if !options.contains("command=") {
                Restriction::Unrestricted
            } else if options.contains("borg serve") && options.contains("--append-only") {
                Restriction::AppendOnly
            } else {
                Restriction::Other
            }
        })
}

#[cfg(test)]
mod tests {
    use ssh_key::{Algorithm, PrivateKey, rand_core::OsRng};

    use super::*;

    fn key(comment: &str) -> PublicKey {
        let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone();
        key.set_comment(comment);
        key
    }

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn an_escaped_quote_inside_an_option_does_not_end_it() {
        let key = key("someone");
        let line = format!(
            "command=\"echo \\\"two words\\\" done\",no-pty {}",
            key.to_openssh().unwrap()
        );
        let parsed = parse_line(&line).expect("a line with options");
        assert_eq!(
            parsed.options,
            "command=\"echo \\\"two words\\\" done\",no-pty"
        );
        assert_eq!(parsed.key.key_data(), key.key_data());
        assert!(holds_key(&line, &key));
    }

    #[test]
    fn a_line_whose_options_never_close_is_not_a_key() {
        let key = key("someone");
        let line = format!("command=\"borg serve {}", key.to_openssh().unwrap());
        assert!(parse_line(&line).is_none());
    }

    #[test]
    fn repo_paths_are_absolute_whether_or_not_they_were_typed_that_way() {
        assert_eq!(borg_repo_path("srv/borg/a"), "/srv/borg/a");
        assert_eq!(borg_repo_path("/srv/borg/a"), "/srv/borg/a");
    }

    #[test]
    fn forced_command_restricts_to_each_repository() {
        let command = forced_command(&paths(&["/srv/borg/a", "srv/borg/b"])).unwrap();
        assert_eq!(
            command,
            "borg serve --append-only --restrict-to-repository '/srv/borg/a' \
             --restrict-to-repository '/srv/borg/b'"
        );
    }

    #[test]
    fn forced_command_quotes_a_path_with_a_space() {
        let command = forced_command(&paths(&["/srv/my repos/a"])).unwrap();
        assert!(command.ends_with("'/srv/my repos/a'"), "{command}");
    }

    #[test]
    fn forced_command_refuses_a_path_that_could_end_the_option_early() {
        for bad in ["/srv/a\"b", "/srv/a\\b", "/srv/a\nb"] {
            assert_eq!(
                forced_command(&paths(&[bad])),
                Err(AuthorizedKeysError::UnsafePath(borg_repo_path(bad))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn forced_command_needs_a_repository() {
        assert_eq!(
            forced_command(&[]),
            Err(AuthorizedKeysError::NoRepositories)
        );
    }

    #[test]
    fn a_single_quote_in_a_path_is_escaped_for_the_shell() {
        let command = forced_command(&paths(&["/srv/it's"])).unwrap();
        assert!(command.ends_with("'/srv/it'\\''s'"), "{command}");
    }

    #[test]
    fn restricting_replaces_the_servers_plain_line_and_keeps_everything_else() {
        let backup = key("assimilate-server");
        let other = key("alice@laptop");
        let existing = format!("# my keys\n\n{}\n{}\n", key_line(&other), key_line(&backup));
        let updated =
            with_restricted_backup_key(&existing, &backup, &paths(&["/srv/borg/a"])).unwrap();

        let mut lines = updated.lines();
        assert_eq!(lines.next(), Some("# my keys"));
        assert_eq!(lines.next(), Some(""));
        assert_eq!(lines.next(), Some(key_line(&other).as_str()));
        let restricted = lines.next().expect("the restricted line");
        assert!(restricted.starts_with("command=\"borg serve --append-only"));
        assert!(restricted.contains(",restrict ssh-ed25519 "));
        assert_eq!(lines.next(), None);
        assert_eq!(restriction_of(&updated, &backup), Restriction::AppendOnly);
    }

    #[test]
    fn restricting_twice_changes_nothing_further() {
        let backup = key("assimilate-server");
        let once = with_restricted_backup_key("", &backup, &paths(&["/srv/a"])).unwrap();
        let twice = with_restricted_backup_key(&once, &backup, &paths(&["/srv/a"])).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn restricting_updates_the_paths() {
        let backup = key("assimilate-server");
        let once = with_restricted_backup_key("", &backup, &paths(&["/srv/a"])).unwrap();
        let twice =
            with_restricted_backup_key(&once, &backup, &paths(&["/srv/a", "/srv/b"])).unwrap();
        assert_eq!(twice.lines().count(), 1);
        assert!(twice.contains("'/srv/b'"));
    }

    #[test]
    fn restricting_collapses_duplicate_lines_of_the_key() {
        let backup = key("assimilate-server");
        let existing = format!("{}\n{}\n", key_line(&backup), key_line(&backup));
        let updated = with_restricted_backup_key(&existing, &backup, &paths(&["/srv/a"])).unwrap();
        assert_eq!(updated.lines().count(), 1);
    }

    #[test]
    fn restricting_adds_the_key_when_the_file_lacks_it() {
        let backup = key("assimilate-server");
        let updated = with_restricted_backup_key("", &backup, &paths(&["/srv/a"])).unwrap();
        assert_eq!(restriction_of(&updated, &backup), Restriction::AppendOnly);
    }

    #[test]
    fn the_maintenance_line_is_plain_and_marked() {
        let maintenance = key("whatever");
        let line = maintenance_line(&maintenance);
        assert!(line.starts_with("ssh-ed25519 "), "{line}");
        assert!(line.ends_with(MAINTENANCE_COMMENT), "{line}");
    }

    #[test]
    fn the_maintenance_key_is_added_once() {
        let maintenance = key("m");
        let existing = format!("{}\n", key_line(&key("alice")));
        let once = with_maintenance_key(&existing, &maintenance);
        let twice = with_maintenance_key(&once, &maintenance);
        assert_eq!(once, twice);
        assert_eq!(once.lines().count(), 2);
    }

    #[test]
    fn a_file_without_a_trailing_newline_is_extended_on_its_own_line() {
        let maintenance = key("m");
        let alice = key_line(&key("alice"));
        let updated = with_maintenance_key(&alice, &maintenance);
        assert_eq!(updated.lines().next(), Some(alice.as_str()));
        assert_eq!(updated.lines().count(), 2);
    }

    #[test]
    fn turning_append_only_off_restores_the_plain_key_and_drops_the_maintenance_key() {
        let backup = key("assimilate-server");
        let maintenance = key("m");
        let other = key("alice");
        let on = with_maintenance_key(
            &with_restricted_backup_key(
                &format!("{}\n", key_line(&other)),
                &backup,
                &paths(&["/srv/a"]),
            )
            .unwrap(),
            &maintenance,
        );
        let off = without_append_only(&on, &backup, &maintenance);

        assert_eq!(restriction_of(&off, &backup), Restriction::Unrestricted);
        assert!(!off.contains(MAINTENANCE_COMMENT));
        assert!(off.contains(&key_line(&other)));
        assert_eq!(off.lines().count(), 2);
    }

    #[test]
    fn restriction_distinguishes_the_states_of_the_key() {
        let backup = key("assimilate-server");
        assert_eq!(restriction_of("", &backup), Restriction::Absent);
        assert_eq!(
            restriction_of(&key_line(&backup), &backup),
            Restriction::Unrestricted
        );
        let other_command = format!("command=\"/bin/true\" {}", key_line(&backup));
        assert_eq!(restriction_of(&other_command, &backup), Restriction::Other);
        let no_append_only = format!("command=\"borg serve\",restrict {}", key_line(&backup));
        assert_eq!(restriction_of(&no_append_only, &backup), Restriction::Other);
    }

    #[test]
    fn a_line_that_is_not_a_key_is_left_alone() {
        let backup = key("assimilate-server");
        let existing = "this is not a key\n";
        let updated = with_restricted_backup_key(existing, &backup, &paths(&["/srv/a"])).unwrap();
        assert!(updated.starts_with("this is not a key\n"));
    }
}
