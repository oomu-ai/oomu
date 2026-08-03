use super::{ToolError, ToolOutput};
use crate::security::sandbox::SandboxRoot;
use crate::shield_gate::{
    resolve_diagnostic_read_path, CodebasePatchRequest, FileListRequest, FileReadRequest,
    FileWriteRequest,
};
use std::ops::Range;
use std::{fs, path::PathBuf};

const MAX_AGENT_FILE_READ_BYTES: u64 = 1024 * 1024;

pub struct FileSystemTools {
    root: PathBuf,
}

impl FileSystemTools {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn read(&self, request: FileReadRequest) -> Result<ToolOutput, ToolError> {
        let path = self.guard_path(&request.path, "file_read")?;
        let metadata = fs::metadata(&path).map_err(|error| ToolError {
            operation: "file_read".to_string(),
            message: format!("File read metadata failed for {}: {error}", path.display()),
        })?;
        if !metadata.is_file() {
            return Err(ToolError {
                operation: "file_read".to_string(),
                message: format!("File read target is not a regular file: {}", path.display()),
            });
        }
        if metadata.len() > MAX_AGENT_FILE_READ_BYTES {
            return Err(ToolError {
                operation: "file_read".to_string(),
                message: format!(
                    "File read refused {} because it is {} bytes; the native action limit is {} bytes.",
                    path.display(),
                    metadata.len(),
                    MAX_AGENT_FILE_READ_BYTES
                ),
            });
        }
        let content = fs::read_to_string(&path).map_err(|error| ToolError {
            operation: "file_read".to_string(),
            message: format!("File read failed for {}: {error}", path.display()),
        })?;
        let bytes = content.len();
        Ok(ToolOutput {
            operation: "file_read".to_string(),
            message: format!(
                "Read {bytes} byte(s) from {}.\n\n{}",
                path.display(),
                content
            ),
            claims: vec![format!(
                "CLAIM file_exists path={} min_bytes={bytes}",
                path.display()
            )],
        })
    }

