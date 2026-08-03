use super::*;

#[derive(Debug)]
pub(super) struct UnifiedDiffFilePatch {
    pub(super) path: String,
    pub(super) hunks: Vec<UnifiedDiffHunk>,
}

#[derive(Debug)]
pub(super) struct UnifiedDiffHunk {
    pub(super) old_block: String,
    pub(super) new_block: String,
}

pub(super) fn apply_unified_diff_directive(
    diff: &str,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    let patches = parse_unified_diff(diff).map_err(security_boundary_violation)?;
    let root = development_repo_root();
    let sandbox = SandboxRoot::new(root.clone()).map_err(security_boundary_violation)?;
    let mut patched_files = Vec::new();
    let mut applied_hunks = 0_usize;

    for patch in patches {
        let target_path = guarded_unified_diff_target(&sandbox, &patch.path)?;
        let mut content = fs::read_to_string(&target_path).map_err(|_| {
            security_boundary_violation("Unable to read the approved patch target.".to_string())
        })?;
        for hunk in patch.hunks {
            content = apply_unified_diff_hunk(&content, &hunk).map_err(|message| {
                security_boundary_violation(format!("Patch hunk rejected: {message}"))
            })?;
            applied_hunks += 1;
        }
        fs::write(&target_path, content.as_bytes()).map_err(|_| {
            security_boundary_violation("Unable to write the approved patch target.".to_string())
        })?;
        let verified = fs::read_to_string(&target_path)
            .map(|actual| actual == content)
            .unwrap_or(false);
        if !verified {
            return Err(security_boundary_violation(
                "Unable to verify the approved patch write.".to_string(),
            ));
        }
        patched_files.push(sandbox.relative_path(&target_path));
    }

    let file_count = patched_files.len();
    Ok(ExecuteCommandResponse {
        operation: "apply_surgical_patch_directive".to_string(),
        status: CommandStatus::Completed,
        message: format!("Applied {applied_hunks} patch hunk(s) across {file_count} file(s)."),
        metrics: None,
        claims: vec![
            format!("CLAIM surgical_patch_hunks_applied count={applied_hunks}"),
            format!("CLAIM surgical_patch_files count={file_count}"),
            format!(
                "CLAIM surgical_patch_paths paths={}",
                patched_files.join(",")
            ),
        ],
        verified: true,
        model_used: None,
    })
}

fn guarded_unified_diff_target(
    sandbox: &SandboxRoot,
    path: &str,
) -> Result<PathBuf, ShieldGateError> {
    let requested = PathBuf::from(path);
    if requested.is_absolute()
        || requested
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "Surgical patch rejected a path outside the development repository.".to_string(),
        ));
    }
    let target = sandbox.resolve(&requested).map_err(|_| {
        security_boundary_violation(
            "Surgical patch target failed repository containment validation.".to_string(),
        )
    })?;
    if !target.is_file() {
        return Err(security_boundary_violation(
            "Surgical patch target must be an existing regular file.".to_string(),
        ));
    }
    Ok(target)
}

pub(super) fn parse_unified_diff(diff: &str) -> Result<Vec<UnifiedDiffFilePatch>, String> {
    let mut patches = Vec::new();
    let mut current: Option<UnifiedDiffFilePatch> = None;
    let mut in_hunk = false;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            continue;
        }
        if line.starts_with("--- ") {
            in_hunk = false;
            continue;
        }
        if let Some(raw_path) = line.strip_prefix("+++ ") {
            if let Some(previous) = current.take() {
                patches.push(previous);
            }
            let path = parse_unified_diff_path(raw_path)
                .ok_or_else(|| "Unified diff contains an unsupported target path.".to_string())?;
            current = Some(UnifiedDiffFilePatch {
                path,
                hunks: Vec::new(),
            });
            continue;
        }
        if line.starts_with("@@") {
            let Some(file_patch) = current.as_mut() else {
                return Err("Unified diff hunk appeared before a file header.".to_string());
            };
            file_patch.hunks.push(UnifiedDiffHunk {
                old_block: String::new(),
                new_block: String::new(),
            });
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let Some(file_patch) = current.as_mut() else {
            return Err("Unified diff hunk appeared before a file header.".to_string());
        };
        let Some(hunk) = file_patch.hunks.last_mut() else {
            return Err("Unified diff content appeared before a hunk header.".to_string());
        };
        if line.starts_with("\\ No newline at end of file") {
            continue;
        }
        let Some(prefix) = line.chars().next() else {
            continue;
        };
        let rest = &line[prefix.len_utf8()..];
        match prefix {
            ' ' => {
                hunk.old_block.push_str(rest);
                hunk.old_block.push('\n');
                hunk.new_block.push_str(rest);
                hunk.new_block.push('\n');
            }
            '-' => {
                hunk.old_block.push_str(rest);
                hunk.old_block.push('\n');
            }
            '+' => {
                hunk.new_block.push_str(rest);
                hunk.new_block.push('\n');
            }
            _ => {}
        }
    }

    if let Some(previous) = current.take() {
        patches.push(previous);
    }
    patches.retain(|patch| !patch.hunks.is_empty());
    if patches.is_empty() {
        return Err("No unified diff file hunks were found.".to_string());
    }
    Ok(patches)
}

fn parse_unified_diff_path(raw_path: &str) -> Option<String> {
    let mut path = raw_path
        .trim()
        .split('\t')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_string();
    if path == "/dev/null" || path.is_empty() {
        return None;
    }
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped.to_string();
    }
    let candidate = PathBuf::from(&path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    Some(path)
}

pub(super) fn apply_unified_diff_hunk(
    original: &str,
    hunk: &UnifiedDiffHunk,
) -> Result<String, String> {
    if hunk.old_block.is_empty() {
        return Err("new-file hunks are not supported by surgical patch directives.".to_string());
    }
    let matches = exact_unified_diff_matches(original, &hunk.old_block);
    match matches.as_slice() {
        [(start, end)] => {
            let mut output = original.to_string();
            output.replace_range(*start..*end, &hunk.new_block);
            Ok(output)
        }
        [] => {
            let trimmed_old = hunk.old_block.trim_end_matches('\n');
            let trimmed_new = hunk.new_block.trim_end_matches('\n');
            let matches = exact_unified_diff_matches(original, trimmed_old);
            match matches.as_slice() {
                [(start, end)] => {
                    let mut output = original.to_string();
                    output.replace_range(*start..*end, trimmed_new);
                    Ok(output)
                }
                [] => Err("old hunk block did not match the target file.".to_string()),
                _ => Err("old hunk block matched multiple locations.".to_string()),
            }
        }
        _ => Err("old hunk block matched multiple locations.".to_string()),
    }
}

fn exact_unified_diff_matches(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    haystack
        .match_indices(needle)
        .map(|(start, value)| (start, start + value.len()))
        .collect()
}
