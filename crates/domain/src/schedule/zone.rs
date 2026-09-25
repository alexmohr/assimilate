// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeDelta, Utc};

/// A timezone as the scheduler needs it: the wall-clock time at an instant,
/// and the earliest instant that shows a given wall-clock time.
pub trait Zone: fmt::Display {
    /// The wall-clock time in this zone at `utc`, if the zone knows it.
    fn local(&self, utc: DateTime<Utc>) -> Option<NaiveDateTime>;

    /// The earliest instant whose wall-clock time in this zone is `local`, or
    /// `None` when `local` falls into a gap (clocks jumped over it).
    fn earliest_instant(&self, local: NaiveDateTime) -> Option<DateTime<Utc>>;
}

#[cfg(feature = "timezones")]
impl Zone for chrono_tz::Tz {
    fn local(&self, utc: DateTime<Utc>) -> Option<NaiveDateTime> {
        Some(utc.with_timezone(self).naive_local())
    }

    fn earliest_instant(&self, local: NaiveDateTime) -> Option<DateTime<Utc>> {
        use chrono::TimeZone;

        self.from_local_datetime(&local)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// A timezone known only through its UTC offset at any instant, such as the
/// browser's `Intl` database. Lets the frontend run the scheduler's logic
/// without shipping a timezone database of its own.
pub struct OffsetZone<F> {
    name: String,
    offset_at: F,
}

impl<F> OffsetZone<F>
where
    F: Fn(DateTime<Utc>) -> Option<FixedOffset>,
{
    /// A zone called `name` whose UTC offset at an instant is `offset_at`.
    pub fn new(name: impl Into<String>, offset_at: F) -> Self {
        Self {
            name: name.into(),
            offset_at,
        }
    }
}

impl<F> fmt::Display for OffsetZone<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// How far either side of a wall-clock time to look for the offsets that could
/// apply to it. Offsets stay within +-14h and no zone changes offset twice
/// within two days, so the offsets in force a day before and after are the
/// only candidates.
const PROBE: TimeDelta = TimeDelta::days(1);

impl<F> Zone for OffsetZone<F>
where
    F: Fn(DateTime<Utc>) -> Option<FixedOffset>,
{
    fn local(&self, utc: DateTime<Utc>) -> Option<NaiveDateTime> {
        let offset = (self.offset_at)(utc)?;
        Some(utc.with_timezone(&offset).naive_local())
    }

    fn earliest_instant(&self, local: NaiveDateTime) -> Option<DateTime<Utc>> {
        let as_utc = local.and_utc();
        [
            as_utc.checked_sub_signed(PROBE),
            as_utc.checked_add_signed(PROBE),
        ]
        .into_iter()
        .flatten()
        .filter_map(|probe| (self.offset_at)(probe))
        .filter_map(|offset| {
            let instant =
                as_utc.checked_sub_signed(TimeDelta::seconds(offset.local_minus_utc().into()))?;
            ((self.offset_at)(instant)? == offset).then_some(instant)
        })
        .min()
    }
}

#[cfg(test)]
#[cfg(feature = "timezones")]
mod tests {
    use chrono::{Datelike, NaiveDate, Offset, TimeZone};
    use chrono_tz::Tz;

    use super::{OffsetZone, Zone};

    /// Offsets only, looked up in chrono-tz, so the resolution logic can be
    /// checked against chrono-tz's own.
    fn offsets_of(
        tz: Tz,
    ) -> OffsetZone<impl Fn(chrono::DateTime<chrono::Utc>) -> Option<chrono::FixedOffset>> {
        OffsetZone::new(tz.name(), move |utc| {
            Some(tz.offset_from_utc_datetime(&utc.naive_utc()).fix())
        })
    }

    /// Every half hour of `year`, which crosses each DST transition, including
    /// Lord Howe's 30-minute one.
    fn half_hours(year: i32) -> impl Iterator<Item = chrono::NaiveDateTime> {
        let start = NaiveDate::from_ymd_opt(year, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        std::iter::successors(Some(start), |t| {
            t.checked_add_signed(chrono::TimeDelta::minutes(30))
        })
        .take_while(move |t| t.year() == year)
    }

    #[test]
    fn resolves_wall_clock_times_exactly_like_chrono_tz() {
        let zones = [
            "UTC",
            "Europe/Berlin",
            "America/New_York",
            "America/Sao_Paulo",
            "Australia/Lord_Howe",
            "Asia/Kolkata",
            "Pacific/Chatham",
            // Skipped 2011-12-30 entirely when it crossed the date line.
            "Pacific/Apia",
        ];
        for name in zones {
            let tz: Tz = name.parse().unwrap();
            let zone = offsets_of(tz);
            for year in [2011, 2026] {
                for local in half_hours(year) {
                    assert_eq!(
                        zone.earliest_instant(local),
                        tz.earliest_instant(local),
                        "{name} at {local}"
                    );
                    let utc = local.and_utc();
                    assert_eq!(zone.local(utc), tz.local(utc), "{name} at {utc}");
                }
            }
        }
    }

    #[test]
    fn a_skipped_day_has_no_instant() {
        let apia = offsets_of("Pacific/Apia".parse().unwrap());
        let skipped = NaiveDate::from_ymd_opt(2011, 12, 30)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert_eq!(apia.earliest_instant(skipped), None);
    }

    #[test]
    fn an_unknown_offset_resolves_nothing() {
        let zone = OffsetZone::new("nowhere", |_| None);
        let local = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(zone.earliest_instant(local), None);
        assert_eq!(zone.local(local.and_utc()), None);
        assert_eq!(zone.to_string(), "nowhere");
    }
}
