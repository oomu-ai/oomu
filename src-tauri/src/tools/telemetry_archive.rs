use super::{ToolError, ToolOutput};
use crate::foundation::clock::{
    unix_time_ms_i64 as unix_time_ms, unix_time_secs_u64 as unix_time_secs,
};
use crate::shield_gate::TelemetryArchiveRequest;
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use sysinfo::System;

const TAR_BLOCK_SIZE: usize = 512;
const COMMAND_TIMEOUT: Duration = Duration::from_millis(1200);

pub struct TelemetryArchiveTools {
    project_root: PathBuf,
}

struct ArchiveEntry {
    name: String,
    bytes: Vec<u8>,
}

impl TelemetryArchiveTools {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    pub fn create(&self, request: TelemetryArchiveRequest) -> Result<ToolOutput, ToolError> {
        let output_path = PathBuf::from(request.output_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| ToolError {
                operation: "telemetry_archive".to_string(),
                message: format!("Failed to create {}: {error}", parent.display()),
            })?;
        }

        let entries = collect_telemetry_entries(&self.project_root).map_err(|error| ToolError {
            operation: "telemetry_archive".to_string(),
            message: error,
        })?;
        write_gzip_tar(&output_path, &entries).map_err(|error| ToolError {
            operation: "telemetry_archive".to_string(),
            message: format!(
                "Unable to write telemetry archive {}: {error}",
                output_path.display()
            ),
        })?;

        let bytes = fs::metadata(&output_path)
            .map_err(|error| ToolError {
                operation: "telemetry_archive".to_string(),
                message: format!(
                    "Unable to verify telemetry archive {}: {error}",
                    output_path.display()
                ),
            })?
            .len();
        if bytes == 0 {
            return Err(ToolError {
                operation: "telemetry_archive".to_string(),
                message: format!(
                    "Unable to verify telemetry archive {}: file is empty.",
                    output_path.display()
                ),
            });
        }

        Ok(ToolOutput {
            operation: "telemetry_archive".to_string(),
            message: format!(
                "Packaged {} telemetry file(s) into {} ({} byte(s)).",
                entries.len(),
                output_path.display(),
                bytes
            ),
            claims: vec![format!(
                "CLAIM shield_gate_approved_external_write path={} min_bytes={bytes}",
                output_path.display()
            )],
        })
    }
}

fn collect_telemetry_entries(project_root: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let collected_at_ms = unix_time_ms();
    let build_bundle_parent = project_root
        .join("src-tauri")
        .join("target")
        .join("release")
        .join("bundle")
        .join("macos");
    let build_path = build_bundle_parent.join("OOMU.app");
    let build_executable_path = build_path.join("Contents").join("MacOS").join("oomu");
    let mods_dir = home_dir()
        .ok_or_else(|| "Unable to locate the user home directory for the mods audit.".to_string())?
        .join(".oomu")
        .join("mods");
    let testing_dir = project_root.join("planning").join("testing");
    let mut system = System::new_all();
    system.refresh_all();

    let process_snapshot = process_snapshot();
    let applescript_snapshot = applescript_process_snapshot();
    let manifests = mod_manifest_snapshots(&mods_dir);
    let directory_structures = [
        ("build_bundle_parent", build_bundle_parent.as_path()),
        ("mods", mods_dir.as_path()),
        ("testing", testing_dir.as_path()),
    ]
    .into_iter()
    .map(|(label, path)| {
        format!(
            "## {label}: {}\n{}",
            path.display(),
            directory_tree(path, 2, 80)
        )
    })
    .collect::<Vec<_>>()
    .join("\n\n");

    let telemetry = json!({
        "collectedAtMs": collected_at_ms,
        "projectRoot": project_root.display().to_string(),
        "targetBuild": path_snapshot(&build_path),
        "targetExecutable": executable_snapshot(&build_executable_path),
        "targetBuildCandidates": [
            labeled_path_snapshot("tauri_product", &build_path),
        ],
        "modsDirectory": path_snapshot(&mods_dir),
        "testingDirectory": path_snapshot(&testing_dir),
        "hardware": {
            "cpuCount": system.cpus().len(),
            "loadAverage": {
                "one": System::load_average().one,
                "five": System::load_average().five,
                "fifteen": System::load_average().fifteen,
            },
            "totalMemoryBytes": system.total_memory(),
            "usedMemoryBytes": system.used_memory(),
            "freeMemoryBytes": system.free_memory(),
            "availableMemoryBytes": system.available_memory(),
        },
        "processSnapshot": process_snapshot,
        "appleScriptSnapshot": applescript_snapshot,
        "modManifests": manifests,
        "directoryStructures": directory_structures,
    });
    let telemetry_json = serde_json::to_vec_pretty(&telemetry)
        .map_err(|error| format!("Unable to serialize telemetry JSON: {error}"))?;
    let markdown = telemetry_markdown(&telemetry);
    let manifests = telemetry
        .get("modManifests")
        .ok_or_else(|| "Telemetry payload omitted mod manifest evidence.".to_string())?;
    let manifests_text = serde_json::to_string_pretty(manifests)
        .map_err(|error| format!("Unable to serialize mod manifest evidence: {error}"))?;
    let process_snapshot = telemetry
        .get("processSnapshot")
        .ok_or_else(|| "Telemetry payload omitted process evidence.".to_string())?;
    let process_text = serde_json::to_string_pretty(process_snapshot)
        .map_err(|error| format!("Unable to serialize process evidence: {error}"))?;

    Ok(vec![
        ArchiveEntry {
            name: "telemetry_audit.json".to_string(),
            bytes: telemetry_json,
        },
        ArchiveEntry {
            name: "telemetry_audit.md".to_string(),
            bytes: markdown.into_bytes(),
        },
        ArchiveEntry {
            name: "directory_structures.txt".to_string(),
            bytes: directory_structures.into_bytes(),
        },
        ArchiveEntry {
            name: "mod_manifests.json".to_string(),
            bytes: manifests_text.into_bytes(),
        },
        ArchiveEntry {
            name: "process_snapshot.json".to_string(),
            bytes: process_text.into_bytes(),
        },
    ])
}

