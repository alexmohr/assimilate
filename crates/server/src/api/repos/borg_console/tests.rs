// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;

fn argv(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| (*s).to_owned()).collect()
}

fn parse(raw: &[&str]) -> Result<ConsoleCommand, ConsoleCommandError> {
    ConsoleCommand::parse(&argv(raw))
}

fn passed_on(raw: &[&str]) -> Vec<String> {
    parse(raw).unwrap().argv()
}

/// Options that run a command, read or write a local file, reach another
/// repository or change borg's configuration, across borg's subcommands.
const DANGEROUS_OPTIONS: &[&str] = &[
    "--rsh",
    "--remote-path",
    "--upload-ratelimit",
    "--upload-buffer",
    "--debug-profile",
    "--debug-topic",
    "--debug",
    "--patterns-from",
    "--exclude-from",
    "--paths-from-command",
    "--paths-from-stdin",
    "--content-from-command",
    "--stdin-name",
    "--config",
    "--timestamp",
    "--umask",
    "--bypass-lock",
    "--repair",
    "--cache-only",
    "--force",
    "--repo",
    "-r",
    "--rs",
    "--remote",
    "--",
    "-",
];

#[test]
fn every_subcommand_runs_on_its_own() {
    for sub in BorgConsoleSubcommand::ALL {
        assert_eq!(passed_on(&[sub.as_str()]), vec![sub.as_str().to_owned()]);
    }
}

#[test]
fn an_empty_command_is_refused() {
    assert_eq!(parse(&[]), Err(ConsoleCommandError::Empty));
}

#[test]
fn an_unknown_subcommand_is_refused_with_the_permitted_ones() {
    let err = parse(&["serve", "--restrict-to-path", "/"]).unwrap_err();
    assert!(matches!(
        &err,
        ConsoleCommandError::SubcommandNotAllowed { given, .. } if given == "serve"
    ));
    let message = err.to_string();
    for sub in BorgConsoleSubcommand::ALL {
        assert!(message.contains(sub.as_str()), "missing {sub} in {message}");
    }
}

#[test]
fn the_permitted_list_names_every_subcommand_once() {
    let list = permitted_subcommands();
    for sub in BorgConsoleSubcommand::ALL {
        assert!(list.contains(sub.as_str()), "missing {sub} in {list}");
    }
    assert_eq!(
        list.matches(',').count(),
        BorgConsoleSubcommand::ALL.len() - 1
    );
}

#[test]
fn too_many_arguments_are_refused() {
    let mut raw = vec!["list"];
    raw.extend(std::iter::repeat_n("--short", MAX_ARGS));
    assert_eq!(parse(&raw), Err(ConsoleCommandError::TooMany));
}

#[test]
fn every_dangerous_option_is_refused_for_every_subcommand() {
    for sub in BorgConsoleSubcommand::ALL {
        for option in DANGEROUS_OPTIONS {
            let result = parse(&[sub.as_str(), option, "sh -c 'id>/tmp/p' #"]);
            assert!(
                matches!(result, Err(ConsoleCommandError::OptionNotAllowed { .. })),
                "borg {sub} {option}: {result:?}"
            );
        }
    }
}

#[test]
fn a_dangerous_option_with_a_joined_value_is_refused() {
    for sub in BorgConsoleSubcommand::ALL {
        for option in DANGEROUS_OPTIONS.iter().filter(|option| option.len() > 2) {
            let joined = format!("{option}=sh -c 'id>/tmp/p' #");
            let result = parse(&[sub.as_str(), &joined]);
            assert!(
                matches!(
                    &result,
                    Err(ConsoleCommandError::OptionNotAllowed { option: shown, .. })
                        if shown == option
                ),
                "borg {sub} {joined}: {result:?}"
            );
        }
    }
}

#[test]
fn a_dangerous_option_after_positional_arguments_is_refused() {
    for option in [
        "--rsh",
        "--remote-path",
        "--exclude-from",
        "--patterns-from",
    ] {
        let result = parse(&["list", "::nightly", "home/user", option, "x"]);
        assert!(
            matches!(result, Err(ConsoleCommandError::OptionNotAllowed { .. })),
            "{option}: {result:?}"
        );
    }
}