    pub fn write(&self, request: FileWriteRequest) -> Result<ToolOutput, ToolError> {
        let path = self.guard_path(&request.path, "file_write")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ToolError {
                operation: "file_write".to_string(),
                message: format!("Failed to create {}: {error}", parent.display()),
            })?;
        }
        fs::write(&path, request.content.as_bytes()).map_err(|error| ToolError {
            operation: "file_write".to_string(),
            message: format!("File write failed for {}: {error}", path.display()),
        })?;
        let actual = fs::read(&path).map_err(|error| ToolError {
            operation: "file_write".to_string(),
            message: format!(
                "Unable to verify file write for {}: {error}",
                path.display()
            ),
        })?;
        if actual != request.content.as_bytes() {
            return Err(ToolError {
                operation: "file_write".to_string(),
                message: format!(
                    "Unable to verify file write for {}: final contents did not match the requested content.",
                    path.display()
                ),
            });
        }
        let bytes = request.content.len();
        Ok(ToolOutput {
            operation: "file_write".to_string(),
            message: format!("Wrote {bytes} byte(s) to {}.", path.display()),
            claims: vec![format!(
                "CLAIM file_exists path={} min_bytes={bytes}",
                path.display()
            )],
        })
    }

    pub fn codebase_patch(&self, request: CodebasePatchRequest) -> Result<ToolOutput, ToolError> {
        let path = self.guard_path(&request.target_file_path, "codebase_patch")?;
        let original = fs::read_to_string(&path).map_err(|error| ToolError {
            operation: "codebase_patch".to_string(),
            message: format!("Codebase patch read failed for {}: {error}", path.display()),
        })?;
        let patched = apply_surgical_patch(
            &original,
            &request.search_pattern,
            &request.replacement_content,
        )
        .map_err(|message| ToolError {
            operation: "codebase_patch".to_string(),
            message: format!("Codebase patch rejected for {}: {message}", path.display()),
        })?;

        fs::write(&path, patched.as_bytes()).map_err(|error| ToolError {
            operation: "codebase_patch".to_string(),
            message: format!(
                "Codebase patch write failed for {}: {error}",
                path.display()
            ),
        })?;
        let actual = fs::read(&path).map_err(|error| ToolError {
            operation: "codebase_patch".to_string(),
            message: format!(
                "Unable to verify codebase patch for {}: {error}",
                path.display()
            ),
        })?;
        if actual != patched.as_bytes() {
            return Err(ToolError {
                operation: "codebase_patch".to_string(),
                message: format!(
                    "Unable to verify codebase patch for {}: final contents did not match the requested patch.",
                    path.display()
                ),
            });
        }
        let bytes = patched.len();
        Ok(ToolOutput {
            operation: "codebase_patch".to_string(),
            message: format!(
                "Applied codebase patch to {} using a unique search match.",
                path.display()
            ),
            claims: vec![
                format!(
                    "CLAIM codebase_patch path={} replacements=1",
                    path.display()
                ),
                format!(
                    "CLAIM file_exists path={} min_bytes={bytes}",
                    path.display()
                ),
            ],
        })
    }

    pub fn list(&self, request: FileListRequest) -> Result<ToolOutput, ToolError> {
        let path = self.guard_path(&request.path, "file_list")?;
        let directory_entries = fs::read_dir(&path).map_err(|error| ToolError {
            operation: "file_list".to_string(),
            message: format!("File list failed for {}: {error}", path.display()),
        })?;
        let mut entries = Vec::new();
        for entry in directory_entries {
            let entry = entry.map_err(|error| ToolError {
                operation: "file_list".to_string(),
                message: format!(
                    "File list could not inspect an entry in {}: {error}",
                    path.display()
                ),
            })?;
            let file_type = entry.file_type().map_err(|error| ToolError {
                operation: "file_list".to_string(),
                message: format!(
                    "File list could not inspect the type of {}: {error}",
                    entry.path().display()
                ),
            })?;
            if file_type.is_symlink() {
                continue;
            }
            let mut name = entry.file_name().to_string_lossy().to_string();
            if file_type.is_dir() {
                name.push('/');
            }
            entries.push(name);
        }
        entries.sort();
        let entry_count = entries.len();
        let message = if entries.is_empty() {
            "(directory is empty)".to_string()
        } else {
            entries.join("\n")
        };
        Ok(ToolOutput {
            operation: "file_list".to_string(),
            message,
            claims: vec![
                format!("CLAIM dir_exists path={}", path.display()),
                format!(
                    "CLAIM directory_entries path={} count={entry_count}",
                    path.display()
                ),
            ],
        })
    }

    pub fn guard_path(&self, requested: &str, operation: &str) -> Result<PathBuf, ToolError> {
        if let Ok(path) = resolve_diagnostic_read_path(operation, requested) {
            return Ok(path);
        }

        let requested_path = PathBuf::from(requested);
        if requested_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(ToolError {
                operation: operation.to_string(),
                message: format!("{operation} rejected path traversal outside project quarantine"),
            });
        }
        let sandbox = SandboxRoot::new(self.root.clone()).map_err(|message| ToolError {
            operation: operation.to_string(),
            message: format!("{operation} rejected project quarantine root: {message}"),
        })?;
        sandbox
            .resolve(requested_path)
            .map_err(|message| ToolError {
                operation: operation.to_string(),
                message: format!("{operation} rejected path outside project quarantine: {message}"),
            })
    }
}

