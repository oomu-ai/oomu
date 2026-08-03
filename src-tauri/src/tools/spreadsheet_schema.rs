use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) fn create_spreadsheet_parameters_schema() -> Value {
    json!({
        "type": "object",
        "description": "Create either a self-contained workbook or a deterministic table from a prior verified connected_work result. Never put observed connector values in workbook; use sourceProjection instead.",
        "properties": {
            "workbook": workbook_schema(),
            "sourceProjection": source_projection_schema()
        },
        "oneOf": [
            { "required": ["workbook"] },
            { "required": ["sourceProjection"] }
        ],
        "additionalProperties": false
    })
}

pub(crate) fn validate_public_create_spreadsheet_envelope(value: &Value) -> Result<(), String> {
    let envelope = value
        .as_object()
        .ok_or_else(|| "create_spreadsheet input must be an object.".to_string())?;
    if envelope.len() != 1 {
        return Err(
            "create_spreadsheet requires exactly one of workbook or sourceProjection.".to_string(),
        );
    }
    if let Some(workbook) = envelope.get("workbook") {
        validate_workbook_shape(workbook)
    } else if let Some(projection) = envelope.get("sourceProjection") {
        validate_projection_shape(projection)
    } else {
        Err("create_spreadsheet contains an unsupported public envelope.".to_string())
    }
}

fn validate_workbook_shape(value: &Value) -> Result<(), String> {
    let workbook = value
        .as_object()
        .ok_or_else(|| "create_spreadsheet.workbook must be an object.".to_string())?;
    let allowed = [
        "schemaVersion",
        "title",
        "locale",
        "dateSystem",
        "revision",
        "formats",
        "worksheets",
        "namedRanges",
        "recalculation",
        "policy",
    ];
    if workbook.keys().any(|key| !allowed.contains(&key.as_str()))
        || workbook.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || workbook.get("revision").and_then(Value::as_u64) != Some(1)
        || !matches!(
            workbook.get("dateSystem").and_then(Value::as_str),
            Some("1900" | "1904")
        )
        || !bounded_string(workbook.get("title"), 1, 240)
        || !bounded_string(workbook.get("locale"), 2, 64)
    {
        return Err("create_spreadsheet.workbook header is invalid.".to_string());
    }
    let sheets = workbook
        .get("worksheets")
        .and_then(Value::as_array)
        .filter(|sheets| !sheets.is_empty() && sheets.len() <= 1_024)
        .ok_or_else(|| "create_spreadsheet.workbook worksheets are invalid.".to_string())?;
    for sheet in sheets {
        validate_worksheet_shape(sheet)?;
    }
    if !optional_bounded_array(workbook.get("formats"), 10_000)
        || !optional_bounded_array(workbook.get("namedRanges"), 10_000)
        || workbook
            .get("recalculation")
            .is_some_and(|value| !value.is_object())
        || workbook
            .get("policy")
            .is_some_and(|value| !value.is_object())
    {
        return Err("create_spreadsheet.workbook exceeds a collection bound.".to_string());
    }
    Ok(())
}

fn validate_worksheet_shape(value: &Value) -> Result<(), String> {
    let sheet = value
        .as_object()
        .ok_or_else(|| "create_spreadsheet worksheet must be an object.".to_string())?;
    let allowed = [
        "sheetId",
        "name",
        "bounds",
        "visibility",
        "critical",
        "cells",
        "mergedRanges",
        "columnWidths",
        "tables",
        "validations",
        "charts",
    ];
    let bounds = sheet.get("bounds").and_then(Value::as_object);
    if sheet.keys().any(|key| !allowed.contains(&key.as_str()))
        || !bounded_string(sheet.get("sheetId"), 1, 256)
        || !bounded_string(sheet.get("name"), 1, 31)
        || bounds.is_none_or(|bounds| {
            bounds.len() != 2
                || !bounded_integer(bounds.get("rowCount"), 1, 1_048_576)
                || !bounded_integer(bounds.get("columnCount"), 1, 16_384)
        })
    {
        return Err("create_spreadsheet worksheet shape is invalid.".to_string());
    }
    if sheet
        .get("visibility")
        .is_some_and(|value| !matches!(value.as_str(), Some("visible" | "hidden" | "very_hidden")))
        || sheet
            .get("critical")
            .is_some_and(|value| !value.is_boolean())
        || [
            "mergedRanges",
            "columnWidths",
            "tables",
            "validations",
            "charts",
        ]
        .into_iter()
        .any(|field| sheet.get(field).is_some_and(|value| !value.is_array()))
    {
        return Err("create_spreadsheet worksheet optional fields are invalid.".to_string());
    }
    if let Some(cells) = sheet.get("cells") {
        let cells = cells
            .as_array()
            .filter(|cells| cells.len() <= 2_000_000)
            .ok_or_else(|| "create_spreadsheet worksheet cells are invalid.".to_string())?;
        for cell in cells {
            let cell = cell
                .as_object()
                .ok_or_else(|| "create_spreadsheet cell must be an object.".to_string())?;
            let allowed = ["address", "value", "formatId", "comment", "provenance"];
            if cell.keys().any(|key| !allowed.contains(&key.as_str()))
                || !bounded_string(cell.get("address"), 2, 16)
                || !cell.get("value").is_some_and(Value::is_object)
                || cell.get("formatId").is_some_and(|value| !value.is_string())
                || cell.get("comment").is_some_and(|value| !value.is_object())
                || cell
                    .get("provenance")
                    .is_some_and(|value| value.as_array().is_none_or(|values| !values.is_empty()))
            {
                return Err("create_spreadsheet cell shape is invalid.".to_string());
            }
        }
    }
    Ok(())
}

