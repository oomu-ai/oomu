use crate::mcp::client::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::security::sandbox::SandboxRoot;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "local_filesystem";
const SERVER_VERSION: &str = "1.0.0";
const SANDBOX_ENV: &str = "OOMU_MCP_SANDBOX_DIR";
const PRIVATE_SANDBOX_REF: &str = "private://mcp-sandbox";
const SANDBOX_UNAVAILABLE: &str = "local_filesystem_sandbox_unavailable";
const PATH_REJECTED: &str = "local_filesystem_path_rejected";
const TARGET_NOT_DIRECTORY: &str = "local_filesystem_target_not_directory";
const TARGET_NOT_FILE: &str = "local_filesystem_target_not_file";
const DIRECTORY_UNAVAILABLE: &str = "local_filesystem_directory_unavailable";
const READ_FAILED: &str = "local_filesystem_read_failed";
const PARENT_UNAVAILABLE: &str = "local_filesystem_parent_unavailable";
const CONTENT_INVALID: &str = "local_filesystem_content_invalid";
const WRITE_FAILED: &str = "local_filesystem_write_failed";
const WRITE_VERIFICATION_FAILED: &str = "local_filesystem_write_verification_failed";
const DELETE_FAILED: &str = "local_filesystem_delete_failed";
const DELETE_VERIFICATION_FAILED: &str = "local_filesystem_delete_verification_failed";

#[derive(Debug, Clone)]
pub struct NativeLocalFilesystemServer {
    sandbox: SandboxRoot,
}

impl NativeLocalFilesystemServer {
    pub fn from_env(env: &HashMap<String, String>) -> Result<Self, String> {
        let sandbox_root = env
            .get(SANDBOX_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::mcp::bootstrap::mcp_sandbox_root);
        Self::new(sandbox_root)
    }

