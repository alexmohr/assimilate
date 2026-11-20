// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The instant type used across the workspace, built on [`jiff`].
//!
//! [`Timestamp`] wraps [`jiff::Timestamp`] so it can carry the Postgres,
//! `ts-rs` and `utoipa` integrations, none of which know about jiff on their
//! own. Arithmetic saturates at the representable range instead of panicking.

use std::{
    fmt,
    ops::{Add, AddAssign, Sub, SubAssign},
    str::FromStr,
};

pub use jiff::SignedDuration;

/// A UTC instant, serialized as an RFC 3339 string.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(transparent)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// The Unix epoch, `1970-01-01T00:00:00Z`.
    pub const UNIX_EPOCH: Self = Self(jiff::Timestamp::UNIX_EPOCH);

    /// The earliest representable instant.
    pub const MIN: Self = Self(jiff::Timestamp::MIN);

    /// The latest representable instant.
    pub const MAX: Self = Self(jiff::Timestamp::MAX);

    /// The instant at the given UTC date and time, or `None` when it is not real.
    #[must_use]
    pub fn utc(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> Option<Self> {
        jiff::civil::DateTime::new(year, month, day, hour, minute, second, 0)
            .ok()?
            .to_zoned(jiff::tz::TimeZone::UTC)
            .ok()
            .map(|zoned| Self(zoned.timestamp()))
    }

    /// The current instant.
    #[must_use]
    pub fn now() -> Self {
        Self(jiff::Timestamp::now())
    }

    /// The instant `seconds` after the Unix epoch, or `None` out of range.
    #[must_use]
    pub fn from_unix_secs(seconds: i64) -> Option<Self> {
        jiff::Timestamp::from_second(seconds).ok().map(Self)
    }

    /// The instant `micros` microseconds after the Unix epoch, or `None` out of range.
    #[must_use]
    pub fn from_unix_micros(micros: i64) -> Option<Self> {
        jiff::Timestamp::from_microsecond(micros).ok().map(Self)
    }

    /// The instant `millis` milliseconds after the Unix epoch, or `None` out of range.
    #[must_use]
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        jiff::Timestamp::from_millisecond(millis).ok().map(Self)
    }

    /// Whole seconds since the Unix epoch (rounded toward negative infinity).
    #[must_use]
    pub fn unix_secs(self) -> i64 {
        self.0.as_second()
    }

    /// Whole milliseconds since the Unix epoch.
    #[must_use]
    pub fn unix_millis(self) -> i64 {
        self.0.as_millisecond()
    }

    /// Whole microseconds since the Unix epoch.
    #[must_use]
    pub fn unix_micros(self) -> i64 {
        self.0.as_microsecond()
    }

    /// The wrapped [`jiff::Timestamp`].
    #[must_use]
    pub const fn into_inner(self) -> jiff::Timestamp {
        self.0
    }

    /// This instant moved by `delta`, or `None` out of range.
    #[must_use]
    pub fn checked_add(self, delta: SignedDuration) -> Option<Self> {
        self.0.checked_add(delta).ok().map(Self)
    }

    /// This instant moved back by `delta`, or `None` out of range.
    #[must_use]
    pub fn checked_sub(self, delta: SignedDuration) -> Option<Self> {
        self.0.checked_sub(delta).ok().map(Self)
    }

    /// The UTC calendar day this instant falls on.
    #[must_use]
    pub fn utc_date(self) -> Date {
        Date(self.0.to_zoned(jiff::tz::TimeZone::UTC).date())
    }

    /// This instant formatted in UTC with a `strftime` pattern.
    #[must_use]
    pub fn strftime(self, pattern: &str) -> String {
        self.0.strftime(pattern).to_string()
    }

    /// This instant moved by `delta`, clamped to the representable range.
    #[must_use]
    pub fn saturating_add(self, delta: SignedDuration) -> Self {
        Self(self.0.checked_add(delta).unwrap_or(if delta.is_negative() {
            jiff::Timestamp::MIN
        } else {
            jiff::Timestamp::MAX
        }))
    }

    /// This instant moved back by `delta`, clamped to the representable range.
    #[must_use]
    pub fn saturating_sub(self, delta: SignedDuration) -> Self {
        Self(self.0.checked_sub(delta).unwrap_or(if delta.is_negative() {
            jiff::Timestamp::MAX
        } else {
            jiff::Timestamp::MIN
        }))
    }

    /// The time from `earlier` to `self`; negative when `earlier` is later.
    #[must_use]
    pub fn since(self, earlier: Self) -> SignedDuration {
        self.0.duration_since(earlier.0)
    }

    /// This instant in `tz`.
    #[must_use]
    pub fn to_zoned(self, tz: jiff::tz::TimeZone) -> jiff::Zoned {
        self.0.to_zoned(tz)
    }

    /// This instant keeping only the first `digits` fractional-second digits (at most 9).
    #[must_use]
    pub fn trunc_subsecs(self, digits: u32) -> Self {
        let nanos = i64::from(self.0.subsec_nanosecond());
        let unit = 10i64.pow(9u32.saturating_sub(digits.min(9)));
        let kept = nanos.saturating_sub(nanos.rem_euclid(unit));
        jiff::Timestamp::new(self.0.as_second(), i32::try_from(kept).unwrap_or(0))
            .map_or(self, Self)
    }

    /// The RFC 3339 form, e.g. `2026-01-01T00:00:00Z`.
    #[must_use]
    pub fn to_rfc3339(self) -> String {
        self.0.to_string()
    }
}