#[test]
fn the_reported_rce_is_refused() {
    assert_eq!(
        parse(&["list", "--rsh", "sh -c 'id>/tmp/p' #"]),
        Err(ConsoleCommandError::OptionNotAllowed {
            subcommand: BorgConsoleSubcommand::List,
            option: "--rsh".to_owned(),
        })
    );
}

#[test]
fn a_permitted_option_with_a_joined_value_is_refused_consistently() {
    assert_eq!(
        parse(&["prune", "--keep-daily=7"]),
        Err(ConsoleCommandError::JoinedValue {
            option: ConsoleOption::KeepDaily
        })
    );
    assert_eq!(
        parse(&["list", "-a=web-*"]),
        Err(ConsoleCommandError::JoinedValue {
            option: ConsoleOption::GlobArchives
        })
    );
    assert_eq!(
        parse(&["info", "--json=yes"]),
        Err(ConsoleCommandError::JoinedValue {
            option: ConsoleOption::Json
        })
    );
}

#[test]
fn clustered_or_attached_short_options_are_refused() {
    for option in ["-sv", "-afoo", "-d7", "-np"] {
        assert!(
            matches!(
                parse(&["prune", option]),
                Err(ConsoleCommandError::OptionNotAllowed { .. })
            ),
            "{option}"
        );
    }
}

#[test]
fn an_option_another_subcommand_takes_is_refused() {
    assert_eq!(
        parse(&["list", "--keep-daily", "7"]),
        Err(ConsoleCommandError::OptionNotAllowed {
            subcommand: BorgConsoleSubcommand::List,
            option: "--keep-daily".to_owned(),
        })
    );
}

#[test]
fn an_option_value_must_be_given_and_be_valid() {
    assert_eq!(
        parse(&["prune", "--keep-daily"]),
        Err(ConsoleCommandError::MissingValue {
            option: ConsoleOption::KeepDaily
        })
    );
    assert_eq!(
        parse(&["prune", "--keep-daily", "seven"]),
        Err(ConsoleCommandError::InvalidValue {
            option: ConsoleOption::KeepDaily,
            reason: ValueError::NotANumber,
        })
    );
    assert_eq!(
        parse(&["list", "--format", "--rsh"]),
        Err(ConsoleCommandError::InvalidValue {
            option: ConsoleOption::Format,
            reason: ValueError::LeadingOptionChar,
        })
    );
    assert_eq!(
        parse(&["recreate", "::a", "--target", "x/y"]),
        Err(ConsoleCommandError::InvalidValue {
            option: ConsoleOption::Target,
            reason: ValueError::SlashInArchiveName,
        })
    );
}

#[test]
fn a_repository_location_is_refused_as_the_repository_argument() {
    for location in [
        "ssh://attacker.example/./repo",
        "ssh://attacker.example/./repo::x",
        "user@host:repo",
        "host:repo::x",
        "/srv/other",
        "/srv/other::x",
        "file:///srv/other",
        "relative-repo",
    ] {
        for sub in BorgConsoleSubcommand::ALL {
            let result = parse(&[sub.as_str(), location]);
            assert!(
                matches!(
                    result,
                    Err(ConsoleCommandError::InvalidArgument { .. }
                        | ConsoleCommandError::UnexpectedArgument { .. })
                ),
                "borg {sub} {location}: {result:?}"
            );
        }
    }
}

#[test]
fn a_repository_location_is_refused_as_a_later_argument() {
    for raw in [
        ["list", "::a", "/etc/shadow"],
        ["list", "::a", "ssh://h/r"],
        ["diff", "::a", "b/c"],
        ["delete", "::a", "../b"],
        ["recreate", "::a", "user@host:r"],
    ] {
        let result = parse(&raw);
        assert!(
            matches!(result, Err(ConsoleCommandError::InvalidArgument { .. })),
            "{raw:?}: {result:?}"
        );
    }
}

#[test]
fn subcommands_refuse_more_positional_arguments_than_they_take() {
    for raw in [
        &["compact", "::a"][..],
        &["prune", "::a"],
        &["info", "::a", "::b"],
        &["check", "::a", "x"],
        &["rename", "::a", "b", "c"],
    ] {
        let result = parse(raw);
        assert!(
            matches!(result, Err(ConsoleCommandError::UnexpectedArgument { .. })),
            "{raw:?}: {result:?}"
        );
    }
}