fn telemetry_markdown(telemetry: &Value) -> String {
    let target = telemetry
        .get("targetBuild")
        .and_then(|value| value.get("exists"))
        .and_then(Value::as_bool)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    let output = telemetry
        .get("collectedAtMs")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    let executable_architecture = telemetry
        .pointer("/targetExecutable/architecture/architecture")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let apple_silicon = telemetry
        .pointer("/targetExecutable/architecture/appleSilicon")
        .and_then(Value::as_bool)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    let oomu_process_detected = telemetry
        .pointer("/processSnapshot/oomuProcessDetected")
        .and_then(Value::as_bool)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    format!(
        "# Telemetry Audit\n\nCollected at: {output}\n\n- Selected app bundle present: {target}\n- Selected app bundle path: {}\n- Main executable architecture: {executable_architecture}\n- Apple Silicon executable: {apple_silicon}\n- OOMU process detected: {oomu_process_detected}\n- Mods directory: {}\n- Testing directory: {}\n\nSee `telemetry_audit.json` for the structured payload.\n",
        telemetry
            .get("targetBuild")
            .and_then(|value| value.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        telemetry
            .get("modsDirectory")
            .and_then(|value| value.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        telemetry
            .get("testingDirectory")
            .and_then(|value| value.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
    )
}

fn path_snapshot(path: &Path) -> Value {
    match fs::symlink_metadata(path) {
        Ok(metadata) => json!({
            "path": path.display().to_string(),
            "exists": true,
            "isDirectory": metadata.is_dir(),
            "isFile": metadata.is_file(),
            "isSymlink": metadata.file_type().is_symlink(),
            "bytes": metadata.len(),
        }),
        Err(error) => json!({
            "path": path.display().to_string(),
            "exists": false,
            "error": error.to_string(),
        }),
    }
}

fn labeled_path_snapshot(label: &str, path: &Path) -> Value {
    let mut snapshot = path_snapshot(path);
    if let Value::Object(map) = &mut snapshot {
        map.insert("label".to_string(), Value::String(label.to_string()));
    }
    snapshot
}

fn executable_snapshot(path: &Path) -> Value {
    let mut snapshot = path_snapshot(path);
    let architecture = mach_o_architecture(path);
    if let Value::Object(map) = &mut snapshot {
        map.insert("architecture".to_string(), architecture);
    }
    snapshot
}

fn mach_o_architecture(path: &Path) -> Value {
    let mut header = [0_u8; 8];
    let read = File::open(path).and_then(|mut file| file.read_exact(&mut header));
    if let Err(error) = read {
        return json!({
            "format": "unavailable",
            "appleSilicon": false,
            "error": error.to_string(),
        });
    }

    let magic_le = u32::from_le_bytes(header[0..4].try_into().unwrap_or_default());
    let magic_be = u32::from_be_bytes(header[0..4].try_into().unwrap_or_default());
    let (format, cpu_type) = match (magic_le, magic_be) {
        (0xfeedfacf, _) => (
            "mach-o-64",
            u32::from_le_bytes(header[4..8].try_into().unwrap()),
        ),
        (_, 0xfeedfacf) => (
            "mach-o-64",
            u32::from_be_bytes(header[4..8].try_into().unwrap()),
        ),
        (0xfeedface, _) => (
            "mach-o-32",
            u32::from_le_bytes(header[4..8].try_into().unwrap()),
        ),
        (_, 0xfeedface) => (
            "mach-o-32",
            u32::from_be_bytes(header[4..8].try_into().unwrap()),
        ),
        (_, 0xcafebabe | 0xcafebabf) => {
            return json!({
                "format": "mach-o-universal",
                "architecture": "universal",
                "appleSilicon": null,
            });
        }
        _ => {
            return json!({
                "format": "unknown",
                "magicHex": hex::encode(&header[0..4]),
                "appleSilicon": false,
            });
        }
    };
    let architecture = match cpu_type {
        0x0100_000c => "arm64",
        0x0100_0007 => "x86_64",
        12 => "arm",
        7 => "x86",
        _ => "unknown",
    };
    json!({
        "format": format,
        "cpuType": cpu_type,
        "architecture": architecture,
        "appleSilicon": architecture == "arm64",
    })
}

fn mod_manifest_snapshots(mods_dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(mods_dir) else {
        return vec![json!({
            "modsDirectory": mods_dir.display().to_string(),
            "status": "unavailable",
        })];
    };

    let mut manifests = entries
        .take(80)
        .map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    return json!({
                        "modsDirectory": mods_dir.display().to_string(),
                        "status": "entry_unavailable",
                        "error": error.to_string(),
                    });
                }
            };
            let path = entry.path();
            let manifest_path = if path.is_dir() {
                path.join("manifest.json")
            } else {
                path.clone()
            };
            let manifest_name = manifest_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if manifest_name != "manifest.json" && !manifest_name.ends_with(".oomu") {
                return json!({
                    "path": path.display().to_string(),
                    "status": "skipped",
                    "reason": "not a manifest candidate",
                });
            }
            match fs::read_to_string(&manifest_path) {
                Ok(content) => {
                    let parsed = serde_json::from_str::<Value>(&content).ok();
                    let validation = parsed
                        .as_ref()
                        .ok_or_else(|| "manifest.json is not valid JSON".to_string())
                        .and_then(|value| validate_installed_mod_manifest(value, &path));
                    json!({
                        "path": manifest_path.display().to_string(),
                        "status": if validation.is_ok() {
                            "valid_oomu_manifest"
                        } else if parsed.is_some() {
                            "schema_mismatch"
                        } else {
                            "invalid_json"
                        },
                        "bytes": content.len(),
                        "manifest": parsed,
                        "validationError": validation.err(),
                    })
                }
                Err(error) => json!({
                    "path": manifest_path.display().to_string(),
                    "status": "unreadable",
                    "error": error.to_string(),
                }),
            }
        })
        .collect::<Vec<_>>();
    manifests.sort_by(|left, right| {
        left.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    manifests
}

fn validate_installed_mod_manifest(value: &Value, installed_path: &Path) -> Result<(), String> {
    let manifest = serde_json::from_value::<crate::security::mods::ModManifest>(value.clone())
        .map_err(|error| format!("manifest.json does not match the OOMU mod schema: {error}"))?;
    for (label, field) in [
        ("id", manifest.id.as_str()),
        ("name", manifest.name.as_str()),
        ("version", manifest.version.as_str()),
        ("author", manifest.author.as_str()),
        ("description", manifest.description.as_str()),
    ] {
        if field.trim().is_empty() {
            return Err(format!("manifest.json field `{label}` cannot be empty."));
        }
    }
    let entrypoint = Path::new(manifest.entrypoint.trim());
    if manifest.entrypoint.trim().is_empty() {
        return Err("manifest.json field `entrypoint` cannot be empty.".to_string());
    }
    if entrypoint.is_absolute()
        || entrypoint
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("manifest.json entrypoint must stay inside the installed mod.".to_string());
    }
    let install_dir = if installed_path.is_dir() {
        installed_path
    } else {
        installed_path.parent().unwrap_or(installed_path)
    };
    if !install_dir.join(entrypoint).is_file() {
        return Err(format!(
            "Mod entrypoint `{}` is missing from the installed mod.",
            manifest.entrypoint
        ));
    }
    Ok(())
}

