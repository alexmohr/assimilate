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

fn rows(raw: &str) -> Vec<FileChangePatternRow> {
    grammar::parse_file_change_patterns(raw)
        .into_iter()
        .map(FileChangePatternRow::from)
        .collect()
}

fn serialize_rows(rows: Vec<FileChangePatternRow>) -> String {
    let patterns: Vec<FileChangePattern> = rows.into_iter().map(FileChangePattern::from).collect();
    grammar::serialize_file_change_patterns(&patterns)
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
    serde_wasm_bindgen::to_value(&rows(raw)).map_err(|e| JsError::new(&e.to_string()))
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
    let rows = serde_wasm_bindgen::from_value(rows).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(serialize_rows(rows))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FileChangePatternRow, rows, serialize_rows};

    #[test]
    fn rows_use_the_lowercase_keywords_the_frontend_expects() {
        let rows = serde_json::to_value(rows("*/tmp* ignore\n*/etc*\n*/var/log* fatal")).unwrap();
        assert_eq!(
            rows,
            json!([
                { "path": "*/tmp*", "action": "ignore" },
                { "path": "*/etc*", "action": "warn" },
                { "path": "*/var/log*", "action": "fatal" },
            ])
        );
    }

    #[test]
    fn rows_from_the_frontend_serialize_back_to_the_raw_form() {
        let rows: Vec<FileChangePatternRow> = serde_json::from_value(json!([
            { "path": "*/tmp*", "action": "ignore" },
            { "path": "*/etc*", "action": "warn" },
        ]))
        .unwrap();
        assert_eq!(serialize_rows(rows), "*/tmp* ignore\n*/etc*");
    }

    #[test]
    fn an_unknown_action_from_the_frontend_is_rejected() {
        let rows = serde_json::from_value::<Vec<FileChangePatternRow>>(json!([
            { "path": "*/tmp*", "action": "explode" },
        ]));
        assert!(rows.is_err());
    }
}