fn validate_projection_shape(value: &Value) -> Result<(), String> {
    let projection = value
        .as_object()
        .ok_or_else(|| "create_spreadsheet.sourceProjection must be an object.".to_string())?;
    let required = [
        "fromStep",
        "collectionPointer",
        "title",
        "locale",
        "sheetName",
        "columns",
    ];
    if projection.len() != required.len()
        || projection
            .keys()
            .any(|key| !required.contains(&key.as_str()))
        || !bounded_integer(projection.get("fromStep"), 0, 31)
        || !matches!(
            projection.get("collectionPointer").and_then(Value::as_str),
            Some("/result" | "/result/value")
        )
        || !bounded_string(projection.get("title"), 1, 240)
        || !bounded_string(projection.get("locale"), 2, 64)
        || !bounded_string(projection.get("sheetName"), 1, 31)
    {
        return Err("create_spreadsheet.sourceProjection shape is invalid.".to_string());
    }
    let columns = projection
        .get("columns")
        .and_then(Value::as_array)
        .filter(|columns| !columns.is_empty() && columns.len() <= 64)
        .ok_or_else(|| "create_spreadsheet.sourceProjection columns are invalid.".to_string())?;
    let mut headers = HashSet::new();
    let mut fields = HashSet::new();
    for column in columns {
        let column = column
            .as_object()
            .filter(|column| {
                column.len() == 2 && column.contains_key("header") && column.contains_key("field")
            })
            .ok_or_else(|| "create_spreadsheet projection column is invalid.".to_string())?;
        let header = column.get("header").and_then(Value::as_str).unwrap_or("");
        let field = column.get("field").and_then(Value::as_str).unwrap_or("");
        if header.is_empty()
            || header.chars().count() > 255
            || !safe_field(field)
            || !headers.insert(header.to_ascii_lowercase())
            || !fields.insert(field)
        {
            return Err("create_spreadsheet projection column is invalid.".to_string());
        }
    }
    Ok(())
}

fn bounded_string(value: Option<&Value>, minimum: usize, maximum: usize) -> bool {
    value.and_then(Value::as_str).is_some_and(|value| {
        let length = value.chars().count();
        !value.trim().is_empty() && length >= minimum && length <= maximum
    })
}

fn optional_bounded_array(value: Option<&Value>, maximum: usize) -> bool {
    value.is_none_or(|value| {
        value
            .as_array()
            .is_some_and(|values| values.len() <= maximum)
    })
}

fn bounded_integer(value: Option<&Value>, minimum: u64, maximum: u64) -> bool {
    value
        .and_then(Value::as_u64)
        .is_some_and(|value| (minimum..=maximum).contains(&value))
}

fn safe_field(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
}

fn workbook_schema() -> Value {
    json!({
        "type": "object",
        "description": "A complete bounded WorkbookIr for self-contained values only. Cell values use kind=blank|text|number|boolean|date|formula. Formulas, formats, tables, validations, and charts are checked again by the native workbook validator.",
        "properties": {
            "schemaVersion": { "type": "integer", "enum": [1] },
            "title": { "type": "string", "minLength": 1, "maxLength": 240 },
            "locale": { "type": "string", "minLength": 2, "maxLength": 64 },
            "dateSystem": { "type": "string", "enum": ["1900", "1904"] },
            "revision": { "type": "integer", "enum": [1] },
            "formats": { "type": "array", "maxItems": 10000 },
            "worksheets": {
                "type": "array",
                "minItems": 1,
                "maxItems": 1024,
                "items": worksheet_schema()
            },
            "namedRanges": { "type": "array", "maxItems": 10000 },
            "recalculation": { "type": "object" },
            "policy": { "type": "object" }
        },
        "required": ["schemaVersion", "title", "locale", "dateSystem", "revision", "worksheets"],
        "additionalProperties": false
    })
}

