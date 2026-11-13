// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Draining a child process's output pipe while a consumer watches it live.

use std::io;

use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};

/// Reads `reader` to the end and returns every byte it produced, handing each
/// non-empty line to `on_line` as soon as its newline arrives. The line has
/// its terminator removed and any invalid UTF-8 replaced, so a stray byte in a
/// child's output loses a character instead of failing the read. The last line
/// is passed on even when the pipe closes before its newline.
pub(crate) async fn drain_lines(
    reader: impl AsyncRead + Unpin,
    mut on_line: impl FnMut(&str),
) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(reader);
    let mut everything = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).await? == 0 {
            return Ok(everything);
        }
        everything.extend_from_slice(&line);
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches('\n').trim_end_matches('\r');
        if !text.is_empty() {
            on_line(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn drained(input: &[u8]) -> (Vec<u8>, Vec<String>) {
        let mut lines = Vec::new();
        let everything = drain_lines(input, |line| lines.push(line.to_owned()))
            .await
            .unwrap();
        (everything, lines)
    }

    #[tokio::test]
    async fn returns_every_byte_and_passes_on_each_line_without_its_terminator() {
        let (everything, lines) = drained(b"one\r\ntwo\nthree").await;

        assert_eq!(everything, b"one\r\ntwo\nthree");
        assert_eq!(lines, ["one", "two", "three"]);
    }

    #[tokio::test]
    async fn empty_lines_are_not_passed_on_but_are_still_returned() {
        let (everything, lines) = drained(b"a\n\n\r\nb\n").await;

        assert_eq!(everything, b"a\n\n\r\nb\n");
        assert_eq!(lines, ["a", "b"]);
    }

    #[tokio::test]
    async fn a_byte_that_is_not_utf8_does_not_fail_the_read() {
        let (everything, lines) = drained(b"ok\nbad \xff byte\n").await;

        assert_eq!(everything, b"ok\nbad \xff byte\n");
        assert_eq!(lines, ["ok", "bad \u{fffd} byte"]);
    }
}
