// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Reading the SSH wire encoding (RFC 4251 section 5) the agent protocol's
//! messages are written in: big-endian `uint32`s, single bytes, booleans and
//! length-prefixed strings.

/// A message body that does not decode as the fields it should hold.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    /// The body ends before a field it should hold.
    #[error("the message ends before a field")]
    Truncated,
    /// The body holds bytes after its last field.
    #[error("the message has bytes after its last field")]
    TrailingBytes,
}

/// Reads fields from the front of a message body.
#[derive(Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    /// The next `uint32`.
    pub fn u32(&mut self) -> Result<u32, WireError> {
        let (value, rest) = self
            .rest
            .split_first_chunk::<4>()
            .ok_or(WireError::Truncated)?;
        self.rest = rest;
        Ok(u32::from_be_bytes(*value))
    }

    /// The next single byte.
    pub fn byte(&mut self) -> Result<u8, WireError> {
        let (&value, rest) = self.rest.split_first().ok_or(WireError::Truncated)?;
        self.rest = rest;
        Ok(value)
    }

    /// The next `boolean`; any byte other than zero is true.
    pub fn boolean(&mut self) -> Result<bool, WireError> {
        self.byte().map(|value| value != 0)
    }

    /// The next `string`: its bytes, without the length in front of them.
    pub fn string(&mut self) -> Result<&'a [u8], WireError> {
        let len = usize::try_from(self.u32()?).map_err(|_| WireError::Truncated)?;
        if self.rest.len() < len {
            return Err(WireError::Truncated);
        }
        let (value, rest) = self.rest.split_at(len);
        self.rest = rest;
        Ok(value)
    }

    /// Checks that every field has been read.
    pub fn finish(self) -> Result<(), WireError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_read_in_order() {
        let bytes = [&[0, 0, 0, 7][..], &[9], &[1], &[0, 0, 0, 2], b"hi"].concat();
        let mut reader = Reader::new(&bytes);

        assert_eq!(reader.u32(), Ok(7));
        assert_eq!(reader.byte(), Ok(9));
        assert_eq!(reader.boolean(), Ok(true));
        assert_eq!(reader.string(), Ok(&b"hi"[..]));
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_string_longer_than_the_rest_is_truncated() {
        let bytes = [&[0, 0, 0, 5][..], b"abc"].concat();

        assert_eq!(Reader::new(&bytes).string(), Err(WireError::Truncated));
        assert_eq!(Reader::new(&[0, 0]).u32(), Err(WireError::Truncated));
        assert_eq!(Reader::new(&[]).byte(), Err(WireError::Truncated));
    }

    #[test]
    fn unread_bytes_are_an_error_at_the_end() {
        let mut reader = Reader::new(&[1, 2]);
        reader.byte().unwrap();

        assert_eq!(reader.finish(), Err(WireError::TrailingBytes));
    }
}
