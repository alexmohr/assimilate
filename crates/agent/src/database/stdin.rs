// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a database command reads on stdin.
//!
//! A restore streams a dump out of borg into `pg_restore`, so a command may
//! read its input while its output is copied back. The input's end closes the
//! command's stdin, which is how it learns the dump is complete. An input that
//! breaks off with an error must never look like that end: the backend stops
//! the command instead, so it cannot mistake half a dump for a whole one.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::exec::ExecError;

/// Bytes copied into a command's stdin at a time.
const CHUNK_BYTES: usize = 256 * 1024;

/// What a command reads on stdin.
pub enum Stdin<'a> {
    /// Nothing: the command's stdin is closed from the start.
    Closed,
    /// Everything `reader` yields; its end closes the command's stdin. A read
    /// error stops the command without closing its stdin.
    From(&'a mut (dyn AsyncRead + Unpin + Send)),
}

impl Stdin<'_> {
    /// Whether the command gets an input to read.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::From(_))
    }

    /// Copies the input into `pipe`, the command's stdin, then closes it by
    /// shutting it down and dropping it. Does nothing without an input.
    ///
    /// A command that stops reading early is left to say how it ended: what
    /// is left of the input is not copied, and its exit decides. On a read
    /// error `pipe` is left open, so the caller can stop the command before
    /// its stdin closes.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::Input`] when the input cannot be read, and
    /// [`ExecError::Stream`] when there is an input but the command has no
    /// stdin to take it, which would otherwise leave it waiting for one.
    pub async fn feed<W: AsyncWrite + Unpin>(self, pipe: &mut Option<W>) -> Result<(), ExecError> {
        let Self::From(reader) = self else {
            return Ok(());
        };
        let writer = pipe.as_mut().ok_or_else(|| {
            ExecError::Stream("the command has no stdin for its input".to_owned())
        })?;
        let mut chunk = vec![0u8; CHUNK_BYTES];
        loop {
            let read = reader.read(&mut chunk).await.map_err(ExecError::Input)?;
            let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else {
                break;
            };
            if writer.write_all(bytes).await.is_err() {
                break;
            }
        }
        // A pipe the command already closed cannot be shut down; dropping it
        // is all that is left to do.
        let _ = writer.shutdown().await;
        *pipe = None;
        Ok(())
    }
}

#[cfg(test)]
pub mod broken {
    //! An input that breaks off, as borg's output does when borg fails.

    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use tokio::io::{AsyncRead, ReadBuf};

    /// Yields its bytes once, then fails.
    pub struct Broken(pub Option<Vec<u8>>);

    impl Broken {
        pub fn after(bytes: &[u8]) -> Self {
            Self(Some(bytes.to_vec()))
        }
    }

    impl AsyncRead for Broken {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(match self.0.take() {
                Some(data) => {
                    buf.put_slice(&data);
                    Ok(())
                }
                None => Err(std::io::Error::other("borg died")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{broken::Broken, *};

    #[tokio::test]
    async fn the_input_is_copied_and_then_closed() {
        let mut input: &[u8] = b"dump bytes";
        let stdin = Stdin::From(&mut input);
        assert!(stdin.is_open());
        let (writer, mut reader) = tokio::io::duplex(64);
        let mut pipe = Some(writer);
        stdin.feed(&mut pipe).await.unwrap();
        assert!(pipe.is_none(), "the pipe is closed after the input");
        let mut got = Vec::new();
        reader.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"dump bytes");
    }

    #[tokio::test]
    async fn without_an_input_the_pipe_is_left_alone() {
        let mut pipe = Some(Vec::<u8>::new());
        assert!(!Stdin::Closed.is_open());
        Stdin::Closed.feed(&mut pipe).await.unwrap();
        assert_eq!(pipe, Some(Vec::new()));
    }

    #[tokio::test]
    async fn an_input_for_a_command_without_stdin_is_an_error() {
        let mut input: &[u8] = b"dump bytes";
        let mut pipe: Option<Vec<u8>> = None;
        let err = Stdin::From(&mut input).feed(&mut pipe).await.unwrap_err();
        assert!(matches!(err, ExecError::Stream(_)), "{err}");
    }

    #[tokio::test]
    async fn a_command_that_stops_reading_ends_the_copy() {
        let (writer, reader) = tokio::io::duplex(4);
        drop(reader);
        let mut input: &[u8] = b"more than the command reads";
        let mut pipe = Some(writer);
        Stdin::From(&mut input).feed(&mut pipe).await.unwrap();
        assert!(pipe.is_none());
    }

    #[tokio::test]
    async fn an_input_that_breaks_off_leaves_the_pipe_open() {
        let (writer, _reader) = tokio::io::duplex(64);
        let mut input = Broken::after(b"half");
        let mut pipe = Some(writer);
        let err = Stdin::From(&mut input).feed(&mut pipe).await.unwrap_err();
        assert!(matches!(err, ExecError::Input(_)));
        assert!(err.to_string().contains("borg died"), "{err}");
        assert!(pipe.is_some(), "the caller stops the command first");
    }
}