impl From<jiff::Timestamp> for Timestamp {
    fn from(value: jiff::Timestamp) -> Self {
        Self(value)
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(value: Timestamp) -> Self {
        value.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl TryFrom<std::time::SystemTime> for Timestamp {
    type Error = jiff::Error;

    fn try_from(value: std::time::SystemTime) -> Result<Self, Self::Error> {
        jiff::Timestamp::try_from(value).map(Self)
    }
}

impl FromStr for Timestamp {
    type Err = jiff::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<jiff::Timestamp>().map(Self)
    }
}

impl Add<SignedDuration> for Timestamp {
    type Output = Self;

    fn add(self, rhs: SignedDuration) -> Self {
        self.saturating_add(rhs)
    }
}

impl Sub<SignedDuration> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: SignedDuration) -> Self {
        self.saturating_sub(rhs)
    }
}

impl Sub for Timestamp {
    type Output = SignedDuration;

    fn sub(self, rhs: Self) -> SignedDuration {
        self.since(rhs)
    }
}

impl AddAssign<SignedDuration> for Timestamp {
    fn add_assign(&mut self, rhs: SignedDuration) {
        *self = self.saturating_add(rhs);
    }
}

impl SubAssign<SignedDuration> for Timestamp {
    fn sub_assign(&mut self, rhs: SignedDuration) {
        *self = self.saturating_sub(rhs);
    }
}

/// A calendar day with no time or zone, serialized as `YYYY-MM-DD`.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(transparent)]
pub struct Date(jiff::civil::Date);

impl Date {
    /// The day `year`-`month`-`day`, or `None` when it is not a real date.
    #[must_use]
    pub fn new(year: i16, month: i8, day: i8) -> Option<Self> {
        jiff::civil::Date::new(year, month, day).ok().map(Self)
    }

    /// The first and the day after the last of `month` in `year`, or `None` when
    /// either is out of range.
    #[must_use]
    pub fn month_bounds(year: i32, month: u32) -> Option<(Self, Self)> {
        let first = Self::new(i16::try_from(year).ok()?, i8::try_from(month).ok()?, 1)?;
        let next = first.0.checked_add(jiff::Span::new().months(1)).ok()?;
        Some((first, Self(next)))
    }

