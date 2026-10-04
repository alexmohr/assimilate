// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Changes to where notifications go are audited, without any channel secret.

use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::test_support::{audit_entries, audit_events, build_test_state, insert_auth_user};

const KEY: &[u8] = b"notifications-audit-test-key";

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_channel_lifecycle_is_audited_without_its_secrets(pool: PgPool) {
    let state = build_test_state(pool.clone(), KEY);
    let admin = insert_auth_user(&pool, "notify-admin").await;

    let (_, Json(channel)) = create_channel(
        State(state.clone()),
        RequireAdmin(admin.clone()),
        ClientIp::default(),
        ApiJson(
            serde_json::from_value(json!({
                "name": "Ops Webhook",
                "channel_type": "webhook",
                "config": {
                    "url": "https://hooks.example.com/assimilate",
                    "headers": { "Authorization": "Bearer audit-secret-value" }
                }
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let Json(_) = update_channel(
        State(state.clone()),
        RequireAdmin(admin.clone()),
        ClientIp::default(),
        Path(channel.id),
        ApiJson(serde_json::from_value(json!({ "name": "Ops Pager" })).unwrap()),
    )
    .await
    .unwrap();
    delete_channel(
        State(state),
        RequireAdmin(admin),
        ClientIp::default(),
        Path(channel.id),
    )
    .await
    .unwrap();

    let entries = audit_entries(&pool).await;
    assert!(entries.iter().all(|entry| {
        entry.target_type.as_deref() == Some("notification_channel")
            && entry.target_id == Some(channel.id)
    }));
    let stored = serde_json::to_string(&entries).unwrap();
    assert!(
        !stored.contains("audit-secret-value") && !stored.contains("hooks.example.com"),
        "a channel's configuration must never reach the audit log"
    );
    let events: Vec<_> = entries.into_iter().map(|entry| entry.event).collect();
    assert_eq!(
        events,
        [
            AuditEvent::DeleteNotificationChannel {
                name: "Ops Pager".to_owned(),
                channel_type: ChannelType::Webhook,
            },
            AuditEvent::UpdateNotificationChannel {
                name: "Ops Pager".to_owned(),
                channel_type: ChannelType::Webhook,
            },
            AuditEvent::CreateNotificationChannel {
                name: "Ops Webhook".to_owned(),
                channel_type: ChannelType::Webhook,
            },
        ]
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_rule_lifecycle_is_audited(pool: PgPool) {
    let state = build_test_state(pool.clone(), KEY);
    let admin = insert_auth_user(&pool, "notify-admin").await;
    let (_, Json(channel)) = create_channel(
        State(state.clone()),
        RequireAdmin(admin.clone()),
        ClientIp::default(),
        ApiJson(
            serde_json::from_value(json!({
                "name": "Hook",
                "channel_type": "webhook",
                "config": { "url": "https://hooks.example.com/rules" }
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();

    let (_, Json(rule)) = create_rule(
        State(state.clone()),
        RequireAdmin(admin.clone()),
        ClientIp::default(),
        ApiJson(CreateRuleRequest {
            channel_id: channel.id,
            event_type: EventType::BackupFailed,
            repo_id: None,
            agent_id: None,
            enabled: None,
        }),
    )
    .await
    .unwrap();
    delete_rule(
        State(state),
        RequireAdmin(admin),
        ClientIp::default(),
        Path(rule.id),
    )
    .await
    .unwrap();

    assert_eq!(
        audit_events(&pool).await,
        [
            AuditEvent::DeleteNotificationRule {
                channel_id: channel.id,
                event_type: EventType::BackupFailed,
                repo_id: None,
                agent_id: None,
            },
            AuditEvent::CreateNotificationRule {
                channel_id: channel.id,
                event_type: EventType::BackupFailed,
                repo_id: None,
                agent_id: None,
            },
            AuditEvent::CreateNotificationChannel {
                name: "Hook".to_owned(),
                channel_type: ChannelType::Webhook,
            },
        ]
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn deleting_a_missing_rule_or_channel_is_not_audited(pool: PgPool) {
    let state = build_test_state(pool.clone(), KEY);
    let admin = insert_auth_user(&pool, "notify-admin").await;

    let rule = delete_rule(
        State(state.clone()),
        RequireAdmin(admin.clone()),
        ClientIp::default(),
        Path(987_654),
    )
    .await;
    let channel = delete_channel(
        State(state),
        RequireAdmin(admin),
        ClientIp::default(),
        Path(987_654),
    )
    .await;

    assert!(matches!(rule, Err(ApiError::NotFound(_))));
    assert!(matches!(channel, Err(ApiError::NotFound(_))));
    assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn replacing_the_vapid_keys_is_audited_without_them(pool: PgPool) {
    let state = build_test_state(pool.clone(), KEY);
    let admin = insert_auth_user(&pool, "notify-admin").await;

    set_vapid_keys(
        State(state),
        RequireAdmin(admin),
        ClientIp::default(),
        ApiJson(SetVapidKeysRequest {
            public_key: "vapid-public".to_owned(),
            private_key: "vapid-private-secret".to_owned(),
        }),
    )
    .await
    .unwrap();

    let entries = audit_entries(&pool).await;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.event.clone())
            .collect::<Vec<_>>(),
        [AuditEvent::SetVapidKeys {}]
    );
    assert!(
        !serde_json::to_string(&entries)
            .unwrap()
            .contains("vapid-private-secret")
    );
}