    pub fn new(sandbox_root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            sandbox: SandboxRoot::new(sandbox_root).map_err(|_| SANDBOX_UNAVAILABLE.to_string())?,
        })
    }

    pub fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id;
        let result = match request.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
            })),
            "tools/list" => Ok(json!({ "tools": tool_list() })),
            "tools/call" => Ok(self.call_tool(request.params)),
            _ => Err("local_filesystem_method_unsupported".to_string()),
        };

        match result {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(result),
                error: None,
                id,
            },
            Err(message) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(json!({"code": -32000, "message": message})),
                id,
            },
        }
    }

    pub fn handle_notification(&self, notification: JsonRpcNotification) -> Result<(), String> {
        match notification.method.as_str() {
            "notifications/initialized" => Ok(()),
            _ => Err("local_filesystem_notification_unsupported".to_string()),
        }
    }

    fn call_tool(&self, params: Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(arguments) = arguments.as_object() else {
            return error_result("Tool arguments must be an object.");
        };

        match name {
            "list_directory" => self
                .list_directory(arguments.get("path"))
                .unwrap_or_else(|error| error_result(&error)),
            "read_file" => self
                .read_file(arguments.get("path"))
                .unwrap_or_else(|error| error_result(&error)),
            "write_file" => self
                .write_file(arguments.get("path"), arguments.get("content"))
                .unwrap_or_else(|error| error_result(&error)),
            "delete_file" => self
                .delete_file(arguments.get("path"))
                .unwrap_or_else(|error| error_result(&error)),
            _ => error_result("local_filesystem_tool_unsupported"),
        }
    }

    fn list_directory(&self, raw_path: Option<&Value>) -> Result<Value, String> {
        let target = self.resolve_sandbox_path(raw_path)?;
        if !target.is_dir() {
            return Err(TARGET_NOT_DIRECTORY.to_string());
        }

        let raw_entries = fs::read_dir(&target)
            .map_err(|_| DIRECTORY_UNAVAILABLE.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| DIRECTORY_UNAVAILABLE.to_string())?;
        let mut entries = Vec::new();
        for entry in raw_entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|_| DIRECTORY_UNAVAILABLE.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| DIRECTORY_UNAVAILABLE.to_string())?;
            let mut display_name = entry.file_name().to_string_lossy().to_string();
            let kind = if file_type.is_dir() {
                display_name.push('/');
                "directory"
            } else if file_type.is_file() {
                "file"
            } else {
                continue;
            };
            entries.push(json!({
                "path": self.relative_path(&path),
                "name": display_name,
                "kind": kind,
                "bytes": metadata.len(),
            }));
        }
        entries.sort_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .cmp(&right.get("name").and_then(Value::as_str))
        });

        let text = if entries.is_empty() {
            "(directory is empty)".to_string()
        } else {
            entries
                .iter()
                .filter_map(|entry| entry.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        };

        Ok(text_result(
            &text,
            Some(json!({
                "root": PRIVATE_SANDBOX_REF,
                "relativePath": self.relative_path(&target),
                "files": entries,
            })),
        ))
    }

    fn read_file(&self, raw_path: Option<&Value>) -> Result<Value, String> {
        let raw_path = raw_path.ok_or_else(|| "read_file requires a path.".to_string())?;
        let target = self.resolve_sandbox_path(Some(raw_path))?;
        if !target.is_file() {
            return Err(TARGET_NOT_FILE.to_string());
        }
        let content = fs::read_to_string(&target).map_err(|_| READ_FAILED.to_string())?;
        let relative_path = self.relative_path(&target);
        Ok(text_result(
            &content,
            Some(json!({
                "path": relative_path.clone(),
                "relativePath": relative_path,
                "content": content,
            })),
        ))
    }

    fn write_file(
        &self,
        raw_path: Option<&Value>,
        content: Option<&Value>,
    ) -> Result<Value, String> {
        let raw_path = raw_path.ok_or_else(|| "write_file requires a path.".to_string())?;
        let content = content.ok_or_else(|| "write_file requires content.".to_string())?;
        let target = self.resolve_sandbox_path(Some(raw_path))?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| PARENT_UNAVAILABLE.to_string())?;
        }

        let content = match content.as_str() {
            Some(content) => content.to_string(),
            None => {
                serde_json::to_string_pretty(content).map_err(|_| CONTENT_INVALID.to_string())?
            }
        };
        fs::write(&target, &content).map_err(|_| WRITE_FAILED.to_string())?;
        let mut verified_file =
            fs::File::open(&target).map_err(|_| WRITE_VERIFICATION_FAILED.to_string())?;
        let mut actual = Vec::with_capacity(content.len());
        verified_file
            .read_to_end(&mut actual)
            .map_err(|_| WRITE_VERIFICATION_FAILED.to_string())?;
        if actual != content.as_bytes() {
            return Err(WRITE_VERIFICATION_FAILED.to_string());
        }
        let opened_metadata = verified_file
            .metadata()
            .map_err(|_| WRITE_VERIFICATION_FAILED.to_string())?;
        let path_metadata =
            fs::symlink_metadata(&target).map_err(|_| WRITE_VERIFICATION_FAILED.to_string())?;
        if !path_metadata.is_file()
            || path_metadata.file_type().is_symlink()
            || !same_file_identity(&opened_metadata, &path_metadata)
        {
            return Err(WRITE_VERIFICATION_FAILED.to_string());
        }

        let rel = self.relative_path(&target);
        Ok(text_result(
            &format!("Execution Completed: {rel} generated successfully."),
            Some(json!({
                "path": rel.clone(),
                "relativePath": rel,
                "bytesWritten": content.len(),
                "exists": true,
                "verified": true,
                "contentSha256": crate::foundation::digest::sha256_hex(&actual),
                "targetIdentityVerified": true,
            })),
        ))
    }

    fn delete_file(&self, raw_path: Option<&Value>) -> Result<Value, String> {
        let raw_path = raw_path.ok_or_else(|| "delete_file requires a path.".to_string())?;
        let target = self.resolve_sandbox_path(Some(raw_path))?;
        if !target.is_file() {
            return Err(TARGET_NOT_FILE.to_string());
        }

        fs::remove_file(&target).map_err(|_| DELETE_FAILED.to_string())?;

        if target.exists() {
            return Err(DELETE_VERIFICATION_FAILED.to_string());
        }

        let rel = self.relative_path(&target);
        Ok(text_result(
            &format!("Deleted file: {rel}"),
            Some(json!({
                "path": rel.clone(),
                "relativePath": rel,
                "deleted": true,
            })),
        ))
    }

    fn resolve_sandbox_path(&self, raw_path: Option<&Value>) -> Result<PathBuf, String> {
        let value = raw_path
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| raw_path.map(Value::to_string))
            .unwrap_or_default();
        let raw = PathBuf::from(value);
        self.sandbox
            .resolve(raw)
            .map_err(|_| PATH_REJECTED.to_string())
    }

    fn relative_path(&self, path: &Path) -> String {
        self.sandbox.relative_path(path)
    }
}

#[cfg(unix)]
fn same_file_identity(opened: &fs::Metadata, current_path: &fs::Metadata) -> bool {
    opened.dev() == current_path.dev() && opened.ino() == current_path.ino()
}