    /// The wrapped [`jiff::civil::Date`].
    #[must_use]
    pub const fn into_inner(self) -> jiff::civil::Date {
        self.0
    }

    /// This day formatted with a `strftime` pattern.
    #[must_use]
    pub fn strftime(self, pattern: &str) -> String {
        self.0.strftime(pattern).to_string()
    }
}

impl From<jiff::civil::Date> for Date {
    fn from(value: jiff::civil::Date) -> Self {
        Self(value)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl FromStr for Date {
    type Err = jiff::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<jiff::civil::Date>().map(Self)
    }
}

/// Whole-unit views of a [`SignedDuration`] that jiff itself does not spell out.
pub trait SignedDurationExt {
    /// Whole minutes, truncated toward zero.
    fn as_mins(&self) -> i64;
    /// Whole hours, truncated toward zero.
    fn as_hours(&self) -> i64;
    /// Whole 24-hour days, truncated toward zero.
    fn as_days(&self) -> i64;
}

impl SignedDurationExt for SignedDuration {
    fn as_mins(&self) -> i64 {
        self.as_secs() / 60
    }

    fn as_hours(&self) -> i64 {
        self.as_secs() / 3600
    }

    fn as_days(&self) -> i64 {
        self.as_secs() / 86_400
    }
}

/// Both types travel as strings, so TypeScript sees plain `string` rather than a named alias.
macro_rules! string_in_typescript {
    ($($ty:ty),*) => {$(
        impl ts_rs::TS for $ty {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;

            fn name(_: &ts_rs::Config) -> String {
                String::from("string")
            }

            fn inline(cfg: &ts_rs::Config) -> String {
                <Self as ts_rs::TS>::name(cfg)
            }
        }
    )*};
}

string_in_typescript!(Timestamp, Date);

/// `n` days as a duration (24-hour days), clamped to the representable range.
#[must_use]
pub const fn days(n: i64) -> SignedDuration {
    hours(n.saturating_mul(24))
}

/// `n` hours as a duration, clamped to the representable range.
#[must_use]
pub const fn hours(n: i64) -> SignedDuration {
    seconds(n.saturating_mul(3600))
}

/// `n` minutes as a duration, clamped to the representable range.
#[must_use]
pub const fn minutes(n: i64) -> SignedDuration {
    seconds(n.saturating_mul(60))
}

/// `n` seconds as a duration.
#[must_use]
pub const fn seconds(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

/// `n` milliseconds as a duration.
#[must_use]
pub const fn millis(n: i64) -> SignedDuration {
    SignedDuration::from_millis(n)
}

#[cfg(feature = "sqlx")]
mod pg {
    use std::mem;

    use sqlx::{
        Decode, Encode, Postgres, Type,
        encode::IsNull,
        error::BoxDynError,
        postgres::{
            PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueFormat, PgValueRef, types::Oid,
        },
    };

    use super::Timestamp;

    /// Microseconds from the Unix epoch to the Postgres epoch, 2000-01-01.
    const POSTGRES_EPOCH_MICROS: i64 = 946_684_800_000_000;
    /// Built-in OID of `timestamptz`.
    const TIMESTAMPTZ_OID: u32 = 1184;
    /// Built-in OID of `timestamptz[]`.
    const TIMESTAMPTZ_ARRAY_OID: u32 = 1185;

    impl Type<Postgres> for Timestamp {
        fn type_info() -> PgTypeInfo {
            PgTypeInfo::with_oid(Oid(TIMESTAMPTZ_OID))
        }
    }

    impl PgHasArrayType for Timestamp {
        fn array_type_info() -> PgTypeInfo {
            PgTypeInfo::with_oid(Oid(TIMESTAMPTZ_ARRAY_OID))
        }
    }

    impl Encode<'_, Postgres> for Timestamp {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            let micros = self
                .unix_micros()
                .checked_sub(POSTGRES_EPOCH_MICROS)
                .ok_or_else(|| format!("timestamp out of range for Postgres: {self}"))?;
            Encode::<Postgres>::encode(micros, buf)
        }

        fn size_hint(&self) -> usize {
            mem::size_of::<i64>()
        }
    }

