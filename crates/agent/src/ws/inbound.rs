// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Decoding of server messages, tolerant of message types this agent does not
//! know so that a newer server cannot knock an older agent offline.

use serde::Deserialize;
use shared::protocol::ServerToAgent;

use super::WsError;

/// A server message, decoded as far as this agent understands it.
#[derive(Debug)]
pub(super) enum Inbound {
    Known(ServerToAgent),
    Unrecognised(UnrecognisedMessage),
}

/// A message whose `type` this agent does not know, or whose payload does
/// not match what this agent expects for that `type`. Only the tag and the
/// request id are kept; the payload may carry passphrases and is dropped.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UnrecognisedMessage {
    pub(super) message_type: String,
    pub(super) request_id: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Decodes `text` as a [`ServerToAgent`], falling back to just its envelope
/// when the full message does not parse. Fails only when `text` is not a
/// tagged envelope at all.
pub(super) fn decode(text: &str) -> Result<Inbound, WsError> {
    if let Ok(msg) = serde_json::from_str::<ServerToAgent>(text) {
        return Ok(Inbound::Known(msg));
    }
    let envelope: Envelope = serde_json::from_str(text).map_err(WsError::Deserialize)?;
    let request_id = envelope
        .payload
        .as_ref()
        .and_then(|payload| payload.get("request_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(Inbound::Unrecognised(UnrecognisedMessage {
        message_type: envelope.message_type,
        request_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unrecognised(text: &str) -> UnrecognisedMessage {
        match decode(text).unwrap() {
            Inbound::Unrecognised(msg) => msg,
            Inbound::Known(msg) => panic!("expected an unrecognised message, got {msg:?}"),
        }
    }

    #[test]
    fn known_message_decodes_fully() {
        let decoded = decode(r#"{"type":"Ping"}"#).unwrap();
        assert!(matches!(decoded, Inbound::Known(ServerToAgent::Ping)));
    }

    #[test]
    fn unknown_type_keeps_its_tag_and_request_id() {
        let msg = unrecognised(
            r#"{"type":"SomeFutureRequest","payload":{"request_id":"req-1","passphrase":"x"}}"#,
        );
        assert_eq!(
            msg,
            UnrecognisedMessage {
                message_type: "SomeFutureRequest".into(),
                request_id: Some("req-1".into()),
            }
        );
    }

    #[test]
    fn unknown_type_without_payload_has_no_request_id() {
        let msg = unrecognised(r#"{"type":"SomeFutureNotice"}"#);
        assert_eq!(msg.message_type, "SomeFutureNotice");
        assert_eq!(msg.request_id, None);
    }

    #[test]
    fn non_object_payload_has_no_request_id() {
        let msg = unrecognised(r#"{"type":"SomeFutureNotice","payload":[1,2,3]}"#);
        assert_eq!(msg.request_id, None);
    }

    #[test]
    fn non_string_request_id_is_ignored() {
        let msg = unrecognised(r#"{"type":"SomeFutureRequest","payload":{"request_id":7}}"#);
        assert_eq!(msg.request_id, None);
    }

    #[test]
    fn known_type_with_a_payload_this_agent_cannot_read_is_unrecognised() {
        let msg =
            unrecognised(r#"{"type":"DryRun","payload":{"request_id":"req-2","repo_id":"x"}}"#);
        assert_eq!(
            msg,
            UnrecognisedMessage {
                message_type: "DryRun".into(),
                request_id: Some("req-2".into()),
            }
        );
    }

    #[test]
    fn text_without_an_envelope_is_a_decode_error() {
        assert!(matches!(decode("not json"), Err(WsError::Deserialize(_))));
        assert!(matches!(
            decode(r#"{"payload":{}}"#),
            Err(WsError::Deserialize(_))
        ));
    }
}
