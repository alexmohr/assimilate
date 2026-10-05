// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Length caps on the strings that name and locate a repository.

use super::{CreateRepoRequest, InitRepoRequest, UpdateRepoRequest};
use crate::{
    api::helpers::{self, MaxLen},
    error::ApiError,
};

/// The strings shared by [`CreateRepoRequest`], [`InitRepoRequest`] and
/// [`UpdateRepoRequest`], borrowed so all three are checked by the same rules
/// before the repository is probed over SSH.
pub(super) struct RepoTextFields<'a> {
    name: Option<&'a str>,
    repo_path: &'a str,
    ssh_user: &'a str,
    ssh_host: &'a str,
    sync_schedule: Option<&'a str>,
}

impl<'a> From<&'a CreateRepoRequest> for RepoTextFields<'a> {
    fn from(req: &'a CreateRepoRequest) -> Self {
        Self {
            name: Some(&req.name),
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            sync_schedule: None,
        }
    }
}

impl<'a> From<&'a InitRepoRequest> for RepoTextFields<'a> {
    fn from(req: &'a InitRepoRequest) -> Self {
        Self {
            name: Some(&req.name),
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            sync_schedule: None,
        }
    }
}

impl<'a> From<&'a UpdateRepoRequest> for RepoTextFields<'a> {
    fn from(req: &'a UpdateRepoRequest) -> Self {
        Self {
            name: req.name.as_deref(),
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            sync_schedule: req.sync_schedule.as_ref().and_then(Option::as_deref),
        }
    }
}

impl RepoTextFields<'_> {
    /// Rejects the first string that exceeds its [`MaxLen`] cap.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::BadRequest`] naming the offending field.
    pub(super) fn validate(&self) -> Result<(), ApiError> {
        helpers::validate_opt_max_len(self.name, "name", MaxLen::Name)?;
        helpers::validate_max_len(self.repo_path, "repo_path", MaxLen::Path)?;
        helpers::validate_max_len(self.ssh_user, "ssh_user", MaxLen::Name)?;
        helpers::validate_max_len(self.ssh_host, "ssh_host", MaxLen::Hostname)?;
        helpers::validate_opt_max_len(self.sync_schedule, "sync_schedule", MaxLen::Name)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::RepoTextFields;
    use crate::{
        api::{
            helpers::{MaxLen, rejection_message},
            repos::{CreateRepoRequest, InitRepoRequest, UpdateRepoRequest},
        },
        error::ApiError,
    };

    fn body(overrides: &Value) -> Value {
        let mut body = json!({
            "name": "offsite",
            "repo_path": "/srv/borg/offsite",
            "ssh_user": "borg",
            "ssh_host": "backup.example.com",
            "passphrase": "hunter2",
            "encryption": "repokey-blake2",
        });
        body.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        body
    }

    /// Every request shape's validation result for the same body.
    fn validate_all(overrides: &Value) -> [Result<(), ApiError>; 3] {
        let body = body(overrides);
        let create: CreateRepoRequest = serde_json::from_value(body.clone()).unwrap();
        let init: InitRepoRequest = serde_json::from_value(body.clone()).unwrap();
        let update: UpdateRepoRequest = serde_json::from_value(body).unwrap();
        [
            RepoTextFields::from(&create).validate(),
            RepoTextFields::from(&init).validate(),
            RepoTextFields::from(&update).validate(),
        ]
    }

    #[test]
    fn strings_at_their_limits_are_accepted() {
        let at = |max: MaxLen| "a".repeat(max.chars());
        let overrides = json!({
            "name": at(MaxLen::Name),
            "repo_path": at(MaxLen::Path),
            "ssh_user": at(MaxLen::Name),
            "ssh_host": at(MaxLen::Hostname),
        });
        for result in validate_all(&overrides) {
            assert!(result.is_ok(), "{result:?}");
        }
    }

    #[test]
    fn each_over_limit_field_is_rejected_by_name() {
        let over = |max: MaxLen| "a".repeat(max.chars().saturating_add(1));
        let cases = [
            ("name", over(MaxLen::Name)),
            ("repo_path", over(MaxLen::Path)),
            ("ssh_user", over(MaxLen::Name)),
            ("ssh_host", over(MaxLen::Hostname)),
        ];
        for (field, value) in cases {
            for result in validate_all(&json!({ field: value })) {
                let message = rejection_message(result);
                assert!(message.starts_with(&format!("{field} ")), "{message}");
            }
        }
    }

    #[test]
    fn update_caps_the_sync_schedule_only_when_one_is_set() {
        let update = |sync_schedule: Value| -> UpdateRepoRequest {
            serde_json::from_value(body(&json!({ "sync_schedule": sync_schedule }))).unwrap()
        };
        assert!(
            RepoTextFields::from(&update(Value::Null))
                .validate()
                .is_ok()
        );
        assert!(
            RepoTextFields::from(&update(json!("0 * * * *")))
                .validate()
                .is_ok()
        );
        assert!(matches!(
            RepoTextFields::from(&update(json!("a".repeat(256)))).validate(),
            Err(ApiError::BadRequest(message)) if message.starts_with("sync_schedule ")
        ));
    }
}
