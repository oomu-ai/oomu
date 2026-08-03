use crate::tool_security::{
    classify_system_action, CapabilityClassification, CapabilityRiskTier, SystemActionClass,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const DEFAULT_TERMINAL_TIMEOUT_MS: u64 = 30_000;
pub const MAX_TERMINAL_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeTerminalRequest {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

impl NativeTerminalRequest {
    pub fn validate(mut self) -> Result<Self, String> {
        self.executable = self.executable.trim().to_string();
        if self.executable.is_empty() {
            return Err("terminal_execute.executable must not be empty.".to_string());
        }
        if contains_nul(&self.executable)
            || self.args.iter().any(|value| contains_nul(value))
            || self.env.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || key.contains('=')
                    || contains_nul(key)
                    || contains_nul(value)
            })
            || self.cwd.as_deref().is_some_and(contains_nul)
        {
            return Err("terminal_execute contains an invalid executable, argument, environment key, or working directory.".to_string());
        }
        if self
            .timeout
            .is_some_and(|timeout| timeout == 0 || timeout > MAX_TERMINAL_TIMEOUT_MS)
        {
            return Err(format!(
                "terminal_execute.timeout must be between 1 and {MAX_TERMINAL_TIMEOUT_MS} milliseconds."
            ));
        }
        validate_exact_deletion_target(&self, &[])?;
        Ok(self)
    }

    pub fn validate_protected_deletion_roots(
        &self,
        protected_roots: &[PathBuf],
    ) -> Result<(), String> {
        validate_exact_deletion_target(self, protected_roots)
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_TERMINAL_TIMEOUT_MS)
    }

    pub fn classification(&self) -> CapabilityClassification {
        if command_uses_compound_shell_syntax(self)
            || executable_key(&self.executable)
                .is_some_and(|key| SCRIPT_EXECUTORS.contains(&key.as_str()))
        {
            return CapabilityClassification::new(
                CapabilityRiskTier::SystemExec,
                "terminal command executes a shell, script, pipeline, redirection, or substitution",
            );
        }
        if executable_key(&self.executable).is_some_and(|key| NETWORK_CLIS.contains(&key.as_str()))
        {
            return CapabilityClassification::new(
                CapabilityRiskTier::Network,
                "terminal command can access a remote service, account, project, or data source",
            );
        }
        let classification =
            classify_system_action(SystemActionClass::Binary, &self.executable, &self.args);
        if !self.env.is_empty()
            && matches!(
                classification.tier,
                CapabilityRiskTier::ReadOnly | CapabilityRiskTier::FileRead
            )
        {
            return CapabilityClassification::new(
                CapabilityRiskTier::SystemExec,
                "terminal command changes its process environment",
            );
        }
        classification
    }

    pub fn prompt_free_in_project(&self, project_root: &Path) -> bool {
        if !matches!(
            self.classification().tier,
            CapabilityRiskTier::ReadOnly | CapabilityRiskTier::FileRead
        ) {
            return false;
        }
        let project_root = canonical_or_lexical(project_root);
        let cwd = self.resolved_cwd(&project_root);
        cwd.starts_with(&project_root)
            && self
                .argument_path_candidates(&cwd)
                .iter()
                .all(|path| canonical_or_lexical(path).starts_with(&project_root))
    }

    pub fn external_read_target(&self, project_root: &Path) -> Option<PathBuf> {
        if !matches!(
            self.classification().tier,
            CapabilityRiskTier::ReadOnly | CapabilityRiskTier::FileRead
        ) {
            return None;
        }
        let project_root = canonical_or_lexical(project_root);
        let cwd = self.resolved_cwd(&project_root);
        let argument_target = self
            .argument_path_candidates(&cwd)
            .into_iter()
            .map(|path| canonical_or_lexical(&path))
            .find(|path| !path.starts_with(&project_root));
        argument_target.or_else(|| (!cwd.starts_with(&project_root)).then_some(cwd))
    }

    pub fn display_command(&self) -> String {
        std::iter::once(self.executable.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_escape_for_display)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn environment_keys(&self) -> Vec<&str> {
        self.env.keys().map(String::as_str).collect()
    }

    fn resolved_cwd(&self, project_root: &Path) -> PathBuf {
        self.cwd
            .as_deref()
            .map(PathBuf::from)
            .map(|path| canonical_or_lexical(&path))
            .unwrap_or_else(|| project_root.to_path_buf())
    }

    fn argument_path_candidates(&self, cwd: &Path) -> Vec<PathBuf> {
        self.args
            .iter()
            .filter_map(|arg| {
                if arg == "--" {
                    return None;
                }
                let candidate = if arg.starts_with('-') {
                    let (_, value) = arg.split_once('=')?;
                    if value.is_empty() {
                        return None;
                    }
                    value
                } else {
                    arg.as_str()
                };
                let path = PathBuf::from(candidate);
                Some(if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                })
            })
            .collect()
    }
}

