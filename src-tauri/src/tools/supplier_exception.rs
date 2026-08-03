use crate::{
    foundation::digest::sha256_hex,
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const OPERATION: &str = "analyze_supplier_exceptions";
const MAX_FIXTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SUPPLIERS: usize = 10_000;
pub(crate) const MAX_ANALYSIS_JSON_BYTES: usize = 6 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzeSupplierExceptionsRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
struct SupplierFixture {
    #[serde(default)]
    audit_year: Option<i32>,
    #[serde(default)]
    quarter: Option<String>,
    suppliers: Vec<SupplierQuote>,
}

#[derive(Debug, Deserialize)]
struct SupplierQuote {
    name: String,
    historical_settled_rate: f64,
    active_quote: f64,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupplierExceptionAnalysis {
    source_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    audit_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quarter: Option<String>,
    supplier_count: usize,
    exception_count: usize,
    has_exception: bool,
    suppliers: Vec<SupplierVariance>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupplierVariance {
    name: String,
    historical_settled_rate: f64,
    active_quote: f64,
    variance: f64,
    exceeds_historical: bool,
    status: String,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Analyze supplier quote fixture bytes deterministically and return typed active-versus-settled variances.",
            risk_tier: TaskToolRiskTier::ReadOnly,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "supplier_exception_analysis_failed",
            agent_error_boundary: "SupplierExceptionAnalysis",
            execution_path: "The native analyze_supplier_exceptions tool parsed the exact local fixture bytes and calculated each supplier variance without model inference.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "content":{
                "type":"string",
                "minLength":1,
                "maxLength":MAX_FIXTURE_BYTES,
                "description":"Exact UTF-8 JSON content returned by the approved local fixture read."
            }
        },
        "required":["content"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request =
        serde_json::from_value::<AnalyzeSupplierExceptionsRequest>(arguments).map_err(|_| {
            "analyze_supplier_exceptions arguments do not match the registered schema.".to_string()
        })?;
    analyze_supplier_fixture(&request.content)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: false,
    })
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<AnalyzeSupplierExceptionsRequest>(arguments)
            .map_err(|_| {
                "analyze_supplier_exceptions arguments do not match the registered schema."
                    .to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Supplier analysis requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let analysis = analyze_supplier_fixture(&request.content)?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "supplier_exception.analyzed",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "sourceSha256":analysis["sourceSha256"],
                "auditYear":analysis["auditYear"],
                "quarter":analysis["quarter"],
                "supplierCount":analysis["supplierCount"],
                "exceptionCount":analysis["exceptionCount"],
                "hasException":analysis["hasException"],
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&analysis).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![format!(
                "CLAIM supplier_exception_analysis=true source_sha256={} supplier_count={} exception_count={}",
                analysis["sourceSha256"].as_str().unwrap_or_default(),
                analysis["supplierCount"].as_u64().unwrap_or_default(),
                analysis["exceptionCount"].as_u64().unwrap_or_default()
            )],
            verified: true,
            model_used: None,
        })
    })
}

