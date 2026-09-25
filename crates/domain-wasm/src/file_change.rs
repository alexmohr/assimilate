// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use domain::file_change::{self as grammar, FileChangePattern};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const FILE_CHANGE_PATTERN_ROW: &str = r#"
export type FileChangeAction = "ignore" | "warn" | "fatal"

export interface FileChangePatternRow {
  path: string
  action: FileChangeAction
}
"#;

/// Each rule as a `(path, action keyword)` pair; the keywords are the
/// lowercase ones the `FileChangeAction` union above lists.
fn rows(raw: &str) -> Vec<(String, String)> {
    grammar::parse_file_change_patterns(raw)
        .into_iter()
        .map(|pattern| (pattern.path, pattern.action.to_string()))
        .collect()
}

/// Rows arrive as two columns rather than objects: plain string arrays cross
/// into WebAssembly natively, so no JavaScript-object deserializer is needed.
fn serialize_columns(paths: Vec<String>, actions: Vec<String>) -> Result<String, String> {
    if paths.len() != actions.len() {
        return Err(format!(
            "{} paths but {} actions",
            paths.len(),
            actions.len()
        ));
    }
    let patterns = paths
        .into_iter()
        .zip(actions)
        .map(|(path, action)| {
            let action = action
                .parse()
                .map_err(|_| format!("unknown file-change action '{action}'"))?;
            Ok(FileChangePattern { path, action })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(grammar::serialize_file_change_patterns(&patterns))
}

/// Parses the raw textarea form into `[path, action]` pairs; see
/// [`grammar::parse_file_change_patterns`].
#[must_use]
#[wasm_bindgen(
    js_name = parseFileChangePatterns,
    unchecked_return_type = "Array<[path: string, action: FileChangeAction]>"
)]
pub fn parse_file_change_patterns(raw: &str) -> Vec<JsValue> {
    rows(raw)
        .into_iter()
        .map(|(path, action)| js_sys::Array::of2(&path.into(), &action.into()).into())
        .collect()
}

/// Serializes rows, given as parallel `paths` and `actions` columns, back into
/// the raw textarea form; see [`grammar::serialize_file_change_patterns`].
///
/// # Errors
///
/// Fails if the columns differ in length or an action is not a known keyword.
#[wasm_bindgen(js_name = serializeFileChangePatternColumns)]
pub fn serialize_file_change_pattern_columns(
    paths: Vec<String>,
    #[wasm_bindgen(unchecked_param_type = "FileChangeAction[]")] actions: Vec<String>,
) -> Result<String, JsError> {
    serialize_columns(paths, actions).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests {
    use super::{rows, serialize_columns};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn rows_use_the_lowercase_keywords_the_frontend_expects() {
        assert_eq!(
            rows("*/tmp* ignore\n*/etc*\n*/var/log* fatal"),
            vec![
                ("*/tmp*".to_owned(), "ignore".to_owned()),
                ("*/etc*".to_owned(), "warn".to_owned()),
                ("*/var/log*".to_owned(), "fatal".to_owned()),
            ]
        );
    }

    #[test]
    fn rows_from_the_frontend_serialize_back_to_the_raw_form() {
        let serialized =
            serialize_columns(strings(&["*/tmp*", "*/etc*"]), strings(&["ignore", "warn"]));
        assert_eq!(serialized.unwrap(), "*/tmp* ignore\n*/etc*");
    }

    #[test]
    fn an_unknown_action_from_the_frontend_is_rejected() {
        let err = serialize_columns(strings(&["*/tmp*"]), strings(&["explode"])).unwrap_err();
        assert_eq!(err, "unknown file-change action 'explode'");
    }

    #[test]
    fn columns_of_different_lengths_are_rejected() {
        let err =
            serialize_columns(strings(&["*/tmp*", "*/etc*"]), strings(&["warn"])).unwrap_err();
        assert_eq!(err, "2 paths but 1 actions");
    }
}
