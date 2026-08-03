//! Provider-neutral native tool schemas and validation.
use crate::tool_security::{CapabilityClassification, CapabilityRiskTier};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::{json, Value};

mod objective_scope;
use objective_scope::{registered_task_tool_matches_objective, static_tool_matches_objective};
#[cfg(test)]
#[path = "registry/objective_scope_tests.rs"]
mod objective_scope_tests;

const PLANNER_SCHEMA_OMITTED_ANNOTATION_KEYS: &[&str] = &[
    "description",
    "title",
    "default",
    "examples",
    "example",
    "$comment",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    LocalGemmaIt,
    Gemini,
    FrontierCloud,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedParameters {
    pub tool_name: String,
    pub arguments: Value,
    pub classification: CapabilityClassification,
}

pub trait NativeTool {
    fn name(&self) -> &'static str;
    fn get_schema(&self, target_format: ModelProvider) -> Value;
    fn validate_call(&self, input: &Value) -> Result<ParsedParameters, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredNativeTool {
    FileList,
    FileRead,
    FileWrite,
    FileDelete,
    CodebasePatch,
    CodebaseCompile,
    TerminalExecute,
    SystemDiagnostics,
    SystemAudit,
    TelemetryArchive,
    SyncKnowledgeVault,
    RegisteredTaskTool(&'static str),
}

#[derive(Debug, Clone)]
pub struct NativeToolRegistry {
    tools: Vec<RegisteredNativeTool>,
}

impl Default for NativeToolRegistry {
    fn default() -> Self {
        let mut tools = vec![
            RegisteredNativeTool::FileList,
            RegisteredNativeTool::FileRead,
            RegisteredNativeTool::FileWrite,
            RegisteredNativeTool::FileDelete,
            RegisteredNativeTool::CodebasePatch,
            RegisteredNativeTool::CodebaseCompile,
            RegisteredNativeTool::TerminalExecute,
            RegisteredNativeTool::SystemDiagnostics,
            RegisteredNativeTool::SystemAudit,
            RegisteredNativeTool::TelemetryArchive,
            RegisteredNativeTool::SyncKnowledgeVault,
        ];
        tools.extend(
            crate::tools::task_tool_runtime::registered_operations()
                .into_iter()
                .map(RegisteredNativeTool::RegisteredTaskTool),
        );
        Self { tools }
    }
}

impl NativeToolRegistry {
    #[cfg(test)]
    pub fn tools(&self) -> &[RegisteredNativeTool] {
        &self.tools
    }

    pub fn schemas(&self, target_format: ModelProvider) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| tool.get_schema(target_format))
            .collect()
    }

    pub fn schema_payload(&self, target_format: ModelProvider) -> Value {
        let schemas = self.schemas(target_format);
        match target_format {
            ModelProvider::LocalGemmaIt => json!({
                "provider": "local_gemma_it",
                "tools": schemas,
            }),
            ModelProvider::Gemini => json!({
                "tools": [{
                    "functionDeclarations": schemas,
                }],
            }),
            ModelProvider::FrontierCloud => json!({
                "tools": schemas,
            }),
        }
    }

    pub fn contains(&self, tool_name: &str) -> bool {
        let normalized = normalize_tool_name(tool_name);
        self.tools.iter().any(|tool| tool.name() == normalized)
    }

    pub fn validate_call(
        &self,
        tool_name: &str,
        input: &Value,
    ) -> Result<ParsedParameters, String> {
        let normalized = normalize_tool_name(tool_name);
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == normalized)
            .ok_or_else(|| format!("Unknown native tool: {tool_name}"))?;
        tool.validate_call(input)
    }
}

impl NativeTool for RegisteredNativeTool {
    fn name(&self) -> &'static str {
        match self {
            Self::FileList => "file_list",
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::FileDelete => "delete_file",
            Self::CodebasePatch => "codebase_patch",
            Self::CodebaseCompile => "codebase_compile",
            Self::TerminalExecute => "terminal_execute",
            Self::SystemDiagnostics => "system_diagnostics",
            Self::SystemAudit => "system_audit",
            Self::TelemetryArchive => "telemetry_archive",
            Self::SyncKnowledgeVault => "sync_knowledge_vault",
            Self::RegisteredTaskTool(operation) => operation,
        }
    }

    fn get_schema(&self, target_format: ModelProvider) -> Value {
        let schema = self.parameters_schema();
        match target_format {
            ModelProvider::LocalGemmaIt => json!({
                "kind": self.name(),
                "description": self.description(),
                "inputSchema": schema,
            }),
            ModelProvider::Gemini => json!({
                "name": self.name(),
                "description": self.description(),
                "parameters": gemini_schema(&schema),
            }),
            ModelProvider::FrontierCloud => json!({
                "type": "function",
                "function": {
                    "name": self.name(),
                    "description": self.description(),
                    "parameters": schema,
                    "strict": true,
                },
            }),
        }
    }

    fn validate_call(&self, input: &Value) -> Result<ParsedParameters, String> {
        match self {
            Self::FileList => validate_file_path_tool(
                self.name(),
                input,
                CapabilityRiskTier::FileRead,
                "file_list reads sandbox directory metadata",
            ),
            Self::FileRead => validate_file_path_tool(
                self.name(),
                input,
                CapabilityRiskTier::FileRead,
                "file_read reads sandbox file contents",
            ),
            Self::FileWrite => validate_file_write_tool(input),
            Self::FileDelete => validate_file_delete_tool(input),
            Self::CodebasePatch => validate_codebase_patch_tool(input),
            Self::CodebaseCompile => validate_codebase_compile_tool(input),
            Self::TerminalExecute => validate_terminal_execute(input),
            Self::SystemDiagnostics => validate_single_string_tool(
                self.name(),
                input,
                "principal",
                CapabilityRiskTier::ReadOnly,
                "system_diagnostics collects local read-only metrics",
            ),
            Self::SystemAudit => validate_single_string_tool(
                self.name(),
                input,
                "scope",
                CapabilityRiskTier::ReadOnly,
                "system_audit collects local diagnostic snapshots",
            ),
            Self::TelemetryArchive => validate_telemetry_archive_tool(input),
            Self::SyncKnowledgeVault => validate_sync_knowledge_vault_tool(input),
            Self::RegisteredTaskTool(operation) => validate_registered_task_tool(operation, input),
        }
    }
}