#[cfg(not(unix))]
fn same_file_identity(opened: &fs::Metadata, current_path: &fs::Metadata) -> bool {
    opened.len() == current_path.len() && opened.modified().ok() == current_path.modified().ok()
}

fn tool_list() -> Vec<Value> {
    let path_property = json!({
        "type": "string",
        "description": "Relative sandbox path, or an absolute path inside the sandbox."
    });
    vec![
        json!({
            "name": "list_directory",
            "description": "List files inside the secure local sandbox.",
            "outputSchema": {
                "type": "object",
                "x-oomu-result-contract": {
                    "kind": "collection",
                    "path": "/structuredContent/files",
                    "emptyIsSuccess": true
                },
                "properties": {
                    "structuredContent": {
                        "type": "object",
                        "properties": {"files": {"type": "array", "items": {}}},
                        "required": ["files"],
                        "additionalProperties": true
                    }
                },
                "required": ["structuredContent"],
                "additionalProperties": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {"path": path_property},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "read_file",
            "description": "Read a UTF-8 text file inside the secure local sandbox.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": path_property},
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "write_file",
            "description": "Write UTF-8 text to a file inside the secure local sandbox.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path_property,
                    "content": {
                        "type": "string",
                        "description": "Text content to write."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "delete_file",
            "description": "Delete a regular file inside the secure local sandbox.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": path_property},
                "required": ["path"],
                "additionalProperties": false
            }
        }),
    ]
}

fn text_result(text: &str, structured: Option<Value>) -> Value {
    let mut result = json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    });
    if let Some(structured) = structured {
        result["structuredContent"] = structured;
    }
    result
}