fn process_snapshot() -> Value {
    let output = command_output_with_timeout(
        {
            let mut command = Command::new("ps");
            command.args(["-axo", "pid=,comm="]);
            command
        },
        COMMAND_TIMEOUT,
    );
    process_snapshot_from_command_output(output)
}

fn process_snapshot_from_command_output(output: Value) -> Value {
    let mut detected = Vec::new();
    let mut oomu_process_detected = false;
    let probe_succeeded = output.get("status").and_then(Value::as_str) == Some("completed");
    if probe_succeeded {
        let stdout = output
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default();
        for line in stdout.lines() {
            let lowered = line.to_ascii_lowercase();
            let command_name = lowered.split_whitespace().nth(1).unwrap_or_default();
            if command_name == "oomu" || command_name.ends_with("/oomu") {
                oomu_process_detected = true;
            }
            if [
                "visual studio code",
                "code",
                "terminal",
                "iterm",
                "textedit",
                "cursor",
                "zed",
            ]
            .iter()
            .any(|needle| lowered.contains(needle))
            {
                detected.push(line.trim().to_string());
            }
        }
    }

    json!({
        "command": "ps -axo pid=,comm=",
        "probeSucceeded": probe_succeeded,
        "oomuProcessDetected": if probe_succeeded {
            Value::Bool(oomu_process_detected)
        } else {
            Value::Null
        },
        "detectedEditorProcesses": if probe_succeeded {
            json!(detected)
        } else {
            Value::Null
        },
        "raw": output,
    })
}