impl RegisteredNativeTool {
    fn description(self) -> &'static str {
        match self {
            Self::FileList => "List files under the quarantined local workspace.",
            Self::FileRead => "Read a UTF-8 file under the quarantined local workspace.",
            Self::FileWrite => "Write UTF-8 content under the quarantined local workspace.",
            Self::FileDelete => {
                "Delete a regular file through Shield Gate after explicit approval."
            }
            Self::CodebasePatch => {
                "Apply a surgical search-and-replace patch inside the active development repository."
            }
            Self::CodebaseCompile => {
                "Run a guarded backend or frontend compile for the active development repository."
            }
            Self::TerminalExecute => {
                "Classify and request execution of a local terminal command through Shield Gate."
            }
            Self::SystemDiagnostics => "Collect read-only local system diagnostic metrics.",
            Self::SystemAudit => "Collect read-only process, disk, and network audit details.",
            Self::TelemetryArchive => {
                "Package local system diagnostics into a telemetry_audit.tar.gz archive after approval."
            }
            Self::SyncKnowledgeVault => {
                "Differentially index a user-selected knowledge vault directory into local knowledge storage."
            }
            Self::RegisteredTaskTool(operation) => {
                crate::tools::task_tool_runtime::description(operation)
                    .unwrap_or("Run a Project- and Task-bound native tool.")
            }
        }
    }

    fn parameters_schema(self) -> Value {
        match self {
            Self::FileList | Self::FileRead | Self::FileDelete => object_schema(
                vec![(
                    "path",
                    json!({
                        "type": "string",
                        "description": "Workspace-relative path inside the quarantined project root.",
                        "minLength": 1,
                    }),
                )],
                &["path"],
            ),
            Self::FileWrite => object_schema(
                vec![
                    (
                        "path",
                        json!({
                            "type": "string",
                            "description": "Workspace-relative path inside the quarantined project root.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "content",
                        json!({
                            "type": "string",
                            "description": "UTF-8 text content to write.",
                        }),
                    ),
                ],
                &["path", "content"],
            ),
            Self::CodebasePatch => object_schema(
                vec![
                    (
                        "target_file_path",
                        json!({
                            "type": "string",
                            "description": "Repository-relative file path inside the active development repository to patch.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "search_pattern",
                        json!({
                            "type": "string",
                            "description": "Existing code or text block to locate before patching.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "replacement_content",
                        json!({
                            "type": "string",
                            "description": "Replacement code or text to write at the matched location.",
                        }),
                    ),
                ],
                &["target_file_path", "search_pattern", "replacement_content"],
            ),
            Self::CodebaseCompile => object_schema(
                vec![(
                    "target",
                    json!({
                        "type": "string",
                        "description": "Compilation target to run inside the active development repository.",
                        "enum": ["backend", "frontend"],
                    }),
                )],
                &["target"],
            ),
            Self::TerminalExecute => object_schema(
                vec![
                    (
                        "executable",
                        json!({
                            "type": "string",
                            "description": "Executable name or absolute executable path.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "args",
                        json!({
                            "type": "array",
                            "description": "Arguments passed directly to the executable without a shell.",
                            "items": { "type": "string" },
                            "default": [],
                        }),
                    ),
                    (
                        "env",
                        json!({
                            "type": "object",
                            "description": "Optional environment variables for this process.",
                            "additionalProperties": { "type": "string" },
                            "default": {},
                        }),
                    ),
                    (
                        "cwd",
                        json!({
                            "type": "string",
                            "description": "Optional working directory for the command.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "timeout",
                        json!({
                            "type": "integer",
                            "description": "Optional timeout in milliseconds.",
                            "minimum": 1,
                            "maximum": crate::tools::terminal_contract::MAX_TERMINAL_TIMEOUT_MS,
                        }),
                    ),
                ],
                &["executable"],
            ),
            Self::SystemDiagnostics => object_schema(
                vec![(
                    "principal",
                    json!({
                        "type": "string",
                        "description": "Principal or local actor requesting read-only diagnostics.",
                        "minLength": 1,
                    }),
                )],
                &["principal"],
            ),
            Self::SystemAudit => object_schema(
                vec![(
                    "scope",
                    json!({
                        "type": "string",
                        "description": "Audit scope, for example process_disk_network.",
                        "minLength": 1,
                    }),
                )],
                &["scope"],
            ),
            Self::TelemetryArchive => object_schema(
                vec![(
                    "output_path",
                    json!({
                        "type": "string",
                        "description": "Absolute path to the telemetry_audit.tar.gz archive to create.",
                        "minLength": 1,
                    }),
                )],
                &["output_path"],
            ),
            Self::SyncKnowledgeVault => object_schema(
                vec![
                    (
                        "path",
                        json!({
                            "type": "string",
                            "description": "Absolute or workspace-relative directory path to index as a knowledge vault.",
                            "minLength": 1,
                        }),
                    ),
                    (
                        "max_files",
                        json!({
                            "type": "integer",
                            "description": "Maximum files to consider during one differential sync tick.",
                            "minimum": 1,
                            "maximum": 240,
                        }),
                    ),
                    (
                        "mod_id",
                        json!({
                            "type": "string",
                            "description": "Optional knowledge mod scope for the indexed vault.",
                            "minLength": 1,
                        }),
                    ),
                ],
                &["path"],
            ),
            Self::RegisteredTaskTool(operation) => {
                crate::tools::task_tool_runtime::schema(operation)
                    .unwrap_or_else(|_| json!({"type":"object","not":{}}))
            }
        }
    }
}

#[cfg(test)]
pub fn native_tool_schema_payload(target_format: ModelProvider) -> Value {
    NativeToolRegistry::default().schema_payload(target_format)
}

#[cfg(test)]
pub fn local_gemma_action_plan_contract() -> Value {
    local_gemma_action_plan_contract_with_scope(None)
}

pub fn local_gemma_action_plan_contract_for_objective(objective: &str) -> Value {
    local_gemma_action_plan_contract_with_scope(Some(objective))
}

fn local_gemma_action_plan_contract_with_scope(objective: Option<&str>) -> Value {
    let live_schema_payload =
        NativeToolRegistry::default().schema_payload(ModelProvider::LocalGemmaIt);
    let mut contract = json!({
        "actionPlanSchema": {
            "required": ["steps", "exit_condition"],
            "stepRequired": ["step", "tool", "risk_level"],
            "toolRequired": ["kind"],
            "toolEncoding": "flat",
            "toolKind": "exact key from tools",
            "riskLevel": ["low", "medium", "high"],
            "maxSteps": 32
        },
        "tools": {
            "system_diagnostics": {
                "required": ["principal"]
            },
            "get_system_metrics": {
                "aliasOf": "system_diagnostics",
                "required": ["principal"]
            },
            "file_list": {
                "required": ["path"]
            },
            "file_read": {
                "required": ["path"]
            },
            "file_write": {
                "required": ["path", "content"],
                "riskFloor": "high"
            },
            "delete_file": {
                "required": ["path"],
                "riskFloor": "high"
            },
            "codebase_patch": {
                "required": ["target_file_path", "search_pattern", "replacement_content"],
                "riskFloor": "high"
            },
            "codebase_compile": {
                "required": ["target"],
                "riskFloor": "high"
            },
            "terminal_execute": {
                "required": ["executable"],
                "optional": ["args", "env", "cwd", "timeout"]
            },
            "system_audit": {
                "required": ["scope"]
            },
            "telemetry_archive": {
                "required": ["output_path"],
                "riskFloor": "high"
            },
            "sync_knowledge_vault": {
                "required": ["path"],
                "optional": ["max_files", "mod_id"]
            },
            "connected_work": {
                "required": ["connector_ref", "capability", "arguments"]
            },
            "create_spreadsheet": {
                "requiredOneOf": [["workbook"], ["sourceProjection"]],
                "sourceProjectionExample": {
                    "fromStep": 0,
                    "collectionPointer": "/result/value",
                    "title": "Observed messages",
                    "locale": "en-US",
                    "sheetName": "Messages",
                    "columns": [{"header":"Subject","field":"subject"}]
                }
            },
            "app_control": {
                "required": ["phase"],
                "phases": ["start", "observe", "execute", "stop"]
            },
            "sovereign_duckduckgo_search": {
                "required": ["query"],
                "optional": ["max_results"]
            },
            "duckduckgo_search": {
                "aliasOf": "sovereign_duckduckgo_search",
                "required": ["query"],
                "optional": ["max_results"]
            },
            "unsupported": {
                "required": ["requested"]
            }
        }
    });
    if let Some(tools) = contract.get_mut("tools").and_then(Value::as_object_mut) {
        if let Some(schemas) = live_schema_payload.get("tools").and_then(Value::as_array) {
            for schema in schemas {
                let Some(kind) = schema.get("kind").and_then(Value::as_str) else {
                    continue;
                };
                if objective.is_some_and(|objective| {
                    !static_tool_matches_objective(kind, objective)
                        && !registered_task_tool_matches_objective(kind, objective)
                }) {
                    continue;
                }
                let input_schema = schema
                    .get("inputSchema")
                    .map(compact_planner_schema)
                    .unwrap_or_else(|| json!({"type":"object","not":{}}));
                let entry = tools.entry(kind.to_string()).or_insert_with(|| json!({}));
                if let Some(entry) = entry.as_object_mut() {
                    entry
                        .entry("inputSchema".to_string())
                        .or_insert(input_schema);
                }
            }
        }
        if let Some(objective) = objective {
            tools.retain(|kind, _| static_tool_matches_objective(kind, objective));
        }
        for operation in crate::tools::task_tool_runtime::registered_operations() {
            if objective.is_some_and(|objective| {
                !registered_task_tool_matches_objective(operation, objective)
            }) {
                continue;
            }
            let schema = crate::tools::task_tool_runtime::schema(operation)
                .unwrap_or_else(|_| json!({"type":"object","not":{}}));
            let planner_schema = json!({"inputSchema": compact_planner_schema(&schema)});
            if objective.is_some() {
                tools.insert(operation.to_string(), planner_schema);
            } else {
                tools.entry(operation.to_string()).or_insert(planner_schema);
            }
        }
    }
    contract
}

fn compact_planner_schema(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !PLANNER_SCHEMA_OMITTED_ANNOTATION_KEYS.contains(&key.as_str()))
                .map(|(key, nested)| (key.clone(), compact_planner_schema(nested)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(compact_planner_schema).collect()),
        _ => value.clone(),
    }
}

pub fn local_gemma_action_tool_required_fields(kind: &str) -> Option<&'static [&'static str]> {
    match normalize_tool_name(kind).as_str() {
        "system_diagnostics" | "get_system_metrics" => Some(&["principal"]),
        "file_read" | "file_list" => Some(&["path"]),
        "file_write" => Some(&["path", "content"]),
        "delete_file" => Some(&["path"]),
        "codebase_patch" => Some(&["target_file_path", "search_pattern", "replacement_content"]),
        "codebase_compile" => Some(&["target"]),
        "terminal_execute" => Some(&["executable"]),
        "system_audit" => Some(&["scope"]),
        "telemetry_archive" => Some(&["output_path"]),
        "sync_knowledge_vault" => Some(&["path"]),
        "connected_work" => Some(&["connector_ref", "capability", "arguments"]),
        "create_spreadsheet" => Some(&[]),
        "app_control" => Some(&["phase"]),
        "sovereign_duckduckgo_search" | "duckduckgo_search" => Some(&["query"]),
        "unsupported" => Some(&["requested"]),
        _ => None,
    }
}

fn validate_registered_task_tool(
    operation: &'static str,
    input: &Value,
) -> Result<ParsedParameters, String> {
    use crate::tools::task_tool_runtime::TaskToolRiskTier;
    let request = crate::tools::task_tool_runtime::validate(operation, input.clone())?;
    let tier = if request.potentially_effectful() {
        CapabilityRiskTier::FileWrite
    } else {
        match crate::tools::task_tool_runtime::risk_tier(operation)? {
            TaskToolRiskTier::ReadOnly => CapabilityRiskTier::ReadOnly,
            TaskToolRiskTier::FileRead => CapabilityRiskTier::FileRead,
            TaskToolRiskTier::FileWrite => CapabilityRiskTier::FileWrite,
            TaskToolRiskTier::SystemExec => CapabilityRiskTier::SystemExec,
            TaskToolRiskTier::Network => CapabilityRiskTier::Network,
        }
    };
    Ok(parsed(
        operation,
        request.arguments,
        CapabilityClassification::new(
            tier,
            "registered Task tool remains bound to Project policy, Task evidence, and Shield approval",
        ),
    ))
}

fn validate_file_path_tool(
    tool_name: &str,
    input: &Value,
    tier: CapabilityRiskTier,
    reason: &str,
) -> Result<ParsedParameters, String> {
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &["path"])?;
    let path = required_non_empty_string(tool_name, object, "path")?;
    Ok(parsed(
        tool_name,
        json!({ "path": path }),
        CapabilityClassification::new(tier, reason),
    ))
}

fn validate_file_write_tool(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::FileWrite.name();
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &["path", "content"])?;
    let path = required_non_empty_string(tool_name, object, "path")?;
    let content = required_string(tool_name, object, "content")?;
    Ok(parsed(
        tool_name,
        json!({ "path": path, "content": content }),
        CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "file_write modifies sandbox file contents",
        ),
    ))
}

fn validate_file_delete_tool(input: &Value) -> Result<ParsedParameters, String> {
    validate_file_path_tool(
        RegisteredNativeTool::FileDelete.name(),
        input,
        CapabilityRiskTier::FileWrite,
        "delete_file removes sandbox file contents after approval",
    )
}

fn validate_codebase_patch_tool(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::CodebasePatch.name();
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(
        tool_name,
        object,
        &["target_file_path", "search_pattern", "replacement_content"],
    )?;
    let target_file_path = required_non_empty_string(tool_name, object, "target_file_path")?;
    let search_pattern = required_non_empty_string(tool_name, object, "search_pattern")?;
    let replacement_content = required_string(tool_name, object, "replacement_content")?;
    Ok(parsed(
        tool_name,
        json!({
            "target_file_path": target_file_path,
            "search_pattern": search_pattern,
            "replacement_content": replacement_content
        }),
        CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "codebase_patch modifies repository source files",
        ),
    ))
}

fn validate_codebase_compile_tool(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::CodebaseCompile.name();
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &["target"])?;
    let target = required_non_empty_string(tool_name, object, "target")?;
    let target = normalize_tool_name(&target);
    if !matches!(target.as_str(), "backend" | "frontend") {
        return Err(format!(
            "{tool_name}.target must be either backend or frontend."
        ));
    }
    Ok(parsed(
        tool_name,
        json!({ "target": target }),
        CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "codebase_compile executes local compiler toolchains",
        ),
    ))
}

fn validate_telemetry_archive_tool(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::TelemetryArchive.name();
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &["output_path"])?;
    let output_path = required_non_empty_string(tool_name, object, "output_path")?;
    Ok(parsed(
        tool_name,
        json!({ "output_path": output_path }),
        CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "telemetry_archive writes a compressed diagnostics archive",
        ),
    ))
}

