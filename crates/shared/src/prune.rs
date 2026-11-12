// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The retention arguments of `borg prune`, built in one place for the agent,
//! which prunes after each backup, and the server, which prunes append-only
//! repositories on their maintenance schedule.

/// How many archives of each age bucket `borg prune` keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retention {
    /// Hourly archives to keep.
    pub hourly: u32,
    /// Daily archives to keep.
    pub daily: u32,
    /// Weekly archives to keep.
    pub weekly: u32,
    /// Monthly archives to keep.
    pub monthly: u32,
    /// Yearly archives to keep.
    pub yearly: u32,
}

impl Retention {
    /// Whether every count is zero, so nothing is kept.
    #[must_use]
    pub fn is_unset(self) -> bool {
        self == Self::default()
    }

    /// The bucket-wise maximum: keeps everything either side would keep.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            hourly: self.hourly.max(other.hourly),
            daily: self.daily.max(other.daily),
            weekly: self.weekly.max(other.weekly),
            monthly: self.monthly.max(other.monthly),
            yearly: self.yearly.max(other.yearly),
        }
    }

    /// The `--keep-*` arguments for the counts above zero; see [`keep_args`].
    #[must_use]
    pub fn keep_args(self) -> Vec<String> {
        keep_args(
            self.hourly,
            self.daily,
            self.weekly,
            self.monthly,
            self.yearly,
        )
    }
}

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
    fn a_union_keeps_the_most_either_side_keeps() {
        let a = Retention {
            daily: 7,
            monthly: 3,
            ..Retention::default()
        };
        let b = Retention {
            hourly: 24,
            daily: 2,
            weekly: 4,
            yearly: 1,
            ..Retention::default()
        };
        assert_eq!(
            a.union(b),
            Retention {
                hourly: 24,
                daily: 7,
                weekly: 4,
                monthly: 3,
                yearly: 1
            }
        );
        assert!(Retention::default().is_unset());
        assert!(!a.is_unset());
    }

    #[test]
    fn nothing_to_keep_gives_no_arguments() {
        assert_eq!(keep_args(0, 0, 0, 0, 0), Vec::<String>::new());
    }
}