fn validate_exact_deletion_target(
    request: &NativeTerminalRequest,
    protected_roots: &[PathBuf],
) -> Result<(), String> {
    let Some(executable) = executable_key(&request.executable) else {
        return Ok(());
    };
    if !matches!(executable.as_str(), "rm" | "rmdir") {
        return Ok(());
    }
    let targets = request
        .args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err("A deletion command requires at least one exact target.".to_string());
    }
    let home = dirs::home_dir().map(|path| canonical_or_lexical(&path));
    for target in targets {
        let target_path = PathBuf::from(target);
        let resolved_target = if target_path.is_absolute() {
            target_path.clone()
        } else if let Some(cwd) = request.cwd.as_deref() {
            Path::new(cwd).join(&target_path)
        } else {
            target_path.clone()
        };
        let resolved = canonical_or_lexical(&resolved_target);
        let lexical_escape = target_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir));
        let unresolved = target.contains('$') || target == "~" || target.starts_with("~/");
        let broad = matches!(target.as_str(), "/" | "." | "..")
            || target.contains('*')
            || target.contains('?')
            || target.contains('[')
            || home.as_ref().is_some_and(|home| &resolved == home)
            || protected_roots
                .iter()
                .map(|root| canonical_or_lexical(root))
                .any(|root| resolved == root);
        if unresolved || lexical_escape || broad {
            return Err(
                "A deletion command must name an exact target and cannot remove a protected folder."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn command_uses_compound_shell_syntax(request: &NativeTerminalRequest) -> bool {
    contains_shell_control(&request.executable)
        || request.args.iter().any(|arg| contains_shell_control(arg))
}

fn contains_shell_control(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(
            character,
            '&' | '|' | ';' | '`' | '\n' | '\r' | '>' | '<' | '$'
        )
    })
}

fn contains_nul(value: &str) -> bool {
    value.as_bytes().contains(&0)
}

