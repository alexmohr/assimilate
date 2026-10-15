// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// The name of the cookie carrying a logged-in user's session ID.
pub const SESSION_COOKIE: &str = "session";

/// Returns the value of the cookie called `name` from a `;`-separated header.
///
/// Works on both a `Cookie` request header (`a=1; session=abc`) and a
/// `Set-Cookie` response header (`session=abc; HttpOnly; Path=/`). The name
/// must match exactly, so `sessionx=` is not the `session` cookie. Returns
/// `None` when the cookie is missing or its value is empty.
#[must_use]
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header
        .split(';')
        .filter_map(|pair| pair.trim().strip_prefix(name)?.strip_prefix('='))
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_named_cookie_among_several_pairs() {
        assert_eq!(
            cookie_value("theme=dark; session=abc123; other=val", SESSION_COOKIE),
            Some("abc123")
        );
    }

    #[test]
    fn reads_a_set_cookie_header_with_attributes() {
        assert_eq!(
            cookie_value(
                "session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=60",
                SESSION_COOKIE
            ),
            Some("abc")
        );
    }

    #[test]
    fn missing_cookie_is_none() {
        assert_eq!(cookie_value("other=val; Path=/", SESSION_COOKIE), None);
        assert_eq!(cookie_value("", SESSION_COOKIE), None);
        assert_eq!(cookie_value("garbage", SESSION_COOKIE), None);
    }

    #[test]
    fn empty_value_is_none() {
        assert_eq!(cookie_value("session=", SESSION_COOKIE), None);
        assert_eq!(cookie_value("session=; Max-Age=0", SESSION_COOKIE), None);
    }

    #[test]
    fn name_must_match_exactly() {
        assert_eq!(cookie_value("sessionx=abc", SESSION_COOKIE), None);
        assert_eq!(cookie_value("xsession=abc", SESSION_COOKIE), None);
        assert_eq!(
            cookie_value("sessionx=abc; session=def", SESSION_COOKIE),
            Some("def")
        );
    }
}
