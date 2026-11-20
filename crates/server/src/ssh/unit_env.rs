// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The assignments on a systemd unit's `Environment=` lines, read and written
//! the way systemd itself parses them.

/// Splits the value of an `Environment=` line into its words the way systemd
/// does: on whitespace outside single or double quotes, with a backslash
/// escaping the character after it. Each word keeps its quotes and escapes,
/// so a line rebuilt from them means what it meant before.
pub(super) fn split_words(value: &str) -> Vec<&str> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        Single,
        Double,
    }
    let quote_of = |c: char| match c {
        '\'' => Some(Quote::Single),
        '"' => Some(Quote::Double),
        _ => None,
    };

    let mut words = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;
    for (index, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            start.get_or_insert(index);
            continue;
        }
        match (quote, start) {
            (Some(open), _) => {
                if quote_of(c) == Some(open) {
                    quote = None;
                }
            }
            (None, Some(word_start)) => {
                if c.is_whitespace() {
                    words.push(&value[word_start..index]);
                    start = None;
                } else {
                    quote = quote_of(c);
                }
            }
            (None, None) => {
                if !c.is_whitespace() {
                    start = Some(index);
                    quote = quote_of(c);
                }
            }
        }
    }
    if let Some(word_start) = start {
        words.push(&value[word_start..]);
    }
    words
}

/// Whether `word` is an assignment systemd accepts, `NAME=value` with a
/// shell-style variable name. systemd ignores any other word on an
/// `Environment=` line, with a warning.
pub(super) fn is_assignment(word: &str) -> bool {
    let word = word.trim_start_matches(['"', '\'']);
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    name.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The `key=value` word for an `Environment=` line. A value made only of
/// characters systemd reads literally is written as it is; any other is
/// double-quoted with C escapes, so whitespace, quotes or a line break in it
/// can neither split the word nor start a new directive. A `%` is doubled
/// either way, as systemd would read it as the start of a specifier.
pub(super) fn assignment(key: &str, value: &str) -> String {
    let value = value.replace('%', "%%");
    if value.chars().all(is_literal) {
        return format!("{key}={value}");
    }
    let escaped = value
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".to_owned(),
            '"' => "\\\"".to_owned(),
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            c if c.is_control() => format!("\\x{:02x}", u32::from(c)),
            c => c.to_string(),
        })
        .collect::<String>();
    format!("\"{key}={escaped}\"")
}

/// Whether systemd reads `c` in an unquoted word as itself.
fn is_literal(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '+' | '=' | '@' | '%')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_split_on_whitespace() {
        assert_eq!(split_words("  A=1   B=2\tC=3 "), ["A=1", "B=2", "C=3"]);
        assert!(split_words("   ").is_empty());
    }

    #[test]
    fn whitespace_inside_quotes_does_not_split_a_word() {
        assert_eq!(
            split_words(r#""A=a b" 'B=c d' C="e f"g H=1"#),
            [r#""A=a b""#, "'B=c d'", r#"C="e f"g"#, "H=1"]
        );
    }

    #[test]
    fn a_quote_of_the_other_kind_does_not_end_a_quoted_word() {
        assert_eq!(
            split_words(r#""A=it's here" B=2"#),
            [r#""A=it's here""#, "B=2"]
        );
    }

    #[test]
    fn a_backslash_escapes_the_next_character() {
        assert_eq!(
            split_words(r#"A=a\ b "B=\"c d\"" \C=1"#),
            [r"A=a\ b", r#""B=\"c d\"""#, r"\C=1"]
        );
    }

    #[test]
    fn an_unterminated_quote_runs_to_the_end() {
        assert_eq!(split_words(r#"A=1 "B=2 3"#), ["A=1", r#""B=2 3"#]);
    }

    #[test]
    fn assignments_need_a_shell_variable_name() {
        for word in ["A=1", "_A=", "RUST_LOG=debug", "\"B=x y\"", "'C=z'"] {
            assert!(is_assignment(word), "{word}");
        }
        for word in ["be", "automatically>", "=1", "1A=2", "A-B=1", "A B=1", ""] {
            assert!(!is_assignment(word), "{word}");
        }
    }

    #[test]
    fn a_plain_value_is_written_as_it_is() {
        assert_eq!(
            assignment("URL", "wss://host.example:8443/ws"),
            "URL=wss://host.example:8443/ws"
        );
        assert_eq!(assignment("KEY", "abc+/Ro="), "KEY=abc+/Ro=");
        assert_eq!(assignment("H", "Web_Server-01"), "H=Web_Server-01");
    }

    #[test]
    fn a_percent_sign_is_doubled() {
        assert_eq!(assignment("H", "a%b"), "H=a%%b");
        assert_eq!(assignment("H", "a %b"), "\"H=a %%b\"");
    }

    #[test]
    fn any_other_value_is_quoted_and_escaped() {
        assert_eq!(assignment("H", "my host"), "\"H=my host\"");
        assert_eq!(assignment("H", r#"a"b\c"#), r#""H=a\"b\\c""#);
        assert_eq!(assignment("H", "it's"), "\"H=it's\"");
        assert_eq!(
            assignment("H", "x\nExecStartPre=/bin/evil\r\t\u{1}"),
            r#""H=x\nExecStartPre=/bin/evil\r\t\x01""#
        );
    }

    #[test]
    fn a_written_assignment_is_one_word() {
        for value in [
            "plain",
            "with space",
            "q\"uo'te",
            "back\\slash",
            "line\nbreak",
        ] {
            let word = assignment("BORG_HOSTNAME", value);
            assert_eq!(split_words(&word), [word.as_str()], "{value:?}");
            assert!(is_assignment(&word), "{value:?}");
        }
    }
}