fn worksheet_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sheetId": { "type": "string", "minLength": 1, "maxLength": 256 },
            "name": { "type": "string", "minLength": 1, "maxLength": 31 },
            "bounds": {
                "type": "object",
                "properties": {
                    "rowCount": { "type": "integer", "minimum": 1, "maximum": 1048576 },
                    "columnCount": { "type": "integer", "minimum": 1, "maximum": 16384 }
                },
                "required": ["rowCount", "columnCount"],
                "additionalProperties": false
            },
            "visibility": { "type": "string", "enum": ["visible", "hidden", "very_hidden"] },
            "critical": { "type": "boolean" },
            "cells": {
                "type": "array",
                "maxItems": 2000000,
                "items": {
                    "type": "object",
                    "description": "A1-addressed cell. Use value.kind plus its matching value, iso, expression, or cachedValue field. Direct workbook cells cannot claim provenance.",
                    "properties": {
                        "address": { "type": "string", "minLength": 2, "maxLength": 16 },
                        "value": { "type": "object" },
                        "formatId": { "type": "string" },
                        "comment": { "type": "object" },
                        "provenance": { "type": "array", "maxItems": 0 }
                    },
                    "required": ["address", "value"],
                    "additionalProperties": false
                }
            },
            "mergedRanges": { "type": "array" },
            "columnWidths": { "type": "array" },
            "tables": { "type": "array" },
            "validations": { "type": "array" },
            "charts": { "type": "array" }
        },
        "required": ["sheetId", "name", "bounds"],
        "additionalProperties": false
    })
}

fn source_projection_schema() -> Value {
    json!({
        "type": "object",
        "description": "Build a formula-free table at execution time from scalar fields in one prior verified connected_work result. The model supplies field mappings, never observed cell values or evidence references.",
        "properties": {
            "fromStep": { "type": "integer", "minimum": 0, "maximum": 31 },
            "collectionPointer": {
                "type": "string",
                "enum": ["/result", "/result/value"]
            },
            "title": { "type": "string", "minLength": 1, "maxLength": 240 },
            "locale": { "type": "string", "minLength": 2, "maxLength": 64 },
            "sheetName": { "type": "string", "minLength": 1, "maxLength": 31 },
            "columns": {
                "type": "array",
                "minItems": 1,
                "maxItems": 64,
                "items": {
                    "type": "object",
                    "properties": {
                        "header": { "type": "string", "minLength": 1, "maxLength": 255 },
                        "field": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 64,
                            "pattern": "^[A-Za-z_][A-Za-z0-9_]{0,63}$"
                        }
                    },
                    "required": ["header", "field"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["fromStep", "collectionPointer", "title", "locale", "sheetName", "columns"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_exposes_two_exclusive_closed_envelopes() {
        let schema = create_spreadsheet_parameters_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
        assert_eq!(
            schema["properties"]["sourceProjection"]["properties"]["collectionPointer"]["enum"],
            json!(["/result", "/result/value"])
        );
        assert_eq!(
            schema["properties"]["sourceProjection"]["properties"]["columns"]["maxItems"],
            json!(64)
        );
        assert_eq!(
            schema["properties"]["sourceProjection"]["properties"]["columns"]["items"]
                ["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn neutral_validator_matches_closed_public_shapes_and_character_bounds() {
        let direct = json!({
            "workbook": {
                "schemaVersion": 1,
                "title": "Summary",
                "locale": "en-US",
                "dateSystem": "1900",
                "revision": 1,
                "worksheets": [{
                    "sheetId": "summary",
                    "name": "Summary",
                    "bounds": {"rowCount": 1, "columnCount": 1},
                    "cells": [{"address":"A1","value":{"kind":"blank"}}]
                }]
            }
        });
        let projection = json!({
            "sourceProjection": {
                "fromStep": 0,
                "collectionPointer": "/result/value",
                "title": "Observed",
                "locale": "éé",
                "sheetName": "Rows",
                "columns": [{"header":"Subject","field":"subject"}]
            }
        });
        validate_public_create_spreadsheet_envelope(&direct).unwrap();
        validate_public_create_spreadsheet_envelope(&projection).unwrap();

        for invalid in [
            json!({}),
            json!({"resolvedSourceProjection":{}}),
            json!({"workbook":direct["workbook"],"sourceProjection":projection["sourceProjection"]}),
            json!({"sourceProjection":{
                "fromStep":0,"collectionPointer":"/result/value","title":"   ",
                "locale":"en-US","sheetName":"Rows",
                "columns":[{"header":"Subject","field":"subject"}]
            }}),
            json!({"sourceProjection":{
                "fromStep":0,"collectionPointer":"/result/value","title":"Observed",
                "locale":"a","sheetName":"Rows",
                "columns":[{"header":"Subject","field":"subject"}]
            }}),
            json!({"workbook":{
                "schemaVersion":1,"title":"Summary","locale":"en-US","dateSystem":"1900",
                "revision":1,"formats":{},
                "worksheets":[{"sheetId":"summary","name":"Summary","bounds":{"rowCount":1,"columnCount":1}}]
            }}),
            json!({"workbook":{
                "schemaVersion":1,"title":"Summary","locale":"en-US","dateSystem":"1900",
                "revision":1,"recalculation":[],
                "worksheets":[{"sheetId":"summary","name":"Summary","bounds":{"rowCount":1,"columnCount":1}}]
            }}),
            json!({"workbook":{
                "schemaVersion":1,"title":"Summary","locale":"en-US","dateSystem":"1900",
                "revision":1,
                "worksheets":[{"sheetId":"summary","name":"Summary","bounds":{"rowCount":1,"columnCount":1},"critical":"yes"}]
            }}),
        ] {
            assert!(validate_public_create_spreadsheet_envelope(&invalid).is_err());
        }
    }
}
