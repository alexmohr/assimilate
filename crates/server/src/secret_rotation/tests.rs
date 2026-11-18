// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use sqlx::PgPool;

use super::*;

fn key(material: &[u8]) -> [u8; 32] {
    shared::crypto::derive_key(material).unwrap()
}

fn current() -> [u8; 32] {
    key(b"rotation-test-current-secret")
}

fn previous() -> [u8; 32] {
    key(b"rotation-test-previous-secret")
}

fn keys() -> RotationKeys {
    RotationKeys::new(current(), previous())
}

fn encrypt(plaintext: &str, key: &[u8; 32]) -> Vec<u8> {
    encrypt_passphrase(plaintext, key).unwrap()
}

fn decrypts_to(stored: &[u8], key: &[u8; 32]) -> Option<String> {
    decrypt_passphrase(stored, key).ok()
}

async fn insert_repo(pool: &PgPool, name: &str, passphrase: &[u8]) -> i64 {
    crate::db::insert_repo(
        pool,
        &crate::db::InsertRepoParams {
            name,
            repo_path: &format!("/srv/borg/{name}"),
            ssh_user: "borg",
            ssh_host: "backup.example.com",
            ssh_port: 22,
            passphrase_encrypted: passphrase,
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap()
    .id
}

async fn repo_passphrase(pool: &PgPool, id: i64) -> Vec<u8> {
    sqlx::query_scalar!("SELECT passphrase_encrypted FROM repos WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_totp_user(pool: &PgPool, username: &str, secret: Option<&[u8]>) -> i64 {
    let user = crate::test_support::insert_auth_user(pool, username).await;
    sqlx::query!(
        "UPDATE users SET totp_secret_encrypted = $2 WHERE id = $1",
        user.user_id,
        secret,
    )
    .execute(pool)
    .await
    .unwrap();
    user.user_id
}

async fn totp_secret(pool: &PgPool, id: i64) -> Option<Vec<u8>> {
    sqlx::query_scalar!("SELECT totp_secret_encrypted FROM users WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_email_channel(pool: &PgPool, name: &str, password: Option<&[u8]>) -> i64 {
    sqlx::query_scalar!(
        "INSERT INTO notification_channels (name, channel_type, config, smtp_password_encrypted) \
         VALUES ($1, 'email', '{}'::jsonb, $2) RETURNING id",
        name,
        password,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn smtp_password(pool: &PgPool, id: i64) -> Option<Vec<u8>> {
    sqlx::query_scalar!(
        "SELECT smtp_password_encrypted FROM notification_channels WHERE id = $1",
        id
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_header(pool: &PgPool, channel_id: i64, name: &str, value: Option<&[u8]>) {
    sqlx::query!(
        "INSERT INTO notification_channel_headers (channel_id, name, value_encrypted) VALUES ($1, \
         $2, $3)",
        channel_id,
        name,
        value,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn header_value(pool: &PgPool, channel_id: i64, name: &str) -> Option<Vec<u8>> {
    sqlx::query_scalar!(
        "SELECT value_encrypted FROM notification_channel_headers WHERE channel_id = $1 AND name \
         = $2",
        channel_id,
        name,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn every_kind_of_secret_moves_to_the_current_key(pool: PgPool) {
    let repo = insert_repo(&pool, "rotate-repo", &encrypt("repo-pass", &previous())).await;
    let user = insert_totp_user(
        &pool,
        "rotate-totp-user",
        Some(&encrypt("totp-seed", &previous())),
    )
    .await;
    let channel = insert_email_channel(
        &pool,
        "rotate-email",
        Some(&encrypt("smtp-pass", &previous())),
    )
    .await;
    insert_header(
        &pool,
        channel,
        "Authorization",
        Some(&encrypt("Bearer abc", &previous())),
    )
    .await;

    let rotated = rotate_stored_secrets(&pool, &keys()).await.unwrap();

    assert_eq!(
        rotated,
        RotatedSecrets {
            repository_passphrases: 1,
            totp_secrets: 1,
            smtp_passwords: 1,
            webhook_header_values: 1,
        }
    );
    assert_eq!(rotated.total(), 4);
    let repo = repo_passphrase(&pool, repo).await;
    assert_eq!(decrypts_to(&repo, &current()).as_deref(), Some("repo-pass"));
    assert_eq!(decrypts_to(&repo, &previous()), None);
    let totp = totp_secret(&pool, user).await.unwrap();
    assert_eq!(decrypts_to(&totp, &current()).as_deref(), Some("totp-seed"));
    let smtp = smtp_password(&pool, channel).await.unwrap();
    assert_eq!(decrypts_to(&smtp, &current()).as_deref(), Some("smtp-pass"));
    let header = header_value(&pool, channel, "Authorization").await.unwrap();
    assert_eq!(
        decrypts_to(&header, &current()).as_deref(),
        Some("Bearer abc")
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_second_run_finds_nothing_left_to_do(pool: PgPool) {
    insert_repo(&pool, "rotate-twice", &encrypt("repo-pass", &previous())).await;

    let first = rotate_stored_secrets(&pool, &keys()).await.unwrap();
    let second = rotate_stored_secrets(&pool, &keys()).await.unwrap();

    assert_eq!(first.repository_passphrases, 1);
    assert_eq!(second, RotatedSecrets::default());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn values_already_under_the_current_key_are_left_untouched(pool: PgPool) {
    let repo_secret = encrypt("saved-after-rotation", &current());
    let totp_secret_value = encrypt("totp-after-rotation", &current());
    let smtp_secret = encrypt("smtp-after-rotation", &current());
    let header_secret = encrypt("header-after-rotation", &current());
    let repo = insert_repo(&pool, "rotate-current", &repo_secret).await;
    let user = insert_totp_user(&pool, "rotate-current-totp", Some(&totp_secret_value)).await;
    let channel = insert_email_channel(&pool, "rotate-current-smtp", Some(&smtp_secret)).await;
    insert_header(&pool, channel, "X-Current", Some(&header_secret)).await;

    let rotated = rotate_stored_secrets(&pool, &keys()).await.unwrap();

    assert_eq!(rotated, RotatedSecrets::default());
    assert_eq!(repo_passphrase(&pool, repo).await, repo_secret);
    assert_eq!(totp_secret(&pool, user).await, Some(totp_secret_value));
    assert_eq!(smtp_password(&pool, channel).await, Some(smtp_secret));
    assert_eq!(
        header_value(&pool, channel, "X-Current").await,
        Some(header_secret)
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_undecryptable_two_factor_secret_names_its_user(pool: PgPool) {
    insert_totp_user(
        &pool,
        "rotate-foreign-totp",
        Some(&encrypt("seed", &key(b"some other server"))),
    )
    .await;

    let error = rotate_stored_secrets(&pool, &keys()).await.unwrap_err();

    assert!(
        matches!(
            &error,
            SecretRotationError::Undecryptable {
                kind: StoredSecretKind::TotpSecret,
                owner,
            } if owner.contains("rotate-foreign-totp")
        ),
        "{error}"
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_undecryptable_header_value_names_its_header_and_channel(pool: PgPool) {
    let channel = insert_email_channel(&pool, "rotate-foreign-hook", None).await;
    insert_header(
        &pool,
        channel,
        "Authorization",
        Some(&encrypt("Bearer x", &key(b"some other server"))),
    )
    .await;

    let error = rotate_stored_secrets(&pool, &keys()).await.unwrap_err();

    assert!(
        matches!(
            &error,
            SecretRotationError::Undecryptable {
                kind: StoredSecretKind::WebhookHeaderValue,
                owner,
            } if owner.contains("Authorization") && owner.contains("rotate-foreign-hook")
        ),
        "{error}"
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn absent_secrets_are_skipped(pool: PgPool) {
    let user = insert_totp_user(&pool, "rotate-no-totp", None).await;
    let channel = insert_email_channel(&pool, "rotate-no-smtp", None).await;
    insert_header(&pool, channel, "X-Empty", None).await;

    let rotated = rotate_stored_secrets(&pool, &keys()).await.unwrap();

    assert_eq!(rotated, RotatedSecrets::default());
    assert_eq!(totp_secret(&pool, user).await, None);
    assert_eq!(smtp_password(&pool, channel).await, None);
    assert_eq!(header_value(&pool, channel, "X-Empty").await, None);
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_value_under_neither_key_aborts_without_changing_anything(pool: PgPool) {
    let pending = encrypt("repo-pass", &previous());
    let repo = insert_repo(&pool, "rotate-pending", &pending).await;
    let channel = insert_email_channel(
        &pool,
        "rotate-foreign",
        Some(&encrypt("smtp-pass", &key(b"some other server"))),
    )
    .await;

    let error = rotate_stored_secrets(&pool, &keys()).await.unwrap_err();

    assert!(
        matches!(
            &error,
            SecretRotationError::Undecryptable {
                kind: StoredSecretKind::SmtpPassword,
                owner,
            } if owner.contains("rotate-foreign")
        ),
        "{error}"
    );
    assert!(error.to_string().contains("ASSIMILATE_PREVIOUS_SECRET_KEY"));
    assert_eq!(repo_passphrase(&pool, repo).await, pending);
    assert!(smtp_password(&pool, channel).await.is_some());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn the_wrong_previous_secret_is_reported_and_changes_nothing(pool: PgPool) {
    let stored = encrypt("repo-pass", &previous());
    let repo = insert_repo(&pool, "rotate-wrong-previous", &stored).await;

    let wrong = RotationKeys::new(current(), key(b"not the previous secret"));
    let error = rotate_stored_secrets(&pool, &wrong).await.unwrap_err();

    assert!(
        matches!(
            error,
            SecretRotationError::Undecryptable {
                kind: StoredSecretKind::RepositoryPassphrase,
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(repo_passphrase(&pool, repo).await, stored);
}

#[test]
fn rotation_keys_are_redacted_in_debug_output() {
    assert_eq!(format!("{:?}", keys()), "RotationKeys([REDACTED])");
}

#[test]
fn the_error_names_the_secret_without_its_value() {
    let error = SecretRotationError::Undecryptable {
        kind: StoredSecretKind::WebhookHeaderValue,
        owner: "header \"Authorization\" of notification channel \"ops\"".to_owned(),
    };
    let shown = error.to_string();
    assert!(shown.starts_with("the webhook header value of header \"Authorization\""));
}
