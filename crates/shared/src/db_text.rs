// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Postgres support for domain types stored in `TEXT` columns.
//!
//! Each type below is read back from its stored `Display` form with
//! `FromStr`, so a stored value that names no variant fails the row read
//! instead of silently becoming some default.

use sqlx::{
    Decode, Postgres, Type,
    error::BoxDynError,
    postgres::{PgTypeInfo, PgValueRef},
};

use crate::{
    protocol::RepoOpKind,
    types::{
        BorgEncryption, Compression, ExecutionMode, OnFailure, QuotaAction, ReportStatus,
        RunEventTarget, RunEventType, ScheduleType, ScheduleWakeOverride, Visibility,
    },
    vm::{VmSelectionMode, VmSnapshotMode, VmState},
};

macro_rules! text_column {
    ($($ty:ty),+ $(,)?) => {$(
        impl Type<Postgres> for $ty {
            fn type_info() -> PgTypeInfo {
                <String as Type<Postgres>>::type_info()
            }

            fn compatible(ty: &PgTypeInfo) -> bool {
                <String as Type<Postgres>>::compatible(ty)
            }
        }

        impl<'r> Decode<'r, Postgres> for $ty {
            fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
                let text = <&str as Decode<Postgres>>::decode(value)?;
                Ok(text.parse::<Self>()?)
            }
        }
    )+};
}

text_column!(
    BorgEncryption,
    Compression,
    ExecutionMode,
    OnFailure,
    QuotaAction,
    RepoOpKind,
    ReportStatus,
    RunEventTarget,
    RunEventType,
    ScheduleType,
    ScheduleWakeOverride,
    Visibility,
    VmSelectionMode,
    VmSnapshotMode,
    VmState,
);
