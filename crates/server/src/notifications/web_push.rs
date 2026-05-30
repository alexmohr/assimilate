// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

use super::NotificationError;

pub async fn send(
    vapid_private_key: &str,
    endpoint: String,
    p256dh: String,
    auth: String,
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    let subscription = SubscriptionInfo::new(endpoint, p256dh, auth);

    let partial_builder = VapidSignatureBuilder::from_base64_no_sub(vapid_private_key)
        .map_err(|e| NotificationError::Config(format!("VAPID key parse error: {e}")))?;

    let sig_builder = partial_builder.add_sub_info(&subscription);

    let content = serde_json::to_vec(payload)?;

    let mut builder = WebPushMessageBuilder::new(&subscription);
    builder.set_payload(ContentEncoding::Aes128Gcm, &content);
    builder.set_vapid_signature(
        sig_builder
            .build()
            .map_err(|e| NotificationError::Config(format!("VAPID build error: {e}")))?,
    );

    let message = builder
        .build()
        .map_err(|e| NotificationError::Config(format!("web push message build error: {e}")))?;

    let client = IsahcWebPushClient::new()
        .map_err(|e| NotificationError::Config(format!("web push client error: {e}")))?;

    client.send(message).await?;

    Ok(())
}
