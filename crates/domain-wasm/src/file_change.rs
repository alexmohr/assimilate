// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use domain::file_change::{self as grammar, FileChangePattern};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const FILE_CHANGE_PATTERN_ROW: &str = r#"
export type FileChangeAction = "ignore" | "warn" | "fatal"

export interface FileChangePatternRow {
  path: string
  action: FileChangeAction
}
"#;

/// The action keyword as the frontend sees it; must match the
/// `FileChangeAction` union in the TypeScript section above.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FileChangeAction {
    Ignore,
    Warn,
    Fatal,
}

impl From<grammar::FileChangeAction> for FileChangeAction {
    fn from(action: grammar::FileChangeAction) -> Self {
        match action {
            grammar::FileChangeAction::Ignore => Self::Ignore,
            grammar::FileChangeAction::Warn => Self::Warn,
            grammar::FileChangeAction::Fatal => Self::Fatal,
        }
    }
}

impl From<FileChangeAction> for grammar::FileChangeAction {
    fn from(action: FileChangeAction) -> Self {
        match action {
            FileChangeAction::Ignore => Self::Ignore,
            FileChangeAction::Warn => Self::Warn,
            FileChangeAction::Fatal => Self::Fatal,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileChangePatternRow {
    path: String,
    action: FileChangeAction,
}

impl From<FileChangePattern> for FileChangePatternRow {
    fn from(pattern: FileChangePattern) -> Self {
        Self {
            path: pattern.path,
            action: pattern.action.into(),
        }
    }
}

impl From<FileChangePatternRow> for FileChangePattern {
    fn from(row: FileChangePatternRow) -> Self {
        Self {
            path: row.path,
            action: row.action.into(),
        }
    }
}

/// Parses the raw textarea form into rows; see [`grammar::parse_file_change_patterns`].
///
/// # Errors
///
/// Fails only if the rows cannot be converted into JavaScript values.
#[wasm_bindgen(
    js_name = parseFileChangePatterns,
    unchecked_return_type = "FileChangePatternRow[]"
)]
pub fn parse_file_change_patterns(raw: &str) -> Result<JsValue, JsError> {
    let rows: Vec<FileChangePatternRow> = grammar::parse_file_change_patterns(raw)
        .into_iter()
        .map(FileChangePatternRow::from)
        .collect();
    serde_wasm_bindgen::to_value(&rows).map_err(|e| JsError::new(&e.to_string()))
}

/// Serializes rows back into the raw textarea form; see
/// [`grammar::serialize_file_change_patterns`].
///
/// # Errors
///
/// Fails if `rows` is not an array of `FileChangePatternRow` objects.
#[wasm_bindgen(js_name = serializeFileChangePatterns)]
pub fn serialize_file_change_patterns(
    #[wasm_bindgen(unchecked_param_type = "FileChangePatternRow[]")] rows: JsValue,
) -> Result<String, JsError> {
    let rows: Vec<FileChangePatternRow> =
        serde_wasm_bindgen::from_value(rows).map_err(|e| JsError::new(&e.to_string()))?;
    let patterns: Vec<FileChangePattern> = rows.into_iter().map(FileChangePattern::from).collect();
    Ok(grammar::serialize_file_change_patterns(&patterns))
}
