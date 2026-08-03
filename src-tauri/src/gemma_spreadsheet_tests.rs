use super::*;
use serde_json::json;

#[test]
fn strict_action_plan_enforces_public_spreadsheet_one_of() {
    let valid = json!({
        "kind":"create_spreadsheet",
        "sourceProjection":{
            "fromStep":0,
            "collectionPointer":"/result/value",
            "title":"Messages",
            "locale":"en-US",
            "sheetName":"Messages",
            "columns":[{"header":"Subject","field":"subject"}]
        }
    });
    validate_generated_tool_schema(&valid).unwrap();
    validate_generated_tool_schema(&json!({
        "kind":"create_spreadsheet",
        "workbook":{
            "schemaVersion":1,
            "title":"Summary",
            "locale":"en-US",
            "dateSystem":"1900",
            "revision":1,
            "worksheets":[{
                "sheetId":"summary",
                "name":"Summary",
                "bounds":{"rowCount":1,"columnCount":1}
            }]
        }
    }))
    .unwrap();
    for invalid in [
        json!({"kind":"create_spreadsheet","arbitrary":true}),
        json!({"kind":"create_spreadsheet","resolvedSourceProjection":{}}),
        json!({
            "kind":"create_spreadsheet",
            "workbook":{},
            "sourceProjection":valid["sourceProjection"].clone()
        }),
    ] {
        assert!(validate_generated_tool_schema(&invalid).is_err());
    }
}