fn executable_key(executable: &str) -> Option<String> {
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

fn canonical_or_lexical(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let mut ancestor = path;
    let mut suffix = Vec::new();
    while let Some(file_name) = ancestor.file_name() {
        suffix.push(file_name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        ancestor = parent;
        if let Ok(mut canonical) = std::fs::canonicalize(ancestor) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return normalize_lexically(&canonical);
        }
    }
    normalize_lexically(path)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push("..");
                }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn shell_escape_for_display(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-._/:=@%+,".contains(character))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

const SCRIPT_EXECUTORS: &[&str] = &[
    "bash",
    "sh",
    "zsh",
    "python",
    "python3",
    "node",
    "osascript",
];
const NETWORK_CLIS: &[&str] = &[
    "aws", "az", "curl", "gcloud", "gh", "kubectl", "scp", "ssh", "wget",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_reads_are_prompt_free_but_external_reads_are_not() {
        let root = std::env::temp_dir().join("oomu-terminal-contract-project");
        std::fs::create_dir_all(&root).unwrap();
        let project = NativeTerminalRequest {
            executable: "git".to_string(),
            args: vec!["status".to_string(), "--short".to_string()],
            env: BTreeMap::new(),
            cwd: Some(root.display().to_string()),
            timeout: None,
        }
        .validate()
        .unwrap();
        assert!(project.prompt_free_in_project(&root));

        let external = NativeTerminalRequest {
            executable: "rg".to_string(),
            args: vec![
                "receipt".to_string(),
                "/private/tmp/attached.md".to_string(),
            ],
            env: BTreeMap::new(),
            cwd: Some(root.display().to_string()),
            timeout: None,
        }
        .validate()
        .unwrap();
        assert!(!external.prompt_free_in_project(&root));
        assert_eq!(
            external.external_read_target(&root),
            Some(PathBuf::from("/private/tmp/attached.md"))
        );
    }

    #[test]
    fn relative_traversal_and_external_git_roots_require_approval() {
        let root = std::env::temp_dir().join("oomu-terminal-contract-scoped-project");
        std::fs::create_dir_all(&root).unwrap();
        let request = |executable: &str, args: &[&str]| {
            NativeTerminalRequest {
                executable: executable.to_string(),
                args: args.iter().map(|value| value.to_string()).collect(),
                env: BTreeMap::new(),
                cwd: Some(root.display().to_string()),
                timeout: None,
            }
            .validate()
            .unwrap()
        };

        let parent_read = request("cat", &["../attached.md"]);
        assert!(!parent_read.prompt_free_in_project(&root));
        assert_eq!(
            parent_read.external_read_target(&root),
            root.parent()
                .map(|parent| canonical_or_lexical(&parent.join("attached.md")))
        );

        let external_git = request("git", &["-C", "../another-project", "status"]);
        assert!(!external_git.prompt_free_in_project(&root));
        assert_eq!(
            external_git.external_read_target(&root),
            root.parent()
                .map(|parent| canonical_or_lexical(&parent.join("another-project")))
        );
    }

    #[test]
    fn shell_network_and_delete_requests_fail_closed() {
        let request = |executable: &str, args: &[&str]| NativeTerminalRequest {
            executable: executable.to_string(),
            args: args.iter().map(|value| value.to_string()).collect(),
            env: BTreeMap::new(),
            cwd: None,
            timeout: None,
        };
        assert_eq!(
            request("sh", &["-lc", "pwd | tee /tmp/out"])
                .validate()
                .unwrap()
                .classification()
                .tier,
            CapabilityRiskTier::SystemExec
        );
        assert_eq!(
            request("gcloud", &["projects", "list"])
                .validate()
                .unwrap()
                .classification()
                .tier,
            CapabilityRiskTier::Network
        );
        assert!(request("rm", &["/tmp/*.md"]).validate().is_err());
        assert!(request("rm", &["-rf", "/"]).validate().is_err());
        assert!(request("rm", &["$HOME/report.md"]).validate().is_err());
        assert!(request("rm", &["../report.md"]).validate().is_err());
        let project = std::env::temp_dir().join("oomu-protected-project-root");
        let deletion_request = request("rm", &[project.to_str().unwrap()])
            .validate()
            .unwrap();
        assert!(deletion_request
            .validate_protected_deletion_roots(&[project])
            .is_err());

        for gated in [
            request("sh", &["-lc", "pwd"]),
            request("gcloud", &["projects", "list"]),
            request("touch", &["notes.md"]),
        ] {
            let gated = gated.validate().unwrap();
            assert!(!gated.prompt_free_in_project(Path::new("/tmp/project")));
        }
    }

    #[test]
    fn execution_bearing_read_flags_are_never_prompt_free() {
        let root = std::env::temp_dir().join("oomu-terminal-contract-read-flag-guard");
        std::fs::create_dir_all(&root).unwrap();
        let request = |executable: &str, args: &[&str]| {
            NativeTerminalRequest {
                executable: executable.to_string(),
                args: args.iter().map(|value| value.to_string()).collect(),
                env: BTreeMap::new(),
                cwd: Some(root.display().to_string()),
                timeout: None,
            }
            .validate()
            .unwrap()
        };

        for guarded in [
            request("rg", &["--pre=python3 helper.py", "term", "."]),
            request("git", &["-c", "diff.external=helper", "diff"]),
            request("git", &["diff", "--ext-diff"]),
            request("git", &["show", "--textconv"]),
            request("git", &["grep", "--open-files-in-pager=less", "term"]),
            request("git", &["diff", "--output=review.patch"]),
        ] {
            assert!(!guarded.prompt_free_in_project(&root));
        }

        assert!(request("rg", &["term", "."]).prompt_free_in_project(&root));
        assert!(request("git", &["status", "--short"]).prompt_free_in_project(&root));
    }
}