#[test]
fn an_argument_file_is_refused() {
    assert!(matches!(
        parse(&["list", "@/etc/borg-args"]),
        Err(ConsoleCommandError::InvalidArgument { .. })
    ));
    assert!(matches!(
        parse(&["list", "::a", "@/etc/borg-args"]),
        Err(ConsoleCommandError::InvalidArgument {
            reason: ValueError::LeadingOptionChar,
            ..
        })
    ));
    assert!(matches!(
        parse(&["list", "--format", "@/etc/borg-args"]),
        Err(ConsoleCommandError::InvalidValue {
            reason: ValueError::LeadingOptionChar,
            ..
        })
    ));
}

#[test]
fn allowed_commands_are_passed_on_by_long_option_names() {
    assert_eq!(passed_on(&["list", "--short"]), argv(&["list", "--short"]));
    assert_eq!(
        passed_on(&["info", "--json", "::nightly-2026-03-04T02:00:00"]),
        argv(&["info", "--json", "::nightly-2026-03-04T02:00:00"])
    );
    assert_eq!(
        passed_on(&[
            "prune",
            "-n",
            "-s",
            "--list",
            "-d",
            "7",
            "-w",
            "4",
            "--keep-within",
            "2d"
        ]),
        argv(&[
            "prune",
            "--dry-run",
            "--stats",
            "--list",
            "--keep-daily",
            "7",
            "--keep-weekly",
            "4",
            "--keep-within",
            "2d",
        ])
    );
    assert_eq!(
        passed_on(&[
            "list",
            "-a",
            "web-*",
            "--last",
            "3",
            "--format",
            "{archive}{NL}"
        ]),
        argv(&[
            "list",
            "--glob-archives",
            "web-*",
            "--last",
            "3",
            "--format",
            "{archive}{NL}",
        ])
    );
    assert_eq!(
        passed_on(&["check", "-v", "--repository-only", "--max-duration", "600"]),
        argv(&[
            "check",
            "--info",
            "--repository-only",
            "--max-duration",
            "600"
        ])
    );
    assert_eq!(
        passed_on(&["compact", "--threshold", "10", "--progress"]),
        argv(&["compact", "--threshold", "10", "--progress"])
    );
}

#[test]
fn archives_are_addressed_in_this_repository() {
    assert_eq!(
        passed_on(&["list", "::a", "home/user", "-e", "*.tmp"]),
        argv(&["list", "::a", "home/user", "--exclude", "*.tmp"])
    );
    assert_eq!(
        passed_on(&["diff", "::a", "::b", "etc"]),
        argv(&["diff", "::a", "b", "etc"])
    );
    assert_eq!(
        passed_on(&["delete", "::a", "b", "c"]),
        argv(&["delete", "::a", "b", "c"])
    );
    assert_eq!(
        passed_on(&["rename", "::old", "new"]),
        argv(&["rename", "::old", "new"])
    );
    assert_eq!(
        passed_on(&["recreate", "::a", "--target", "a-slim", "-e", "var/cache"]),
        argv(&[
            "recreate",
            "::a",
            "--target",
            "a-slim",
            "--exclude",
            "var/cache"
        ])
    );
}

#[test]
fn options_may_come_after_positional_arguments() {
    assert_eq!(
        passed_on(&["list", "::a", "--short"]),
        argv(&["list", "::a", "--short"])
    );
}

#[test]
fn the_audited_arguments_leave_out_the_subcommand() {
    let command = parse(&["list", "-a", "web-*"]).unwrap();
    assert_eq!(command.subcommand(), BorgConsoleSubcommand::List);
    assert_eq!(command.args(), argv(&["--glob-archives", "web-*"]));
}

#[test]
fn a_refused_argument_is_shown_shortened() {
    let long = format!("--{}", "x".repeat(500));
    let Err(ConsoleCommandError::OptionNotAllowed { option, .. }) = parse(&["list", &long]) else {
        panic!("expected a refused option");
    };
    assert_eq!(option.chars().count(), MAX_SHOWN_LEN);
}
