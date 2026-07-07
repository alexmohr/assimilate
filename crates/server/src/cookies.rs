// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::http::{HeaderMap, HeaderValue, header};

/// Whether the session cookie should carry the `Secure` attribute.
///
/// Defaults to `Secure` fail-safe: only an explicit `ASSIMILATE_SECURE_COOKIES=false`
/// disables it (e.g. for local HTTP development).
pub(crate) enum CookieSecurity {
    Secure,
    Insecure,
}

impl From<Option<String>> for CookieSecurity {
    fn from(env_value: Option<String>) -> Self {
        match env_value.as_deref() {
            Some("false") => Self::Insecure,
            _ => Self::Secure,
        }
    }
}

impl CookieSecurity {
    fn cookie_flag(self) -> &'static str {
        match self {
            Self::Secure => "; Secure",
            Self::Insecure => "",
        }
    }
}

/// Extract the session ID from the `Cookie` header.
///
/// Splits the `Cookie` header on `;`, trims, looks for `session=` prefix,
/// and returns the value if non-empty. Returns `None` when no session cookie
/// is present or the value is empty.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix("session=")
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

/// Build a `Set-Cookie` header value for the session cookie.
///
/// * `session_id` — `Some(id)` sets the cookie value to the session ID; `None` clears it.
/// * `max_age` — cookie's `Max-Age` in seconds.
///
/// Always includes `HttpOnly; SameSite=Lax; Path=/`. The `Secure` flag is
/// controlled by the `ASSIMILATE_SECURE_COOKIES` environment variable.
pub fn session_set_cookie(
    session_id: Option<&str>,
    max_age: i64,
) -> Result<HeaderValue, header::InvalidHeaderValue> {
    let value = session_id.unwrap_or("");
    let flag = CookieSecurity::from(std::env::var("ASSIMILATE_SECURE_COOKIES").ok()).cookie_flag();
    let cookie =
        format!("session={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{flag}");
    cookie.parse()
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderMap;

    use super::*;

    #[test]
    fn session_cookie_with_valid_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "session=abc123; other=val".parse().unwrap());
        assert_eq!(session_cookie(&headers), Some("abc123".to_string()));
    }

    #[test]
    fn session_cookie_no_session() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "other=val".parse().unwrap());
        assert_eq!(session_cookie(&headers), None);
    }

    #[test]
    fn session_cookie_empty_value() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "session=".parse().unwrap());
        assert_eq!(session_cookie(&headers), None);
    }

    #[test]
    fn session_cookie_no_cookie_header() {
        let headers = HeaderMap::new();
        assert_eq!(session_cookie(&headers), None);
    }

    #[test]
    fn session_set_cookie_creates_valid_header() {
        let result = session_set_cookie(Some("test-id"), 86400);
        let header = result.unwrap();
        let value = header.to_str().unwrap();
        assert!(value.contains("session=test-id"));
        assert!(value.contains("HttpOnly"));
        assert!(value.contains("SameSite=Lax"));
        assert!(value.contains("Path=/"));
        assert!(value.contains("Max-Age=86400"));
    }

    #[test]
    fn session_set_cookie_clear_cookie() {
        let result = session_set_cookie(None, 0);
        let header = result.unwrap();
        let value = header.to_str().unwrap();
        assert!(value.starts_with("session=;"));
        assert!(value.contains("Max-Age=0"));
    }

    #[test]
    fn session_set_cookie_max_age_is_reflected() {
        let result = session_set_cookie(Some("id"), 7776000);
        let header = result.unwrap();
        let value = header.to_str().unwrap();
        assert!(value.contains("Max-Age=7776000"));
    }

    #[test]
    fn cookie_security_defaults_to_secure_when_unset() {
        assert_eq!(CookieSecurity::from(None).cookie_flag(), "; Secure");
    }

    #[test]
    fn cookie_security_is_insecure_when_explicitly_false() {
        assert_eq!(
            CookieSecurity::from(Some("false".to_string())).cookie_flag(),
            ""
        );
    }

    #[test]
    fn cookie_security_is_secure_for_any_other_value() {
        assert_eq!(
            CookieSecurity::from(Some("0".to_string())).cookie_flag(),
            "; Secure"
        );
    }
}