fn error_result(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn native_filesystem_reads_writes_lists_and_rejects_escape() {
        let root = temp_root("oomu-native-fs");
        let server = NativeLocalFilesystemServer::new(root.clone()).expect("server initializes");

        let write = server
            .write_file(
                Some(&json!("reports/out.txt")),
                Some(&json!("approved output")),
            )
            .expect("write succeeds");
        assert_eq!(write["isError"], json!(false));
        assert_eq!(
            write["structuredContent"]["relativePath"],
            json!("reports/out.txt")
        );
        assert_eq!(write["structuredContent"]["path"], json!("reports/out.txt"));
        assert_eq!(write["structuredContent"]["exists"], json!(true));
        assert_eq!(write["structuredContent"]["verified"], json!(true));
        assert_eq!(
            write["structuredContent"]["targetIdentityVerified"],
            json!(true)
        );

        let read = server
            .read_file(Some(&json!("reports/out.txt")))
            .expect("read succeeds");
        assert_eq!(
            read["structuredContent"]["content"],
            json!("approved output")
        );

        let listed = server
            .list_directory(Some(&json!("")))
            .expect("list succeeds");
        assert_eq!(listed["isError"], json!(false));
        assert_eq!(
            listed["structuredContent"]["root"],
            json!(PRIVATE_SANDBOX_REF)
        );
        assert!(listed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("reports/"));

        let escaped = server
            .read_file(Some(&json!("/private/etc/hosts")))
            .expect_err("outside path is rejected");
        assert_eq!(escaped, PATH_REJECTED);

        let deleted = server
            .delete_file(Some(&json!("reports/out.txt")))
            .expect("delete succeeds");
        assert_eq!(deleted["isError"], json!(false));
        assert_eq!(deleted["structuredContent"]["deleted"], json!(true));
        assert!(
            !root.join("reports/out.txt").exists(),
            "delete_file verifies the file is gone"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_filesystem_empty_write_returns_verified_identity_proof() {
        let root = temp_root("oomu-native-fs-empty");
        let server = NativeLocalFilesystemServer::new(root.clone()).expect("server initializes");

        let write = server
            .write_file(Some(&json!("placeholder.txt")), Some(&json!("")))
            .expect("explicit empty write succeeds");

        assert_eq!(write["structuredContent"]["bytesWritten"], json!(0));
        assert_eq!(write["structuredContent"]["exists"], json!(true));
        assert_eq!(write["structuredContent"]["verified"], json!(true));
        assert_eq!(
            write["structuredContent"]["contentSha256"],
            json!("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            write["structuredContent"]["targetIdentityVerified"],
            json!(true)
        );
        assert!(root.join("placeholder.txt").is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_filesystem_rejects_symlink_escape() {
        let root = temp_root("oomu-native-fs-link");
        let outside = temp_root("oomu-native-fs-outside");
        fs::create_dir_all(&outside).expect("outside directory creates");
        fs::write(outside.join("hosts"), "outside").expect("outside file writes");
        let server = NativeLocalFilesystemServer::new(root.clone()).expect("server initializes");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("etc-link")).expect("symlink creates");
            let escaped = server
                .read_file(Some(&json!("etc-link/hosts")))
                .expect_err("symlink escape is rejected");
            assert_eq!(escaped, PATH_REJECTED);
            let listed = server
                .list_directory(Some(&json!("")))
                .expect("list succeeds");
            assert!(!listed["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("etc-link"));
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn native_filesystem_accepts_case_only_sandbox_path_differences() {
        let root = temp_root("OOMU-Native-FS-Case");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Instruction_Input.txt"), "case-safe").unwrap();
        let server_root = PathBuf::from(root.to_string_lossy().to_lowercase());
        let server = NativeLocalFilesystemServer::new(server_root).expect("server initializes");
        let case_variant = root.join("instruction_input.txt");

        let read = server
            .read_file(Some(&json!(case_variant)))
            .expect("case-only absolute path is accepted");
        assert_eq!(read["structuredContent"]["content"], json!("case-safe"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renderer_results_never_serialize_absolute_sandbox_paths() {
        let root = temp_root("oomu-native-fs-private-root-canary");
        fs::create_dir_all(root.join("nested")).unwrap();
        let absolute_target = root.join("nested/private.txt");
        let server = NativeLocalFilesystemServer::new(root.clone()).expect("server initializes");

        let write = server
            .write_file(
                Some(&json!("nested/private.txt")),
                Some(&json!("approved relative content")),
            )
            .unwrap();
        let listed = server.list_directory(Some(&json!("nested"))).unwrap();
        let read = server
            .read_file(Some(&json!("nested/private.txt")))
            .unwrap();
        let deleted = server
            .delete_file(Some(&json!("nested/private.txt")))
            .unwrap();

        assert_eq!(
            listed["structuredContent"]["root"],
            json!(PRIVATE_SANDBOX_REF)
        );
        assert_eq!(listed["structuredContent"]["relativePath"], json!("nested"));
        for result in [&write, &listed, &read, &deleted] {
            let serialized = serde_json::to_string(result).unwrap();
            assert!(!serialized.contains(root.to_string_lossy().as_ref()));
            assert!(!serialized.contains(absolute_target.to_string_lossy().as_ref()));
        }
        for result in [&write, &read, &deleted] {
            assert_eq!(
                result["structuredContent"]["path"],
                json!("nested/private.txt")
            );
            assert_eq!(
                result["structuredContent"]["relativePath"],
                json!("nested/private.txt")
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renderer_errors_use_generic_codes_without_local_paths() {
        let root = temp_root("oomu-native-fs-error-root-canary");
        fs::create_dir_all(&root).unwrap();
        let invalid_utf8 = root.join("private-error-canary.txt");
        fs::write(&invalid_utf8, [0xff, 0xfe, 0xfd]).unwrap();
        let server = NativeLocalFilesystemServer::new(root.clone()).expect("server initializes");

        let read_error = server.call_tool(json!({
            "name": "read_file",
            "arguments": {"path": "private-error-canary.txt"}
        }));
        assert_eq!(read_error["content"][0]["text"], json!(READ_FAILED));
        let serialized = serde_json::to_string(&read_error).unwrap();
        assert!(!serialized.contains(root.to_string_lossy().as_ref()));
        assert!(!serialized.contains(invalid_utf8.to_string_lossy().as_ref()));

        let outside = root
            .parent()
            .unwrap_or(Path::new("/"))
            .join("outside-path-canary.txt");
        let rejected = server.call_tool(json!({
            "name": "read_file",
            "arguments": {"path": outside.to_string_lossy()}
        }));
        assert_eq!(rejected["content"][0]["text"], json!(PATH_REJECTED));
        assert!(!serde_json::to_string(&rejected)
            .unwrap()
            .contains(outside.to_string_lossy().as_ref()));

        let invalid_root = root.join("not-a-directory-root");
        fs::write(&invalid_root, b"file").unwrap();
        let initialization_error = NativeLocalFilesystemServer::new(invalid_root.clone())
            .expect_err("file roots are rejected");
        assert_eq!(initialization_error, SANDBOX_UNAVAILABLE);
        assert!(!initialization_error.contains(invalid_root.to_string_lossy().as_ref()));

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