fn apply_surgical_patch(
    original: &str,
    search_pattern: &str,
    replacement_content: &str,
) -> Result<String, String> {
    if search_pattern.trim().is_empty() {
        return Err("search_pattern must not be empty.".to_string());
    }

    let exact_matches = exact_match_ranges(original, search_pattern);
    match exact_matches.as_slice() {
        [range] => {
            let mut output = original.to_string();
            output.replace_range(range.clone(), replacement_content);
            return Ok(output);
        }
        [] => {}
        _ => {
            return Err(format!(
                "search_pattern matched {} exact locations; refusing ambiguous patch.",
                exact_matches.len()
            ))
        }
    }

    let fuzzy_matches = whitespace_insensitive_match_ranges(original, search_pattern);
    match fuzzy_matches.as_slice() {
        [range] => {
            let mut output = original.to_string();
            let replacement =
                if original[range.clone()].contains('\n') || search_pattern.contains('\n') {
                    let line_range = expand_to_line_range(original, range.clone());
                    let base_indent = line_indent(original, line_range.start);
                    let replacement = reindent_replacement(
                        replacement_content,
                        base_indent,
                        original[line_range.clone()].ends_with('\n'),
                    );
                    output.replace_range(line_range, &replacement);
                    return Ok(output);
                } else {
                    replacement_content.to_string()
                };
            output.replace_range(range.clone(), &replacement);
            Ok(output)
        }
        [] => Err(
            "search_pattern did not match the target file, even after whitespace normalization."
                .to_string(),
        ),
        _ => Err(format!(
            "search_pattern matched {} normalized locations; refusing ambiguous patch.",
            fuzzy_matches.len()
        )),
    }
}

fn exact_match_ranges(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    haystack
        .match_indices(needle)
        .map(|(start, value)| start..start + value.len())
        .collect()
}

fn whitespace_insensitive_match_ranges(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    let needle_chars = needle
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<Vec<_>>();
    if needle_chars.is_empty() {
        return Vec::new();
    }

    let haystack_chars = haystack
        .char_indices()
        .filter_map(|(index, character)| {
            (!character.is_whitespace()).then_some((character, index, index + character.len_utf8()))
        })
        .collect::<Vec<_>>();
    if needle_chars.len() > haystack_chars.len() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    for index in 0..=haystack_chars.len() - needle_chars.len() {
        let matched = needle_chars
            .iter()
            .enumerate()
            .all(|(offset, expected)| haystack_chars[index + offset].0 == *expected);
        if matched {
            let start = haystack_chars[index].1;
            let end = haystack_chars[index + needle_chars.len() - 1].2;
            ranges.push(start..end);
        }
    }
    ranges
}

