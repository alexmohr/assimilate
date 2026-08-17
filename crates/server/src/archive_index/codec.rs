// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Packed encoding for the children of a single archive directory.
//!
//! The content index stores one blob per `(archive, directory)` rather than one
//! row per file, so the per-entry cost is a few dozen bytes of payload instead of
//! a ~117-byte heap tuple plus ~92 bytes of index entries. Entries are serialised
//! as escaped TSV - one line per entry, fields `name`, `type`, `size`, `mtime`,
//! `mode` - and the result is LZ4-compressed. Directory listings repeat the same
//! `mode` string and the same `mtime` prefix over and over, which is exactly what
//! LZ4 collapses.
//!
//! `name`, `type`, `mtime` and `mode` are escaped (`\` to `\\`, tab to `\t`,
//! newline to `\n`) so that a file name containing a tab or a newline - both legal
//! on Linux - cannot corrupt the framing. Escaping guarantees no raw tab or
//! newline survives inside a field, so decoding can split on those bytes directly.

use std::borrow::Cow;

/// A single entry in a directory listing, stored relative to its parent
/// directory: `name` is the final path segment, not the full path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Final path segment of the entry.
    pub name: String,
    /// Borg entry type (`d` for directories, `-` and so on).
    pub entry_type: String,
    /// Size in bytes.
    pub size: i64,
    /// Modification timestamp, verbatim as borg reported it.
    pub mtime: String,
    /// Mode/permission string, verbatim as borg reported it.
    pub mode: String,
}

/// Number of fields in the encoded form of a single entry.
const FIELD_COUNT: usize = 5;

/// Failure modes when decoding a stored directory blob.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The blob could not be LZ4-decompressed.
    #[error("failed to decompress archive directory blob: {0}")]
    Decompress(#[from] lz4_flex::block::DecompressError),
    /// The decompressed blob was not valid UTF-8.
    #[error("archive directory blob is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    /// An entry line did not have exactly [`FIELD_COUNT`] fields.
    #[error("archive directory entry has {0} fields, expected {FIELD_COUNT}")]
    FieldCount(usize),
    /// The `size` field was not a valid integer.
    #[error("archive directory entry has an invalid size: {0}")]
    Size(#[from] std::num::ParseIntError),
    /// The line ended with a dangling escape character.
    #[error("archive directory entry ends with a trailing escape character")]
    TrailingEscape,
    /// An escape sequence used a character the encoder never emits.
    #[error("archive directory entry has an unknown escape sequence: \\{0}")]
    UnknownEscape(char),
}

/// Orders entries the way a directory listing is presented: directories first
/// (borg's `d` sorts above `-`, `l` and the other type characters when compared
/// in reverse), then by name.
///
/// Encoding applies this order so that chunk order is listing order, letting a
/// limited listing read only the leading chunks.
fn listing_order(left: &DirEntry, right: &DirEntry) -> std::cmp::Ordering {
    right
        .entry_type
        .cmp(&left.entry_type)
        .then_with(|| left.name.cmp(&right.name))
}

/// Sorts `entries` into listing order in place.
pub fn sort_for_listing(entries: &mut [DirEntry]) {
    entries.sort_by(listing_order);
}

fn escape(value: &str) -> Cow<'_, str> {
    if !value.contains(['\\', '\t', '\n']) {
        return Cow::Borrowed(value);
    }

    Cow::Owned(value.chars().fold(
        String::with_capacity(value.len().saturating_add(8)),
        |mut acc, ch| {
            match ch {
                '\\' => acc.push_str("\\\\"),
                '\t' => acc.push_str("\\t"),
                '\n' => acc.push_str("\\n"),
                other => acc.push(other),
            }
            acc
        },
    ))
}

fn unescape(value: &str) -> Result<String, CodecError> {
    if !value.contains('\\') {
        return Ok(value.to_owned());
    }

    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => return Err(CodecError::UnknownEscape(other)),
            None => return Err(CodecError::TrailingEscape),
        }
    }
    Ok(out)
}

fn encode_entry(entry: &DirEntry) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\n",
        escape(&entry.name),
        escape(&entry.entry_type),
        entry.size,
        escape(&entry.mtime),
        escape(&entry.mode),
    )
}

fn decode_entry(line: &str) -> Result<DirEntry, CodecError> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [name, entry_type, size, mtime, mode] = fields.as_slice() else {
        return Err(CodecError::FieldCount(fields.len()));
    };

    Ok(DirEntry {
        name: unescape(name)?,
        entry_type: unescape(entry_type)?,
        size: size.parse()?,
        mtime: unescape(mtime)?,
        mode: unescape(mode)?,
    })
}

/// Encodes one chunk of directory children into a compressed blob.
///
/// `entries` are expected to already be in [`sort_for_listing`] order.
pub fn encode(entries: &[DirEntry]) -> Vec<u8> {
    let plain: String = entries.iter().map(encode_entry).collect();
    lz4_flex::block::compress_prepend_size(plain.as_bytes())
}