fn validate_single_string_tool(
    tool_name: &str,
    input: &Value,
    field: &str,
    tier: CapabilityRiskTier,
    reason: &str,
) -> Result<ParsedParameters, String> {
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &[field])?;
    let value = required_non_empty_string(tool_name, object, field)?;
    Ok(parsed(
        tool_name,
        json!({ field: value }),
        CapabilityClassification::new(tier, reason),
    ))
}

fn validate_terminal_execute(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::TerminalExecute.name();
    let request = serde_json::from_value::<crate::tools::terminal_contract::NativeTerminalRequest>(
        input.clone(),
    )
    .map_err(|error| format!("{tool_name} arguments are invalid: {error}"))?
    .validate()?;
    request.validate_protected_deletion_roots(&[
        crate::settings::app_data_root(),
        crate::shield_gate::development_repo_root(),
    ])?;
    let classification = request.classification();
    let arguments = serde_json::to_value(request)
        .map_err(|error| format!("{tool_name} arguments could not be normalized: {error}"))?;
    Ok(parsed(tool_name, arguments, classification))
}

fn validate_sync_knowledge_vault_tool(input: &Value) -> Result<ParsedParameters, String> {
    let tool_name = RegisteredNativeTool::SyncKnowledgeVault.name();
    let object = require_object(tool_name, input)?;
    reject_unknown_fields(tool_name, object, &["path", "max_files", "mod_id"])?;
    let path = required_non_empty_string(tool_name, object, "path")?;
    let max_files = optional_positive_usize(tool_name, object, "max_files")?;
    let mod_id = optional_non_empty_string(object, "mod_id");
    let mut arguments = json!({ "path": path });
    if let (Some(max_files), Some(arguments)) = (max_files, arguments.as_object_mut()) {
        arguments.insert(
            "max_files".to_string(),
            Value::Number(serde_json::Number::from(max_files as u64)),
        );
    }
    if let (Some(mod_id), Some(arguments)) = (mod_id, arguments.as_object_mut()) {
        arguments.insert("mod_id".to_string(), Value::String(mod_id));
    }
    Ok(parsed(
        tool_name,
        arguments,
        CapabilityClassification::new(
            CapabilityRiskTier::FileRead,
            "sync_knowledge_vault reads local vault files for differential knowledge indexing",
        ),
    ))
}

