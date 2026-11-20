// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Build script that embeds version metadata (git SHA and commit count)
//! into the `agent` binary at compile time. The logic is shared with the other
//! binary crate in `crates/shared/build_version.rs`.

include!("../shared/build_version.rs");
