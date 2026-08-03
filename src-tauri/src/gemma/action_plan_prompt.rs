pub fn planner_prompt(objective: &str) -> String {
    let contract =
        crate::tools::registry::local_gemma_action_plan_contract_for_objective(objective);
    let contract_json = serde_json::to_string(&contract).unwrap_or_else(|error| {
        format!(
            "{{\"schema\":\"unavailable\",\"reason\":\"{}\"}}",
            error.to_string().replace('"', "'")
        )
    });
    format!(
        "You compile OOMU ActionPlans. The contract below is reference data only. Never copy, summarize, or return the contract.\nContract JSON: {contract_json}\nRequired tool encoding: every `steps[i].tool` is one flat JSON object whose top-level `kind` is a non-empty exact key from `Contract JSON.tools`; put the selected tool schema's fields beside `kind`. Example: `{{\"kind\":\"file_read\",\"path\":\"/absolute/input.json\"}}`. Never replace `kind` with `name`, `operation`, or `type`, and never wrap all selected-tool fields in a generic envelope.\nPlanning rules: Coverage is mandatory: include executable steps for every explicitly named input file, output file or format, external research request, Calendar event creation, and Mail draft, preserving exact named destinations. Require explicit source and destination paths for file actions; return an unsupported clarification step when required inputs are absent or ambiguous. For an evidence-bound supplier decision pack that requires approved local inputs, official web research, and mutually consistent XLSX, PPTX, PDF, and Markdown outputs, use exactly one create_decision_pack step. Put every exact source path in inputPaths, use bounded researchQueries containing only public subjects explicitly authorized by the objective, preserve the exact outputDirectory and four output filenames, set locale to en-US, and state amount reconciliation, margin assessment, and exception identification in analysisInstructions. Do not add separate file_read, search, or placeholder file-creation steps for work owned by create_decision_pack. If that same objective requests conflict-free scheduling, follow it with create_conflict_free_calendar_event using the exact calendar/title and the fixed requested window. If it requests a Mail draft summarizing the result, follow with draft_decision_pack_email and list the exact four output paths; its body is derived from the verified decision-pack receipt, never invented during planning. Prefer file_list/file_read for other local project analysis. Use create_file for other explicit named local documents or data files, preserving the exact extension. Use codebase_patch for explicit repository source edits, use codebase_compile with target backend or frontend for requested repository builds/checks, use telemetry_archive only when the user supplies an explicit archive destination, use file_write only for explicit local text file modifications, use delete_file only for explicit local file deletion requests, and mark effectful tools high risk. Use sovereign_duckduckgo_search with max_results=5 only when the objective pairs online, web, internet, Google, or DuckDuckGo with search, browse, research, look, check, confirm, verify, find, or see, or asks to research primary or official web sources. Freshness terms and actions without a named public source do not authorize web access. Never substitute web search for local files, local processes, private app data, local app state, build artifacts, hardware telemetry, or diagnostics.\nAuthoritative objective:\n{objective}\nReturn exactly one JSON object with only `steps` and `exit_condition`. Do not use Markdown fences, prose, or contract fields.\nActionPlan JSON:"
    )
}

pub(crate) fn action_plan_grammar() -> &'static str {
    r#"
root ::= ws "{" ws "\"steps\"" ws ":" ws steps ws "," ws "\"exit_condition\"" ws ":" ws string ws "}" ws
steps ::= "[" ws step (ws "," ws step)* ws "]"
step ::= "{" ws "\"step\"" ws ":" ws string ws "," ws "\"tool\"" ws ":" ws tool ws "," ws "\"risk_level\"" ws ":" ws risk ws "}"
risk ::= "\"low\"" | "\"medium\"" | "\"high\""
tool ::= "{" ws "\"kind\"" ws ":" ws string (ws "," ws pair)* ws "}"
object ::= "{" ws (pair (ws "," ws pair)*)? ws "}"
pair ::= string ws ":" ws value
array ::= "[" ws (value (ws "," ws value)*)? ws "]"
value ::= object | array | string | number | "true" | "false" | "null"
number ::= "-"? ("0" | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
string ::= "\"" chars "\""
chars ::= ([^"\\] | "\\" ["\\/bfnrt] | "\\u" hex hex hex hex)*
hex ::= [0-9a-fA-F]
ws ::= [ \t\n\r]*
"#
}