fn applescript_process_snapshot() -> Value {
    #[cfg(target_os = "macos")]
    {
        command_output_with_timeout(
            {
                let mut command = Command::new("osascript");
                command.args([
                    "-e",
                    "tell application \"System Events\" to get name of every process whose background only is false",
                ]);
                command
            },
            COMMAND_TIMEOUT,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        json!({
            "status": "skipped",
            "reason": "AppleScript process query is only available on macOS.",
        })
    }
}

fn command_output_with_timeout(mut command: Command, timeout: Duration) -> Value {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return json!({
                "status": "failed_to_start",
                "error": error.to_string(),
            });
        }
    };

    loop {
        match child.try_wait() {
            Ok(Some(_)) => match child.wait_with_output() {
                Ok(output) => {
                    return json!({
                        "status": if output.status.success() { "completed" } else { "failed" },
                        "exitCode": output.status.code(),
                        "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
                        "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
                    });
                }
                Err(error) => {
                    return json!({
                        "status": "output_failed",
                        "error": error.to_string(),
                    });
                }
            },
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({
                    "status": "timed_out",
                    "timeoutMs": timeout.as_millis(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(30)),
            Err(error) => {
                let _ = child.kill();
                return json!({
                    "status": "poll_failed",
                    "error": error.to_string(),
                });
            }
        }
    }
}

fn directory_tree(path: &Path, max_depth: usize, max_entries: usize) -> String {
    let mut lines = Vec::new();
    directory_tree_inner(path, 0, max_depth, max_entries, &mut lines);
    if lines.is_empty() {
        "(unavailable)".to_string()
    } else {
        lines.join("\n")
    }
}

fn directory_tree_inner(
    path: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    lines: &mut Vec<String>,
) {
    if lines.len() >= max_entries {
        return;
    }
    let indent = "  ".repeat(depth);
    lines.push(format!("{indent}{}", path.display()));
    if depth >= max_depth {
        return;
    }
    let read_entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            lines.push(format!("{indent}  [unavailable: {error}]"));
            return;
        }
    };
    let mut entries_with_errors = Vec::new();
    let mut entries = Vec::new();
    for entry in read_entries {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => entries_with_errors.push(error.to_string()),
        }
    }
    entries.sort_by_key(|entry| entry.file_name());
    for error in entries_with_errors {
        if lines.len() >= max_entries {
            return;
        }
        lines.push(format!("{indent}  [entry unavailable: {error}]"));
    }
    for entry in entries {
        if lines.len() >= max_entries {
            lines.push(format!("{indent}  ..."));
            return;
        }
        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                lines.push(format!(
                    "{indent}  [type unavailable for {}: {error}]",
                    entry.file_name().to_string_lossy()
                ));
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            directory_tree_inner(&entry_path, depth + 1, max_depth, max_entries, lines);
        } else {
            lines.push(format!("{indent}  {}", entry.file_name().to_string_lossy()));
        }
    }
}

