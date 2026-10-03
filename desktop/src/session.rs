// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use tauri::webview::Cookie;

/// The cookie that signs the webview in as the local admin: the same
/// `session` cookie the server sets on login, scoped to the loopback host.
pub fn session_cookie(value: &str) -> Cookie<'static> {
    Cookie::build(("session", value.to_owned()))
        .domain("127.0.0.1")
        .path("/")
        .http_only(true)
        .same_site(tauri::webview::cookie::SameSite::Lax)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_cookie_the_server_sets() {
        let cookie = session_cookie("abc");

        assert_eq!(cookie.name(), "session");
        assert_eq!(cookie.value(), "abc");
        assert_eq!(cookie.domain(), Some("127.0.0.1"));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(
            cookie.same_site(),
            Some(tauri::webview::cookie::SameSite::Lax)
        );
        assert_eq!(cookie.secure(), None);
    }
}
