// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A repository's Borg console command, checked before borg sees it.
//!
//! The console runs borg as the server process, which holds the key that
//! decrypts every repository passphrase, so an argument borg would act on
//! beyond the repository (a command to run, a file to read, another
//! repository) must never reach it. Every argument is parsed into an
//! allowlisted option, a checked option value, an archive of this
//! repository or a path inside an archive; the command passed to borg is
//! rebuilt from those, never from the raw strings.

use shared::audit::BorgConsoleSubcommand;

mod options;
mod values;

use self::{
    options::{ConsoleOption, ValueKind},
    values::{ArchiveName, ArchivePath, ValueError},
};

/// The most arguments a console command may have, subcommand included.
const MAX_ARGS: usize = 32;
/// How much of a refused argument an error message repeats.
const MAX_SHOWN_LEN: usize = 64;

/// Why a console command was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum ConsoleCommandError {
    #[error("args must not be empty")]
    Empty,
    #[error("too many arguments")]
    TooMany,
    #[error("subcommand '{given}' is not allowed; permitted: {permitted}")]
    SubcommandNotAllowed { given: String, permitted: String },
    #[error("option '{option}' is not allowed for borg {subcommand}")]
    OptionNotAllowed {
        subcommand: BorgConsoleSubcommand,
        option: String,
    },
    #[error("give the value of '{option}' as the next argument, not joined with '='")]
    JoinedValue { option: ConsoleOption },
    #[error("option '{option}' needs a value")]
    MissingValue { option: ConsoleOption },
    #[error("invalid value for '{option}': {reason}")]
    InvalidValue {
        option: ConsoleOption,
        reason: ValueError,
    },
    #[error("unexpected argument '{arg}': borg {subcommand} takes no further arguments")]
    UnexpectedArgument {
        subcommand: BorgConsoleSubcommand,
        arg: String,
    },
    #[error("invalid argument '{arg}': {reason}")]
    InvalidArgument { arg: String, reason: ValueError },
}

/// What a positional argument stands for, by its place on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The `REPO_OR_ARCHIVE` argument, which the console only takes as
    /// `::ARCHIVE` so borg resolves it against this repository.
    RepoOrArchive,
    /// A further archive, named without the repository.
    Name,
    /// A path or pattern inside an archive.
    Path,
}