/// Decodes a blob previously produced by [`encode`].
///
/// # Errors
///
/// Returns [`CodecError`] if the blob is not decompressible, is not UTF-8, or
/// holds a malformed entry line.
pub fn decode(blob: &[u8]) -> Result<Vec<DirEntry>, CodecError> {
    let plain = String::from_utf8(lz4_flex::block::decompress_size_prepended(blob)?)?;
    plain.split_terminator('\n').map(decode_entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, entry_type: &str, size: i64) -> DirEntry {
        DirEntry {
            name: name.to_owned(),
            entry_type: entry_type.to_owned(),
            size,
            mtime: "2026-06-05T12:00:00.000000".to_owned(),
            mode: "-rw-r--r--".to_owned(),
        }
    }

    #[test]
    fn roundtrips_a_plain_listing() {
        let entries = vec![
            entry("config.toml", "-", 1024),
            entry("src", "d", 0),
            entry("link", "l", 7),
        ];

        assert_eq!(decode(&encode(&entries)).unwrap(), entries);
    }

    #[test]
    fn roundtrips_an_empty_listing() {
        assert_eq!(decode(&encode(&[])).unwrap(), Vec::new());
    }

    #[test]
    fn roundtrips_names_containing_tabs_and_newlines() {
        let entries = vec![
            entry("weird\tname", "-", 1),
            entry("multi\nline", "-", 2),
            entry("back\\slash", "-", 3),
            entry("all\\\t\n", "-", 4),
        ];

        assert_eq!(decode(&encode(&entries)).unwrap(), entries);
    }

    #[test]
    fn roundtrips_non_ascii_names() {
        let entries = vec![entry(
            "Ordner-\u{dc}bersicht-\u{65e5}\u{672c}\u{8a9e}-\u{1f389}",
            "d",
            0,
        )];

        assert_eq!(decode(&encode(&entries)).unwrap(), entries);
    }

    #[test]
    fn roundtrips_extreme_sizes() {
        let entries = vec![entry("huge", "-", i64::MAX), entry("zero", "-", 0)];

        assert_eq!(decode(&encode(&entries)).unwrap(), entries);
    }

    #[test]
    fn preserves_mtime_and_mode_verbatim() {
        let mut odd = entry("file", "-", 1);
        odd.mtime = "2026-06-05T12:00:00.123456".to_owned();
        odd.mode = "drwxr-sr-t".to_owned();

        let decoded = decode(&encode(std::slice::from_ref(&odd))).unwrap();

        assert_eq!(decoded.first().unwrap().mtime, "2026-06-05T12:00:00.123456");
        assert_eq!(decoded.first().unwrap().mode, "drwxr-sr-t");
    }

    #[test]
    fn sorts_directories_before_files_then_by_name() {
        let mut entries = vec![
            entry("zebra", "-", 1),
            entry("beta", "d", 0),
            entry("alpha", "-", 1),
            entry("alpha-dir", "d", 0),
        ];

        sort_for_listing(&mut entries);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha-dir", "beta", "alpha", "zebra"]);
    }

    #[test]
    fn compresses_repetitive_listings() {
        let entries: Vec<DirEntry> = (0..500)
            .map(|i| entry(&format!("module_{i:05}.rs"), "-", i))
            .collect();

        let plain: usize = entries.iter().map(|e| encode_entry(e).len()).sum();

        assert!(
            encode(&entries).len() * 3 < plain,
            "expected better than 3x compression on a repetitive listing"
        );
    }

    #[test]
    fn rejects_a_truncated_blob() {
        let entries: Vec<DirEntry> = (0..200)
            .map(|i| entry(&format!("module_{i:05}.rs"), "-", i))
            .collect();
        let blob = encode(&entries);
        let truncated = blob.get(..blob.len() / 2).unwrap().to_vec();

        assert!(matches!(decode(&truncated), Err(CodecError::Decompress(_))));
    }

    #[test]
    fn rejects_an_entry_with_missing_fields() {
        let blob = lz4_flex::block::compress_prepend_size(b"name\t-\t12\n");

        assert!(matches!(decode(&blob), Err(CodecError::FieldCount(3))));
    }

    #[test]
    fn rejects_an_entry_with_a_non_numeric_size() {
        let blob = lz4_flex::block::compress_prepend_size(b"name\t-\tbig\tmtime\tmode\n");

        assert!(matches!(decode(&blob), Err(CodecError::Size(_))));
    }

    #[test]
    fn rejects_an_unknown_escape_sequence() {
        let blob = lz4_flex::block::compress_prepend_size(b"na\\qme\t-\t1\tmtime\tmode\n");

        assert!(matches!(decode(&blob), Err(CodecError::UnknownEscape('q'))));
    }

    #[test]
    fn rejects_a_trailing_escape() {
        let blob = lz4_flex::block::compress_prepend_size(b"name\\\t-\t1\tmtime\tmode\n");

        assert!(matches!(decode(&blob), Err(CodecError::TrailingEscape)));
    }
}
