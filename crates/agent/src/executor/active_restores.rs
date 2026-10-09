// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The restores this agent is working on, by request id.
//!
//! The server sends every unfinished restore again whenever the agent
//! connects, because an agent that restarted has lost the ones it had. One
//! that only lost its connection still has them, queued or extracting, and
//! must not extract the same files a second time.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Debug, Clone, Default)]
pub(super) struct ActiveRestores {
    request_ids: Arc<Mutex<HashSet<String>>>,
}

/// Holds a restore's place until it finishes, however it finishes.
#[derive(Debug)]
pub(super) struct ActiveRestore {
    request_ids: Arc<Mutex<HashSet<String>>>,
    request_id: String,
}

impl ActiveRestores {
    /// Claims `request_id`, or returns `None` when the agent is already
    /// working on it.
    pub(super) fn claim(&self, request_id: &str) -> Option<ActiveRestore> {
        let newly_claimed = self
            .request_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(request_id.to_owned());
        newly_claimed.then(|| ActiveRestore {
            request_ids: Arc::clone(&self.request_ids),
            request_id: request_id.to_owned(),
        })
    }
}

impl Drop for ActiveRestore {
    fn drop(&mut self) {
        self.request_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_is_claimed_once_until_it_finishes() {
        let active = ActiveRestores::default();

        let first = active.claim("req-1");
        assert!(first.is_some());
        assert!(
            active.claim("req-1").is_none(),
            "a resent restore is ignored"
        );
        assert!(
            active.claim("req-2").is_some(),
            "other restores are unaffected"
        );

        drop(first);
        assert!(
            active.claim("req-1").is_some(),
            "once finished, the same request id can run again"
        );
    }
}
