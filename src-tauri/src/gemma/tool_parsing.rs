use super::*;

fn generated_tool_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn missing_tool_input(kind: &str, fields: &str) -> Option<GeneratedToolDraft> {
    Some(GeneratedToolDraft::Unsupported {
        requested: format!(
            "Clarification required: {kind} needs explicit {fields} from the current request or picker grants."
        ),
    })
}

pub(super) fn parse_generated_tool(value: &Value) -> Option<GeneratedToolDraft> {
    let kind = value.get("kind")?.as_str()?;
    if let Some(arguments) =
        crate::tools::task_tool_runtime::validated_generated_arguments(kind, value)
    {
        return Some(GeneratedToolDraft::RegisteredTaskTool {
            operation: kind.trim().replace('-', "_").to_ascii_lowercase(),
            arguments,
        });
    }
    match kind {
        "system_diagnostics" | "get_system_metrics" => {
            Some(GeneratedToolDraft::SystemDiagnostics {
                principal: value
                    .get("principal")
                    .and_then(Value::as_str)
                    .unwrap_or("local_principal")
                    .to_string(),
            })
        }
        "file_read" => generated_tool_text(value, &["path"])
            .map(|path| GeneratedToolDraft::FileRead { path })
            .or_else(|| missing_tool_input("file_read", "source path")),
        "file_write" => parse_file_write(value),
        "delete_file" | "trash" | "trash_file" => generated_tool_text(value, &["path"])
            .map(|path| GeneratedToolDraft::DeleteFile { path })
            .or_else(|| missing_tool_input(kind, "target path")),
        "codebase_patch" => parse_codebase_patch(value),
        "codebase_compile" => {
            generated_tool_text(value, &["target", "compile_target", "compileTarget"])
                .map(|target| GeneratedToolDraft::CodebaseCompile { target })
                .or_else(|| missing_tool_input("codebase_compile", "compile target"))
        }
        "terminal_execute" => terminal_tool::parse_terminal_generated_tool(value),
        remaining => parse_context_tool(remaining, value),
    }
}

fn parse_file_write(value: &Value) -> Option<GeneratedToolDraft> {
    let Some(path) = generated_tool_text(value, &["path"]) else {
        return missing_tool_input("file_write", "destination path and content");
    };
    let Some(content) = value
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return missing_tool_input("file_write", "destination path and content");
    };
    Some(GeneratedToolDraft::FileWrite { path, content })
}

fn parse_codebase_patch(value: &Value) -> Option<GeneratedToolDraft> {
    let fields = "target path, search pattern, and replacement content";
    let Some(target_file_path) =
        generated_tool_text(value, &["target_file_path", "targetFilePath"])
    else {
        return missing_tool_input("codebase_patch", fields);
    };
    let Some(search_pattern) = generated_tool_text(value, &["search_pattern", "searchPattern"])
    else {
        return missing_tool_input("codebase_patch", fields);
    };
    let Some(replacement_content) =
        generated_tool_text(value, &["replacement_content", "replacementContent"])
    else {
        return missing_tool_input("codebase_patch", fields);
    };
    Some(GeneratedToolDraft::CodebasePatch {
        target_file_path,
        search_pattern,
        replacement_content,
    })
}

fn parse_context_tool(kind: &str, value: &Value) -> Option<GeneratedToolDraft> {
    match kind {
        "file_list" => generated_tool_text(value, &["path"])
            .map(|path| GeneratedToolDraft::FileList { path })
            .or_else(|| missing_tool_input("file_list", "directory path")),
        "system_audit" => Some(GeneratedToolDraft::SystemAudit {
            scope: value
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("process_disk_network")
                .to_string(),
        }),
        "telemetry_archive" => generated_tool_text(value, &["output_path", "outputPath", "path"])
            .map(|output_path| GeneratedToolDraft::TelemetryArchive { output_path })
            .or_else(|| missing_tool_input("telemetry_archive", "archive destination path")),
        "web_fetch" => generated_tool_text(value, &["url", "path"])
            .map(|url| GeneratedToolDraft::WebFetch {
                url,
                extraction_hint: generated_tool_text(value, &["extraction_hint"]),
            })
            .or_else(|| missing_tool_input("web_fetch", "URL")),
        "document_index" => Some(GeneratedToolDraft::DocumentIndex {
            workspace: value
                .get("workspace")
                .or_else(|| value.get("path"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
        }),
        "ask_local_document_index" => generated_tool_text(value, &["question"])
            .map(|question| GeneratedToolDraft::AskLocalDocumentIndex { question })
            .or_else(|| missing_tool_input("ask_local_document_index", "question")),
        "sovereign_duckduckgo_search" | "duckduckgo_search" => parse_search_tool(value),
        "unsupported" => Some(GeneratedToolDraft::Unsupported {
            requested: value
                .get("requested")
                .and_then(Value::as_str)
                .unwrap_or("unsupported")
                .to_string(),
        }),
        requested => Some(GeneratedToolDraft::Unsupported {
            requested: requested.to_string(),
        }),
    }
}

fn parse_search_tool(value: &Value) -> Option<GeneratedToolDraft> {
    let Some(query) = generated_tool_text(value, &["query", "principal", "path"]) else {
        return missing_tool_input("sovereign_duckduckgo_search", "search query");
    };
    Some(GeneratedToolDraft::SovereignDuckDuckGoSearch {
        query,
        max_results: value
            .get("max_results")
            .or_else(|| value.get("maxResults"))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str()?.parse::<u64>().ok())
            })
            .map(|value| (value as usize).clamp(1, 5)),
    })
}