fn write_gzip_tar(path: &Path, entries: &[ArchiveEntry]) -> io::Result<()> {
    let file = File::create(path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    for entry in entries {
        append_tar_entry(&mut encoder, entry)?;
    }
    encoder.write_all(&[0_u8; TAR_BLOCK_SIZE])?;
    encoder.write_all(&[0_u8; TAR_BLOCK_SIZE])?;
    encoder.finish()?;
    Ok(())
}

fn append_tar_entry<W: Write>(writer: &mut W, entry: &ArchiveEntry) -> io::Result<()> {
    let name = sanitize_tar_name(&entry.name);
    let mut header = [0_u8; TAR_BLOCK_SIZE];
    write_bytes(&mut header[0..100], name.as_bytes());
    write_octal(&mut header[100..108], 0o644);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], entry.bytes.len() as u64);
    write_octal(&mut header[136..148], unix_time_secs());
    for byte in &mut header[148..156] {
        *byte = b' ';
    }
    header[156] = b'0';
    write_bytes(&mut header[257..263], b"ustar\0");
    write_bytes(&mut header[263..265], b"00");

    let checksum = header.iter().map(|byte| *byte as u32).sum::<u32>();
    let checksum_text = format!("{checksum:06o}\0 ");
    write_bytes(&mut header[148..156], checksum_text.as_bytes());

    writer.write_all(&header)?;
    writer.write_all(&entry.bytes)?;
    let remainder = entry.bytes.len() % TAR_BLOCK_SIZE;
    if remainder != 0 {
        writer.write_all(&vec![0_u8; TAR_BLOCK_SIZE - remainder])?;
    }
    Ok(())
}

fn write_bytes(target: &mut [u8], value: &[u8]) {
    let len = target.len().min(value.len());
    target[..len].copy_from_slice(&value[..len]);
}

fn write_octal(target: &mut [u8], value: u64) {
    let digits = target.len().saturating_sub(1);
    let text = format!("{value:0digits$o}\0");
    write_bytes(target, text.as_bytes());
}