fn parsed(
    tool_name: &str,
    arguments: Value,
    classification: CapabilityClassification,
) -> ParsedParameters {
    ParsedParameters {
        tool_name: tool_name.to_string(),
        arguments,
        classification,
    }
}

fn require_object<'a>(tool_name: &str, input: &'a Value) -> Result<&'a Map<String, Value>, String> {
    input
        .as_object()
        .ok_or_else(|| format!("{tool_name} arguments must be a JSON object."))
}

fn reject_unknown_fields(
    tool_name: &str,
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = object
        .keys()
        .find(|key| !allowed.iter().any(|allowed| key.as_str() == *allowed))
    {
        return Err(format!("{tool_name} received unsupported field '{key}'."));
    }
    Ok(())
}

fn required_non_empty_string(
    tool_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, String> {
    let value = required_string(tool_name, object, field)?;
    if value.trim().is_empty() {
        return Err(format!("{tool_name}.{field} must not be empty."));
    }
    Ok(value)
}

fn required_string(
    tool_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("{tool_name}.{field} must be a string."))
}

fn optional_non_empty_string(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn optional_positive_usize(
    tool_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let Some(raw) = value.as_u64() else {
        return Err(format!("{tool_name}.{field} must be a positive integer."));
    };
    if raw == 0 {
        return Err(format!("{tool_name}.{field} must be greater than zero."));
    }
    usize::try_from(raw)
        .map(Some)
        .map_err(|_| format!("{tool_name}.{field} is too large."))
}

fn object_schema(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let properties = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<Map<_, _>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(object) => {
            let mut converted = Map::new();
            for (key, value) in object {
                if matches!(key.as_str(), "additionalProperties" | "default") {
                    continue;
                }
                let value = if key == "type" {
                    value
                        .as_str()
                        .map(|value| Value::String(value.to_ascii_uppercase()))
                        .unwrap_or_else(|| gemini_schema(value))
                } else {
                    gemini_schema(value)
                };
                converted.insert(key.clone(), value);
            }
            Value::Object(converted)
        }
        Value::Array(values) => Value::Array(values.iter().map(gemini_schema).collect()),
        value => value.clone(),
    }
}