    impl<'r> Decode<'r, Postgres> for Timestamp {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            match value.format() {
                PgValueFormat::Binary => {
                    let micros: i64 = Decode::<Postgres>::decode(value)?;
                    let unix = micros
                        .checked_add(POSTGRES_EPOCH_MICROS)
                        .ok_or("timestamptz out of range")?;
                    Timestamp::from_unix_micros(unix)
                        .ok_or_else(|| "timestamptz out of range".into())
                }
                PgValueFormat::Text => {
                    // Postgres renders `2026-01-01 00:00:00+00`; jiff wants a `T` or space
                    // separator and a full offset, which `+00` is not.
                    let text = value.as_str()?;
                    let parsed: jiff::Timestamp =
                        jiff::fmt::strtime::parse("%Y-%m-%d %H:%M:%S%.f%#z", text)?
                            .to_timestamp()?;
                    Ok(Timestamp(parsed))
                }
            }
        }
    }
}

#[cfg(feature = "sqlx")]
mod pg_date {
    use std::mem;

    use sqlx::{
        Decode, Encode, Postgres, Type,
        encode::IsNull,
        error::BoxDynError,
        postgres::{
            PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueFormat, PgValueRef, types::Oid,
        },
    };

    use super::Date;

    /// Built-in OID of `date`.
    const DATE_OID: u32 = 1082;
    /// Built-in OID of `date[]`.
    const DATE_ARRAY_OID: u32 = 1182;

    fn postgres_epoch() -> jiff::civil::Date {
        jiff::civil::date(2000, 1, 1)
    }

    impl Type<Postgres> for Date {
        fn type_info() -> PgTypeInfo {
            PgTypeInfo::with_oid(Oid(DATE_OID))
        }
    }

    impl PgHasArrayType for Date {
        fn array_type_info() -> PgTypeInfo {
            PgTypeInfo::with_oid(Oid(DATE_ARRAY_OID))
        }
    }