fn sanitize_tar_name(name: &str) -> String {
    let cleaned = name
        .replace('\\', "/")
        .trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect::<Vec<_>>()
        .join("/");
    let cleaned = if cleaned.is_empty() {
        "telemetry_audit.txt".to_string()
    } else {
        cleaned
    };
    if cleaned.len() <= 100 {
        cleaned
    } else {
        cleaned
            .chars()
            .rev()
            .take(100)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn gzip_tar_writer_creates_ustar_entries() {
        let temp_path =
            env::temp_dir().join(format!("oomu-telemetry-test-{}.tar.gz", unix_time_ms()));
        let entries = vec![ArchiveEntry {
            name: "telemetry_audit.json".to_string(),
            bytes: br#"{"ok":true}"#.to_vec(),
        }];

        write_gzip_tar(&temp_path, &entries).expect("archive writes");
        let file = File::open(&temp_path).expect("archive opens");
        let mut decoder = GzDecoder::new(file);
        let mut tar_bytes = Vec::new();
        decoder
            .read_to_end(&mut tar_bytes)
            .expect("gzip decompresses");

        assert!(tar_bytes
            .windows("telemetry_audit.json".len())
            .any(|window| window == b"telemetry_audit.json"));
        assert!(tar_bytes
            .windows(11)
            .any(|window| window == br#"{"ok":true}"#));
        let _ = fs::remove_file(temp_path);
    }

    #[test]
    fn installed_mod_manifest_snapshot_verifies_schema_and_entrypoint() {
        let temp_dir =
            env::temp_dir().join(format!("oomu-telemetry-manifest-test-{}", unix_time_ms()));
        let mod_dir = temp_dir.join("ai.eldris.mods.audit-test");
        fs::create_dir_all(&mod_dir).expect("mod directory is created");
        fs::write(mod_dir.join("main.js"), "export default {};").expect("entrypoint is written");
        fs::write(
            mod_dir.join("manifest.json"),
            serde_json::json!({
                "id": "ai.eldris.mods.audit-test",
                "name": "Audit Test",
                "version": "1.0.0",
                "author": "Eldris",
                "description": "Telemetry manifest fixture.",
                "entrypoint": "main.js"
            })
            .to_string(),
        )
        .expect("manifest is written");

        let snapshots = mod_manifest_snapshots(&temp_dir);

        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].get("status").and_then(Value::as_str),
            Some("valid_oomu_manifest")
        );
        assert_eq!(snapshots[0].get("validationError"), Some(&Value::Null));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn executable_snapshot_identifies_apple_silicon_mach_o() {
        let executable_path =
            env::temp_dir().join(format!("oomu-telemetry-mach-o-test-{}", unix_time_ms()));
        let mut header = Vec::new();
        header.extend_from_slice(&0xfeedfacf_u32.to_le_bytes());
        header.extend_from_slice(&0x0100_000c_u32.to_le_bytes());
        fs::write(&executable_path, header).expect("Mach-O fixture is written");

        let snapshot = executable_snapshot(&executable_path);
        let architecture = snapshot
            .get("architecture")
            .expect("architecture snapshot is present");

        assert_eq!(
            architecture.get("format").and_then(Value::as_str),
            Some("mach-o-64")
        );
        assert_eq!(
            architecture.get("architecture").and_then(Value::as_str),
            Some("arm64")
        );
        assert_eq!(
            architecture.get("appleSilicon").and_then(Value::as_bool),
            Some(true)
        );
        let _ = fs::remove_file(executable_path);
    }

    #[test]
    fn failed_process_probe_does_not_fabricate_empty_observations() {
        let snapshot = process_snapshot_from_command_output(json!({
            "status": "failed_to_start",
            "error": "probe unavailable"
        }));

        assert_eq!(snapshot.get("probeSucceeded"), Some(&Value::Bool(false)));
        assert_eq!(snapshot.get("oomuProcessDetected"), Some(&Value::Null));
        assert_eq!(snapshot.get("detectedEditorProcesses"), Some(&Value::Null));
    }

    #[test]
    fn telemetry_archive_create_packages_complete_local_audit() {
        let project_root =
            env::temp_dir().join(format!("oomu-telemetry-contract-test-{}", unix_time_ms()));
        let executable_path =
            project_root.join("src-tauri/target/release/bundle/macos/OOMU.app/Contents/MacOS/oomu");
        fs::create_dir_all(executable_path.parent().unwrap()).expect("bundle fixture is created");
        let mut header = Vec::new();
        header.extend_from_slice(&0xfeedfacf_u32.to_le_bytes());
        header.extend_from_slice(&0x0100_000c_u32.to_le_bytes());
        fs::write(&executable_path, header).expect("bundle executable fixture is written");
        let output_path = project_root
            .join("planning/testing")
            .join("telemetry_audit.tar.gz");

        let output = TelemetryArchiveTools::new(project_root.clone())
            .create(TelemetryArchiveRequest {
                output_path: output_path.display().to_string(),
            })
            .expect("telemetry archive is created");

        assert_eq!(output.operation, "telemetry_archive");
        assert!(output_path.is_file());
        let file = File::open(&output_path).expect("archive opens");
        let mut decoder = GzDecoder::new(file);
        let mut tar_bytes = Vec::new();
        decoder
            .read_to_end(&mut tar_bytes)
            .expect("archive decompresses");
        for required in [
            "telemetry_audit.json",
            "directory_structures.txt",
            "mod_manifests.json",
            "process_snapshot.json",
            "\"appleScriptSnapshot\"",
            "\"hardware\"",
            "\"appleSilicon\": true",
            "\"architecture\": \"arm64\"",
        ] {
            assert!(
                tar_bytes
                    .windows(required.len())
                    .any(|window| window == required.as_bytes()),
                "archive is missing {required}"
            );
        }
        let _ = fs::remove_dir_all(project_root);
    }
}