fn expand_to_line_range(content: &str, range: Range<usize>) -> Range<usize> {
    let start = content[..range.start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = content[range.end..]
        .find('\n')
        .map(|index| range.end + index + 1)
        .unwrap_or(content.len());
    start..end
}

fn line_indent(content: &str, line_start: usize) -> &str {
    let tail = &content[line_start..];
    let indent_end = tail
        .char_indices()
        .find_map(|(index, character)| {
            (!matches!(character, ' ' | '\t')).then_some(line_start + index)
        })
        .unwrap_or(content.len());
    &content[line_start..indent_end]
}

fn reindent_replacement(
    replacement: &str,
    base_indent: &str,
    keep_trailing_newline: bool,
) -> String {
    let mut lines = replacement.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("");
    }
    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(|ch| matches!(ch, ' ' | '\t')).len())
        .min()
        .unwrap_or(0);
    let mut output = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{base_indent}{}", &line[common_indent..])
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if keep_trailing_newline && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn guard_path_rejects_absolute_escape() {
        let root = temp_root("oomu-tool-fs-escape");
        let tools = FileSystemTools::new(root.clone());

        let error = tools
            .guard_path("/private/etc/hosts", "file_read")
            .expect_err("absolute escape is rejected");
        assert!(error.message.contains("outside project quarantine"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn guard_path_keeps_implicit_downloads_inside_project_quarantine() {
        let root = temp_root("oomu-tool-fs-downloads");
        let tools = FileSystemTools::new(root.clone());

        let path = tools
            .guard_path("Downloads", "file_list")
            .expect("relative Downloads remains inside project quarantine");

        let canonical_root = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        assert_eq!(path, canonical_root.join("Downloads"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn guard_path_rejects_symlink_escape() {
        let root = temp_root("oomu-tool-fs-link");
        let outside = temp_root("oomu-tool-fs-outside");
        fs::create_dir_all(&outside).expect("outside directory creates");
        let tools = FileSystemTools::new(root.clone());
        fs::create_dir_all(&root).expect("root creates");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("outside-link"))
                .expect("symlink creates");
            let error = tools
                .guard_path("outside-link/report.txt", "file_write")
                .expect_err("symlink escape is rejected");
            assert!(error.message.contains("outside project quarantine"));
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn file_read_returns_observed_content_and_rejects_oversized_input() {
        let root = temp_root("oomu-tool-file-read-content");
        fs::create_dir_all(&root).expect("root creates");
        fs::write(root.join("notes.md"), "# Observed\nreal content\n").expect("text input writes");
        let tools = FileSystemTools::new(root.clone());

        let output = tools
            .read(FileReadRequest {
                path: "notes.md".to_string(),
            })
            .expect("bounded text file reads");
        assert!(output.message.contains("# Observed\nreal content"));

        let oversized =
            fs::File::create(root.join("oversized.txt")).expect("oversized input creates");
        oversized
            .set_len(MAX_AGENT_FILE_READ_BYTES + 1)
            .expect("oversized input expands");
        let error = tools
            .read(FileReadRequest {
                path: "oversized.txt".to_string(),
            })
            .expect_err("oversized input must not produce partial evidence");
        assert!(error.message.contains("native action limit"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codebase_patch_applies_unique_exact_match() {
        let root = temp_root("oomu-tool-codebase-patch-exact");
        let target = root.join("src/app/page.tsx");
        fs::create_dir_all(target.parent().expect("target parent")).expect("parent creates");
        fs::write(
            &target,
            "export default function Page() {\n  return null;\n}\n",
        )
        .expect("target writes");
        let tools = FileSystemTools::new(root.clone());

        let output = tools
            .codebase_patch(CodebasePatchRequest {
                target_file_path: "src/app/page.tsx".to_string(),
                search_pattern: "return null;".to_string(),
                replacement_content: "return <main />;".to_string(),
            })
            .expect("patch applies");

        assert_eq!(output.operation, "codebase_patch");
        assert!(fs::read_to_string(&target)
            .expect("patched file reads")
            .contains("return <main />;"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codebase_patch_matches_whitespace_and_preserves_indent() {
        let root = temp_root("oomu-tool-codebase-patch-fuzzy");
        let target = root.join("src/main.rs");
        fs::create_dir_all(target.parent().expect("target parent")).expect("parent creates");
        fs::write(
            &target,
            "fn main() {\n    if ready {\n        launch();\n    }\n}\n",
        )
        .expect("target writes");
        let tools = FileSystemTools::new(root.clone());

        tools
            .codebase_patch(CodebasePatchRequest {
                target_file_path: "src/main.rs".to_string(),
                search_pattern: "if ready {\n  launch();\n}".to_string(),
                replacement_content: "if ready {\n    deploy();\n}".to_string(),
            })
            .expect("fuzzy patch applies");

        let patched = fs::read_to_string(&target).expect("patched file reads");
        assert!(patched.contains("    if ready {\n        deploy();\n    }\n"));
        assert!(!patched.contains("launch();"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codebase_patch_rejects_ambiguous_normalized_matches() {
        let root = temp_root("oomu-tool-codebase-patch-ambiguous");
        let target = root.join("src/lib.rs");
        fs::create_dir_all(target.parent().expect("target parent")).expect("parent creates");
        fs::write(&target, "fn a(){ run(); }\nfn b() { run(); }\n").expect("target writes");
        let tools = FileSystemTools::new(root.clone());

        let error = tools
            .codebase_patch(CodebasePatchRequest {
                target_file_path: "src/lib.rs".to_string(),
                search_pattern: "run();".to_string(),
                replacement_content: "stop();".to_string(),
            })
            .expect_err("ambiguous patch rejected");
        assert!(error.message.contains("matched 2 exact locations"));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        ))
    }
}