fn normalize_tool_name(value: &str) -> String {
    match value.trim().replace('-', "_").to_ascii_lowercase().as_str() {
        "get_system_metrics" => "system_diagnostics".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_security::classify_mcp_tool_call;

    #[test]
    fn serializes_gemini_and_gemma_schema_payloads() {
        let registry = NativeToolRegistry::default();
        let gemini = registry.schema_payload(ModelProvider::Gemini);
        let gemma = registry.schema_payload(ModelProvider::LocalGemmaIt);
        let frontier = registry.schema_payload(ModelProvider::FrontierCloud);

        let gemini_round_trip = serde_json::from_str::<Value>(
            &serde_json::to_string(&gemini).expect("gemini schema serializes"),
        )
        .expect("gemini schema remains json");
        let gemma_round_trip = serde_json::from_str::<Value>(
            &serde_json::to_string(&gemma).expect("gemma schema serializes"),
        )
        .expect("gemma schema remains json");

        assert!(gemini_round_trip
            .pointer("/tools/0/functionDeclarations")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == registry.tools().len()));
        assert!(gemma_round_trip
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == registry.tools().len()));
        assert!(frontier
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == registry.tools().len()));
        assert_eq!(
            gemini_round_trip["tools"][0]["functionDeclarations"][0]["parameters"]["type"],
            json!("OBJECT")
        );
    }

    #[test]
    fn validates_terminal_calls_with_security_classification() {
        let registry = NativeToolRegistry::default();
        let parsed = registry
            .validate_call(
                "terminal_execute",
                &json!({
                    "executable": "git",
                    "args": ["branch", "-D", "testing"]
                }),
            )
            .expect("terminal call validates");

        assert_eq!(parsed.classification.tier, CapabilityRiskTier::FileWrite);

        let parsed = registry
            .validate_call(
                "terminal_execute",
                &json!({
                    "executable": "find",
                    "args": [".", "-fprintf", "matches.txt", "%p"]
                }),
            )
            .expect("terminal call validates");
        assert_eq!(parsed.classification.tier, CapabilityRiskTier::FileWrite);
    }

    #[test]
    fn rejects_unknown_or_incomplete_parameters() {
        let registry = NativeToolRegistry::default();
        assert!(registry
            .validate_call("file_read", &json!({ "path": "README.md", "extra": true }))
            .is_err());
        assert!(registry
            .validate_call("file_write", &json!({ "path": "README.md" }))
            .is_err());
        assert!(registry
            .validate_call(
                "codebase_patch",
                &json!({
                    "target_file_path": "src/app/page.tsx",
                    "search_pattern": "old",
                    "replacement_content": "new",
                    "extra": true
                })
            )
            .is_err());
        assert!(registry
            .validate_call(
                "codebase_compile",
                &json!({ "target": "backend", "extra": true })
            )
            .is_err());
        assert!(registry
            .validate_call("codebase_compile", &json!({ "target": "database" }))
            .is_err());
    }

    #[test]
    fn gemma_action_contract_exposes_required_fields() {
        let contract = local_gemma_action_plan_contract();
        assert_eq!(
            contract.pointer("/actionPlanSchema/toolRequired"),
            Some(&json!(["kind"]))
        );
        assert_eq!(
            contract.pointer("/actionPlanSchema/toolEncoding"),
            Some(&json!("flat"))
        );
        assert!(contract
            .pointer("/tools/file_write/riskFloor")
            .is_some_and(|value| value == "high"));
        assert!(contract
            .pointer("/tools/delete_file/riskFloor")
            .is_some_and(|value| value == "high"));
        assert!(contract
            .pointer("/tools/codebase_patch/riskFloor")
            .is_some_and(|value| value == "high"));
        assert!(contract
            .pointer("/tools/codebase_compile/riskFloor")
            .is_some_and(|value| value == "high"));
        assert_eq!(
            local_gemma_action_tool_required_fields("file-write"),
            Some(&["path", "content"][..])
        );
        assert_eq!(
            local_gemma_action_tool_required_fields("delete_file"),
            Some(&["path"][..])
        );
        assert_eq!(
            local_gemma_action_tool_required_fields("codebase_patch"),
            Some(&["target_file_path", "search_pattern", "replacement_content"][..])
        );
        assert_eq!(
            local_gemma_action_tool_required_fields("codebase_compile"),
            Some(&["target"][..])
        );
    }

    #[test]
    fn registered_task_tool_scope_is_semantic_and_fail_closed() {
        let compile = "Compile the frontend and report the verified result.";
        assert!(!registered_task_tool_matches_objective(
            "create_decision_pack",
            compile
        ));
        assert!(!registered_task_tool_matches_objective(
            "create_file",
            compile
        ));

        for extension in ["txt", "json", "csv", "xlsx", "pdf"] {
            let input_only = format!(
                "Read /tmp/create_file_input.{extension} and summarize only the stated facts."
            );
            assert!(registered_task_tool_matches_objective(
                "read_project_file",
                &input_only
            ));
            for operation in ["create_file", "create_spreadsheet", "create_presentation"] {
                assert!(
                    !registered_task_tool_matches_objective(operation, &input_only),
                    "input-only .{extension} reference exposed {operation}"
                );
            }
            for kind in [
                "file_write",
                "delete_file",
                "codebase_patch",
                "codebase_compile",
                "telemetry_archive",
            ] {
                assert!(
                    !static_tool_matches_objective(kind, &input_only),
                    "input-only .{extension} reference exposed {kind}"
                );
            }
        }

        let explicit_write = "Write the summary to /tmp/output.txt.";
        assert!(registered_task_tool_matches_objective(
            "create_file",
            explicit_write
        ));
        assert!(static_tool_matches_objective("file_write", explicit_write));

        let explicit_delete = "Delete /tmp/obsolete.txt.";
        assert!(static_tool_matches_objective(
            "delete_file",
            explicit_delete
        ));
        assert!(!registered_task_tool_matches_objective(
            "create_file",
            explicit_delete
        ));

        let explicit_spreadsheet = "Create a spreadsheet at /tmp/report.xlsx.";
        assert!(registered_task_tool_matches_objective(
            "create_file",
            explicit_spreadsheet
        ));
        assert!(registered_task_tool_matches_objective(
            "create_spreadsheet",
            explicit_spreadsheet
        ));
        assert!(!static_tool_matches_objective(
            "file_write",
            explicit_spreadsheet
        ));

        for connected_objective in [
            "Use my configured MCP server to list the approved records.",
            "Use Apple Apps to read my selected calendar.",
        ] {
            assert!(registered_task_tool_matches_objective(
                "connected_work",
                connected_objective
            ));
        }
        assert!(!registered_task_tool_matches_objective(
            "connected_work",
            "Compare Slack and Teams pricing without accessing either service."
        ));

        let recipient_send = "Email recipient@example.com the verified report.";
        assert!(registered_task_tool_matches_objective(
            "send_system_email",
            recipient_send
        ));
        assert!(!registered_task_tool_matches_objective(
            "draft_system_email",
            recipient_send
        ));

        let event_without_calendar = "Create an event titled Review tomorrow at 2 PM.";
        assert!(registered_task_tool_matches_objective(
            "create_system_calendar_event",
            event_without_calendar
        ));

        let scheduled = "Read supplier_proposals.json and project_milestones.json, reconcile supplier rate variances and unfinished milestone risks, retrieve two primary or official public web sources, then create operations_brief_<YYYY-MM-DD_HH-mm>.md and a matching PDF.";
        for operation in [
            "read_project_file",
            "fetch_official_page",
            "analyze_supplier_exceptions",
            "analyze_project_milestones",
            "create_file",
        ] {
            assert!(registered_task_tool_matches_objective(operation, scheduled));
        }
        assert!(!registered_task_tool_matches_objective(
            "create_decision_pack",
            scheduled
        ));

        let comparison = concat!(
            "Research current official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork, then write the background agent comparison to /tmp/background_agent_comparison.md and read it back. ",
            "Also create a separate event titled Comparison Review tomorrow at 2 PM; ",
            "draft a separate email to review@example.com; use my configured MCP server; ",
            "also create a separate presentation at /tmp/comparison_brief.pptx."
        );
        for operation in [
            "prepare_background_agent_comparison",
            "create_system_calendar_event",
            "draft_system_email",
            "connected_work",
            "create_presentation",
        ] {
            assert!(
                registered_task_tool_matches_objective(operation, comparison),
                "comparison objective omitted {operation}"
            );
        }
        assert!(!registered_task_tool_matches_objective(
            "fetch_official_page",
            comparison
        ));

        let recovery = concat!(
            "Read /tmp/project_milestones.json and construct a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the assumptions, critical path, and three failure contingencies to /tmp/recovery_plan.md and verify the file. ",
            "Also create a separate spreadsheet at /tmp/recovery_status.xlsx; ",
            "also create an event titled Recovery Check tomorrow at 3 PM; ",
            "configure the Slack channel."
        );
        for operation in [
            "prepare_milestone_constraint_recovery_plan",
            "create_spreadsheet",
            "create_system_calendar_event",
            "configure_channel",
        ] {
            assert!(
                registered_task_tool_matches_objective(operation, recovery),
                "milestone recovery objective omitted {operation}"
            );
        }
        assert!(!registered_task_tool_matches_objective(
            "read_project_file",
            recovery
        ));

        let release = concat!(
            "Review every overdue or unfinished milestone, prepare a recovery meeting with exactly five agenda items, ",
            "and create an unsent Mail draft. Also create a separate spreadsheet at /tmp/release_status.xlsx; ",
            "email audit@example.com the verified status; configure the Slack channel."
        );
        for operation in [
            "prepare_release_recovery_agenda",
            "create_release_recovery_calendar_event",
            "draft_release_recovery_email",
            "create_spreadsheet",
            "send_system_email",
            "configure_channel",
        ] {
            assert!(
                registered_task_tool_matches_objective(operation, release),
                "release recovery objective omitted {operation}"
            );
        }

        let decision_pack = concat!(
            "Prepare a supplier decision pack with supplier_decision.xlsx, supplier_decision.pptx, ",
            "supplier_decision.pdf, and sources.md. Create a conflict-free event titled Supplier Decision Review ",
            "and an unsent Mail draft to owner@example.com. Email audit@example.com a separate status update; ",
            "also create a separate event titled Executive Follow-up tomorrow at 3 PM; ",
            "also create a separate presentation at /tmp/board_appendix.pptx."
        );
        for operation in [
            "create_decision_pack",
            "create_conflict_free_calendar_event",
            "draft_decision_pack_email",
            "send_system_email",
            "create_system_calendar_event",
            "create_presentation",
        ] {
            assert!(
                registered_task_tool_matches_objective(operation, decision_pack),
                "decision-pack objective omitted {operation}"
            );
        }
        assert!(!registered_task_tool_matches_objective(
            "draft_system_email",
            decision_pack
        ));

        let unknown = "Perform the requested bounded task.";
        assert!(!registered_task_tool_matches_objective(
            "future_unrelated_effect",
            unknown
        ));
        assert!(registered_task_tool_matches_objective(
            "future_unrelated_effect",
            "Use future_unrelated_effect for this exact task."
        ));
    }

    #[test]
    fn compact_planner_schema_preserves_every_runtime_validation_constraint() {
        let schema = json!({
            "type": "object",
            "description": "Prose is intentionally omitted from the planner envelope.",
            "properties": {
                "label": {
                    "type": "string",
                    "minLength": 2,
                    "maxLength": 12,
                    "pattern": "^[a-z]+$",
                    "description": "Planner-only prose"
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5
                },
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 3,
                    "uniqueItems": true,
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "x-newValidationConstraint": { "limit": 3 }
                    }
                }
            },
            "required": ["label", "count", "items"],
            "additionalProperties": false
        });

        let compact = compact_planner_schema(&schema);

        assert!(compact.get("description").is_none());
        assert!(compact.pointer("/properties/label/description").is_none());
        assert_eq!(
            compact.pointer("/properties/label/minLength"),
            Some(&json!(2))
        );
        assert_eq!(
            compact.pointer("/properties/label/maxLength"),
            Some(&json!(12))
        );
        assert_eq!(
            compact.pointer("/properties/label/pattern"),
            Some(&json!("^[a-z]+$"))
        );
        assert_eq!(
            compact.pointer("/properties/count/minimum"),
            Some(&json!(1))
        );
        assert_eq!(
            compact.pointer("/properties/count/maximum"),
            Some(&json!(5))
        );
        assert_eq!(
            compact.pointer("/properties/items/minItems"),
            Some(&json!(1))
        );
        assert_eq!(
            compact.pointer("/properties/items/maxItems"),
            Some(&json!(3))
        );
        assert_eq!(
            compact.pointer("/properties/items/uniqueItems"),
            Some(&json!(true))
        );
        assert_eq!(
            compact.pointer("/properties/items/items/minLength"),
            Some(&json!(1))
        );
        assert_eq!(
            compact.pointer("/properties/items/items/x-newValidationConstraint/limit"),
            Some(&json!(3))
        );
    }

    #[test]
    fn filesystem_classifications_are_stable() {
        let registry = NativeToolRegistry::default();
        let read = registry
            .validate_call("file_read", &json!({ "path": "notes.txt" }))
            .expect("file_read validates");
        assert_eq!(read.classification.tier, CapabilityRiskTier::FileRead);

        let write = registry
            .validate_call(
                "file_write",
                &json!({ "path": "notes.txt", "content": "ok" }),
            )
            .expect("file_write validates");
        assert_eq!(write.classification.tier, CapabilityRiskTier::FileWrite);

        let delete = registry
            .validate_call("delete_file", &json!({ "path": "notes.txt" }))
            .expect("delete_file validates");
        assert_eq!(delete.classification.tier, CapabilityRiskTier::FileWrite);

        let patch = registry
            .validate_call(
                "codebase_patch",
                &json!({
                    "target_file_path": "src/app/page.tsx",
                    "search_pattern": "export default",
                    "replacement_content": "export default"
                }),
            )
            .expect("codebase_patch validates");
        assert_eq!(patch.classification.tier, CapabilityRiskTier::FileWrite);
        assert_eq!(
            patch.arguments["target_file_path"],
            json!("src/app/page.tsx")
        );

        let compile = registry
            .validate_call("codebase_compile", &json!({ "target": "frontend" }))
            .expect("codebase_compile validates");
        assert_eq!(compile.classification.tier, CapabilityRiskTier::SystemExec);
        assert_eq!(compile.arguments["target"], json!("frontend"));

        let mcp = classify_mcp_tool_call("local_filesystem", "write_file", None);
        assert_eq!(mcp.tier, write.classification.tier);
    }

    #[test]
    fn app_control_registry_is_reachable_and_rejects_unbounded_actions() {
        crate::tools::task_tool_runtime::register_app_control_test_fixture();
        let registry = NativeToolRegistry::default();
        let parsed = registry
            .validate_call("app_control", &json!({"phase":"start"}))
            .expect("registered app control phase validates");
        assert_eq!(parsed.classification.tier, CapabilityRiskTier::SystemExec);
        for phase in ["observe", "execute", "stop"] {
            assert!(registry
                .validate_call("app_control", &json!({"phase":phase}))
                .is_ok());
        }
        assert!(registry
            .validate_call(
                "app_control",
                &json!({
                    "phase":"execute",
                    "action":{"kind":"run_script","script":"unsafe"}
                }),
            )
            .is_err());
        assert!(registry
            .validate_call(
                "app_control",
                &json!({"phase":"execute","action":{"kind":"press","x":1,"y":2}}),
            )
            .is_err());
    }
}