/// What the `index`th positional argument of `subcommand` stands for, or
/// `None` if the subcommand takes no argument there.
fn slot(subcommand: BorgConsoleSubcommand, index: usize) -> Option<Slot> {
    let first = index == 0;
    match subcommand {
        BorgConsoleSubcommand::Compact | BorgConsoleSubcommand::Prune => None,
        BorgConsoleSubcommand::Info | BorgConsoleSubcommand::Check => {
            first.then_some(Slot::RepoOrArchive)
        }
        BorgConsoleSubcommand::List | BorgConsoleSubcommand::Recreate => Some(if first {
            Slot::RepoOrArchive
        } else {
            Slot::Path
        }),
        BorgConsoleSubcommand::Delete => Some(if first {
            Slot::RepoOrArchive
        } else {
            Slot::Name
        }),
        BorgConsoleSubcommand::Diff => Some(match index {
            0 => Slot::RepoOrArchive,
            1 => Slot::Name,
            _ => Slot::Path,
        }),
        BorgConsoleSubcommand::Rename => match index {
            0 => Some(Slot::RepoOrArchive),
            1 => Some(Slot::Name),
            _ => None,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OptionValue {
    Number(u32),
    Text(String),
    Archive(ArchiveName),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConsoleArg {
    Flag(ConsoleOption),
    WithValue(ConsoleOption, OptionValue),
    ArchiveRef(ArchiveName),
    ArchiveName(ArchiveName),
    ArchivePath(ArchivePath),
}

impl ConsoleArg {
    fn words(&self) -> Vec<String> {
        match self {
            Self::Flag(option) => vec![option.to_string()],
            Self::WithValue(option, value) => vec![
                option.to_string(),
                match value {
                    OptionValue::Number(n) => n.to_string(),
                    OptionValue::Text(text) => text.clone(),
                    OptionValue::Archive(name) => name.to_string(),
                },
            ],
            Self::ArchiveRef(name) => vec![format!("::{name}")],
            Self::ArchiveName(name) => vec![name.to_string()],
            Self::ArchivePath(path) => vec![path.to_string()],
        }
    }
}

/// A console command every argument of which was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConsoleCommand {
    subcommand: BorgConsoleSubcommand,
    args: Vec<ConsoleArg>,
}

fn shown(raw: &str) -> String {
    raw.chars().take(MAX_SHOWN_LEN).collect()
}

fn permitted_subcommands() -> String {
    BorgConsoleSubcommand::ALL
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl ConsoleCommand {
    /// Parses a console command given as argv, subcommand first.
    ///
    /// # Errors
    ///
    /// Returns the first argument the console will not pass on to borg.
    pub(super) fn parse(raw: &[String]) -> Result<Self, ConsoleCommandError> {
        let (raw_subcommand, rest) = raw.split_first().ok_or(ConsoleCommandError::Empty)?;
        let subcommand = raw_subcommand
            .parse::<BorgConsoleSubcommand>()
            .map_err(|_| ConsoleCommandError::SubcommandNotAllowed {
                given: shown(raw_subcommand),
                permitted: permitted_subcommands(),
            })?;
        if raw.len() > MAX_ARGS {
            return Err(ConsoleCommandError::TooMany);
        }

        let mut args = Vec::with_capacity(rest.len());
        let mut positionals = 0;
        let mut tokens = rest.iter();
        while let Some(token) = tokens.next() {
            let arg = if token.starts_with('-') {
                let option = parse_option(subcommand, token)?;
                match option.value_kind() {
                    None => ConsoleArg::Flag(option),
                    Some(kind) => {
                        let value = tokens
                            .next()
                            .ok_or(ConsoleCommandError::MissingValue { option })?;
                        let value = parse_value(kind, value).map_err(|reason| {
                            ConsoleCommandError::InvalidValue { option, reason }
                        })?;
                        ConsoleArg::WithValue(option, value)
                    }
                }
            } else {
                let arg = parse_positional(subcommand, positionals, token)?;
                positionals = positionals.saturating_add(1);
                arg
            };
            args.push(arg);
        }
        Ok(Self { subcommand, args })
    }

    pub(super) fn subcommand(&self) -> BorgConsoleSubcommand {
        self.subcommand
    }

    /// The arguments after the subcommand, as passed to borg.
    pub(super) fn args(&self) -> Vec<String> {
        self.args.iter().flat_map(ConsoleArg::words).collect()
    }

    /// The whole command line passed to borg, subcommand first.
    pub(super) fn argv(&self) -> Vec<String> {
        std::iter::once(self.subcommand.as_str().to_owned())
            .chain(self.args())
            .collect()
    }
}

/// Parses an option exactly as the allowlist spells it. A value joined with
/// `=` is refused even for a permitted option, so an option is never read
/// two ways.
fn parse_option(
    subcommand: BorgConsoleSubcommand,
    token: &str,
) -> Result<ConsoleOption, ConsoleCommandError> {
    let not_allowed = |option: &str| ConsoleCommandError::OptionNotAllowed {
        subcommand,
        option: shown(option),
    };
    let Ok(option) = token.parse::<ConsoleOption>() else {
        let Some((name, _)) = token.split_once('=') else {
            return Err(not_allowed(token));
        };
        return Err(name
            .parse::<ConsoleOption>()
            .ok()
            .filter(|option| option.is_permitted_for(subcommand))
            .map_or_else(
                || not_allowed(name),
                |option| ConsoleCommandError::JoinedValue { option },
            ));
    };
    if !option.is_permitted_for(subcommand) {
        return Err(not_allowed(token));
    }
    Ok(option)
}

fn parse_value(kind: ValueKind, raw: &str) -> Result<OptionValue, ValueError> {
    match kind {
        ValueKind::Number => values::number(raw).map(OptionValue::Number),
        ValueKind::Interval => values::interval(raw).map(OptionValue::Text),
        ValueKind::Text => values::text(raw).map(OptionValue::Text),
        ValueKind::ArchiveName => ArchiveName::parse(raw).map(OptionValue::Archive),
    }
}

fn parse_positional(
    subcommand: BorgConsoleSubcommand,
    index: usize,
    token: &str,
) -> Result<ConsoleArg, ConsoleCommandError> {
    let Some(slot) = slot(subcommand, index) else {
        return Err(ConsoleCommandError::UnexpectedArgument {
            subcommand,
            arg: shown(token),
        });
    };
    match slot {
        Slot::RepoOrArchive => ArchiveName::parse_ref(token).map(ConsoleArg::ArchiveRef),
        Slot::Name => ArchiveName::parse(token).map(ConsoleArg::ArchiveName),
        Slot::Path => ArchivePath::parse(token).map(ConsoleArg::ArchivePath),
    }
    .map_err(|reason| ConsoleCommandError::InvalidArgument {
        arg: shown(token),
        reason,
    })
}

#[cfg(test)]
mod tests;