    impl Encode<'_, Postgres> for Date {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            let days = self
                .0
                .since((jiff::Unit::Day, postgres_epoch()))?
                .get_days();
            Encode::<Postgres>::encode(days, buf)
        }

        fn size_hint(&self) -> usize {
            mem::size_of::<i32>()
        }
    }

    impl<'r> Decode<'r, Postgres> for Date {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            match value.format() {
                PgValueFormat::Binary => {
                    let days: i32 = Decode::<Postgres>::decode(value)?;
                    let date = postgres_epoch().checked_add(jiff::Span::new().days(days))?;
                    Ok(Date(date))
                }
                PgValueFormat::Text => Ok(Date(value.as_str()?.parse()?)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i16, mo: i8, d: i8, h: i8, mi: i8, s: i8) -> Timestamp {
        Timestamp::utc(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn utc_builds_the_instant_and_rejects_impossible_dates() {
        assert_eq!(at(1970, 1, 1, 0, 0, 0), Timestamp::UNIX_EPOCH);
        assert_eq!(at(2026, 1, 1, 0, 0, 0).unix_secs(), 1_767_225_600);
        assert_eq!(Timestamp::utc(2026, 2, 30, 0, 0, 0), None);
    }

    #[test]
    fn unix_constructors_round_trip_and_reject_out_of_range() {
        let stamp = Timestamp::from_unix_secs(1_700_000_000).unwrap();
        assert_eq!(stamp.unix_secs(), 1_700_000_000);
        assert_eq!(stamp.unix_millis(), 1_700_000_000_000);
        assert_eq!(stamp.unix_micros(), 1_700_000_000_000_000);
        assert_eq!(Timestamp::from_unix_millis(1_700_000_000_000), Some(stamp));
        assert_eq!(
            Timestamp::from_unix_micros(1_700_000_000_000_000),
            Some(stamp)
        );
        assert_eq!(Timestamp::from_unix_secs(i64::MAX), None);
        assert_eq!(Timestamp::from_unix_millis(i64::MAX), None);
        assert_eq!(Timestamp::from_unix_micros(i64::MAX), None);
    }

    #[test]
    fn now_is_after_the_epoch_and_default_is_the_epoch() {
        assert!(Timestamp::now() > Timestamp::UNIX_EPOCH);
        assert_eq!(Timestamp::default(), Timestamp::UNIX_EPOCH);
    }

    #[test]
    fn arithmetic_moves_the_instant_and_measures_the_gap() {
        let start = at(2026, 6, 1, 12, 0, 0);
        assert_eq!(start + hours(2), at(2026, 6, 1, 14, 0, 0));
        assert_eq!(start - days(1), at(2026, 5, 31, 12, 0, 0));
        assert_eq!(at(2026, 6, 1, 12, 30, 0) - start, minutes(30));
        assert_eq!(start.since(at(2026, 6, 1, 13, 0, 0)), hours(-1));

        let mut moved = start;
        moved += seconds(90);
        assert_eq!(moved, at(2026, 6, 1, 12, 1, 30));
        moved -= millis(1_500);
        assert_eq!(moved.unix_secs(), at(2026, 6, 1, 12, 1, 28).unix_secs());
    }

    #[test]
    fn checked_arithmetic_is_none_out_of_range_and_saturation_clamps() {
        let start = at(2026, 6, 1, 0, 0, 0);
        assert_eq!(start.checked_add(hours(1)), Some(at(2026, 6, 1, 1, 0, 0)));
        assert_eq!(start.checked_sub(hours(1)), Some(at(2026, 5, 31, 23, 0, 0)));
        assert_eq!(Timestamp::MAX.checked_add(hours(1)), None);
        assert_eq!(Timestamp::MIN.checked_sub(hours(1)), None);
        assert_eq!(Timestamp::MAX + hours(1), Timestamp::MAX);
        assert_eq!(Timestamp::MIN - hours(1), Timestamp::MIN);
        assert_eq!(Timestamp::MAX.saturating_sub(hours(-1)), Timestamp::MAX);
        assert_eq!(Timestamp::MIN.saturating_add(hours(-1)), Timestamp::MIN);
    }

    #[test]
    fn subsecond_digits_are_truncated_not_rounded() {
        let stamp = Timestamp::from_unix_micros(1_700_000_000_123_456).unwrap();
        assert_eq!(stamp.trunc_subsecs(6), stamp);
        assert_eq!(stamp.trunc_subsecs(3).unix_micros(), 1_700_000_000_123_000);
        assert_eq!(stamp.trunc_subsecs(0).unix_micros(), 1_700_000_000_000_000);
        assert_eq!(stamp.trunc_subsecs(12), stamp);
    }

    #[test]
    fn formatting_and_parsing_use_rfc3339_in_utc() {
        let stamp = at(2026, 3, 4, 5, 6, 7);
        assert_eq!(stamp.to_rfc3339(), "2026-03-04T05:06:07Z");
        assert_eq!(stamp.to_string(), "2026-03-04T05:06:07Z");
        assert_eq!(format!("{stamp:?}"), "2026-03-04T05:06:07Z");
        assert_eq!(stamp.strftime("%Y%m%d-%H%M%S"), "20260304-050607");
        assert_eq!("2026-03-04T05:06:07Z".parse::<Timestamp>().unwrap(), stamp);
        assert_eq!(
            "2026-03-04T06:06:07+01:00".parse::<Timestamp>().unwrap(),
            stamp
        );
        assert!("yesterday".parse::<Timestamp>().is_err());
    }

    #[test]
    fn serde_is_a_plain_rfc3339_string() {
        let stamp = at(2026, 3, 4, 5, 6, 7);
        let json = serde_json::to_string(&stamp).unwrap();
        assert_eq!(json, "\"2026-03-04T05:06:07Z\"");
        assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), stamp);
    }

    #[test]
    fn conversions_with_jiff_and_system_time_agree() {
        let stamp = at(2026, 3, 4, 5, 6, 7);
        let inner: jiff::Timestamp = stamp.into();
        assert_eq!(Timestamp::from(inner), stamp);
        assert_eq!(stamp.into_inner(), inner);
        let system = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(
            Timestamp::try_from(system).unwrap().unix_secs(),
            1_700_000_000
        );
    }

    #[test]
    fn a_timestamp_has_a_utc_day_and_a_zoned_view() {
        let stamp = at(2026, 3, 4, 23, 30, 0);
        assert_eq!(stamp.utc_date(), Date::new(2026, 3, 4).unwrap());
        let berlin = stamp.to_zoned(jiff::tz::TimeZone::get("Europe/Berlin").unwrap());
        assert_eq!(berlin.day(), 5);
    }

    #[test]
    fn dates_parse_format_and_order() {
        let day = Date::new(2026, 3, 4).unwrap();
        assert_eq!(day.to_string(), "2026-03-04");
        assert_eq!(format!("{day:?}"), "2026-03-04");
        assert_eq!(day.strftime("%d.%m.%Y"), "04.03.2026");
        assert_eq!("2026-03-04".parse::<Date>().unwrap(), day);
        assert!("2026-13-01".parse::<Date>().is_err());
        assert_eq!(Date::new(2026, 2, 30), None);
        assert!(day < Date::new(2026, 3, 5).unwrap());
        assert_eq!(Date::from(day.into_inner()), day);
        assert_eq!(serde_json::to_string(&day).unwrap(), "\"2026-03-04\"");
        assert_eq!(serde_json::from_str::<Date>("\"2026-03-04\"").unwrap(), day);
    }

    #[test]
    fn month_bounds_span_the_month_and_reject_nonsense() {
        let (first, next) = Date::month_bounds(2026, 2).unwrap();
        assert_eq!(first, Date::new(2026, 2, 1).unwrap());
        assert_eq!(next, Date::new(2026, 3, 1).unwrap());
        let (first, next) = Date::month_bounds(2026, 12).unwrap();
        assert_eq!(first, Date::new(2026, 12, 1).unwrap());
        assert_eq!(next, Date::new(2027, 1, 1).unwrap());
        assert_eq!(Date::month_bounds(2026, 13), None);
        assert_eq!(Date::month_bounds(2026, 0), None);
        assert_eq!(Date::month_bounds(i32::MAX, 1), None);
        assert_eq!(Date::month_bounds(2026, u32::MAX), None);
    }

    #[test]
    fn duration_helpers_and_whole_unit_views_agree() {
        assert_eq!(days(2), hours(48));
        assert_eq!(hours(1), minutes(60));
        assert_eq!(minutes(1), seconds(60));
        assert_eq!(seconds(1), millis(1_000));
        let span = days(2) + hours(3) + minutes(4);
        assert_eq!(span.as_days(), 2);
        assert_eq!(span.as_hours(), 51);
        assert_eq!(span.as_mins(), 51 * 60 + 4);
        assert_eq!(days(i64::MAX), hours(i64::MAX));
    }

    #[test]
    fn typescript_sees_a_plain_string() {
        use ts_rs::TS as _;
        let cfg = ts_rs::Config::default();
        assert_eq!(Timestamp::name(&cfg), "string");
        assert_eq!(Timestamp::inline(&cfg), "string");
        assert_eq!(Date::name(&cfg), "string");
        assert_eq!(Date::inline(&cfg), "string");
    }
}