pub(crate) fn analyze_supplier_fixture(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() || content.len() > MAX_FIXTURE_BYTES {
        return Err("Supplier fixture content is empty or exceeds the analysis limit.".to_string());
    }
    let fixture = serde_json::from_str::<SupplierFixture>(content)
        .map_err(|_| "Supplier fixture is not valid supplier-proposal JSON.".to_string())?;
    if fixture
        .audit_year
        .is_some_and(|year| !(2000..=2100).contains(&year))
    {
        return Err("Supplier fixture audit year is outside the bounded contract.".to_string());
    }
    let quarter = fixture
        .quarter
        .map(|value| value.trim().to_ascii_uppercase());
    if quarter
        .as_deref()
        .is_some_and(|value| !matches!(value, "Q1" | "Q2" | "Q3" | "Q4"))
    {
        return Err("Supplier fixture quarter is outside the bounded contract.".to_string());
    }
    if fixture.suppliers.is_empty() || fixture.suppliers.len() > MAX_SUPPLIERS {
        return Err(
            "Supplier fixture must contain a bounded, non-empty suppliers list.".to_string(),
        );
    }
    let mut names = HashSet::new();
    let mut suppliers = Vec::with_capacity(fixture.suppliers.len());
    for quote in fixture.suppliers {
        let name = quote.name.trim().to_string();
        let status = quote.status.trim().to_string();
        if name.is_empty()
            || name.len() > 256
            || !quote.historical_settled_rate.is_finite()
            || !quote.active_quote.is_finite()
            || quote.historical_settled_rate < 0.0
            || quote.active_quote < 0.0
            || !names.insert(name.to_ascii_lowercase())
        {
            return Err("Supplier fixture contains an invalid or duplicate quote.".to_string());
        }
        let variance = quote.active_quote - quote.historical_settled_rate;
        suppliers.push(SupplierVariance {
            name,
            historical_settled_rate: quote.historical_settled_rate,
            active_quote: quote.active_quote,
            variance,
            exceeds_historical: variance > 0.0,
            status,
        });
    }
    let exception_count = suppliers
        .iter()
        .filter(|supplier| supplier.exceeds_historical)
        .count();
    let analysis = SupplierExceptionAnalysis {
        source_sha256: sha256_hex(content.as_bytes()),
        audit_year: fixture.audit_year,
        quarter,
        supplier_count: suppliers.len(),
        exception_count,
        has_exception: exception_count > 0,
        suppliers,
    };
    let encoded = serde_json::to_vec(&analysis).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_ANALYSIS_JSON_BYTES {
        return Err(
            "Supplier analysis exceeds the bounded evidence-synthesis contract.".to_string(),
        );
    }
    serde_json::from_slice(&encoded).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "audit_year": 2026,
      "quarter": "Q2",
      "suppliers": [
        {"name":"Apex Cargo","historical_settled_rate":45000,"active_quote":46500,"status":"PENDING_RECONCILIATION"},
        {"name":"Vanguard Freight","historical_settled_rate":31000,"active_quote":31000,"status":"ALIGNED"}
      ]
    }"#;

    #[test]
    fn real_fixture_bytes_drive_typed_exception_branch_without_model_inference() {
        let analysis = analyze_supplier_fixture(FIXTURE).expect("fixture analysis");
        assert_eq!(analysis["hasException"], json!(true));
        assert_eq!(analysis["auditYear"], json!(2026));
        assert_eq!(analysis["quarter"], json!("Q2"));
        assert_eq!(analysis["exceptionCount"], json!(1));
        assert_eq!(analysis["suppliers"][0]["name"], json!("Apex Cargo"));
        assert_eq!(analysis["suppliers"][0]["variance"].as_f64(), Some(1500.0));
        assert_eq!(analysis["suppliers"][1]["name"], json!("Vanguard Freight"));
        assert_eq!(analysis["suppliers"][1]["variance"].as_f64(), Some(0.0));
        assert!(serde_json::to_vec(&analysis).unwrap().len() <= MAX_ANALYSIS_JSON_BYTES);
        assert_eq!(
            crate::condition_expression::evaluate_basic_condition(
                "$.hasException == true",
                &std::collections::HashMap::new(),
                &analysis,
            ),
            Some(true)
        );
    }

    #[test]
    fn malformed_or_duplicate_supplier_fixture_is_rejected() {
        assert!(analyze_supplier_fixture("{}").is_err());
        assert!(analyze_supplier_fixture(
            r#"{"suppliers":[{"name":"Same","historical_settled_rate":1,"active_quote":2},{"name":"same","historical_settled_rate":1,"active_quote":1}]}"#
        )
        .is_err());
        let oversized = (0..MAX_SUPPLIERS)
            .map(|index| {
                json!({
                    "name":format!("Supplier {index} {}", "x".repeat(120)),
                    "historical_settled_rate":1,
                    "active_quote":2,
                    "status":"PENDING_RECONCILIATION"
                })
            })
            .collect::<Vec<_>>();
        assert!(analyze_supplier_fixture(&json!({"suppliers":oversized}).to_string()).is_err());
    }
}
