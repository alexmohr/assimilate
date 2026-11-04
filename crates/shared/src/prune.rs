// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The retention arguments of `borg prune`, built in one place for the agent,
//! which prunes after each backup, and the server, which prunes append-only
//! repositories on their maintenance schedule.

/// The `--keep-*` arguments for the counts above zero, in borg's order. Empty
/// when no count is above zero: pruning with nothing to keep would remove
/// every archive that matches, so callers skip the prune instead.
#[must_use]
pub fn keep_args(hourly: u32, daily: u32, weekly: u32, monthly: u32, yearly: u32) -> Vec<String> {
    [
        ("--keep-hourly", hourly),
        ("--keep-daily", daily),
        ("--keep-weekly", weekly),
        ("--keep-monthly", monthly),
        ("--keep-yearly", yearly),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .flat_map(|(flag, count)| [flag.to_owned(), count.to_string()])
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_counts_above_zero_are_kept_in_borgs_order() {
        assert_eq!(
            keep_args(0, 7, 0, 12, 1),
            [
                "--keep-daily",
                "7",
                "--keep-monthly",
                "12",
                "--keep-yearly",
                "1"
            ]
        );
    }

    #[test]
    fn nothing_to_keep_gives_no_arguments() {
        assert_eq!(keep_args(0, 0, 0, 0, 0), Vec::<String>::new());
    }
}
