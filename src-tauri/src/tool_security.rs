use crate::security::firewall::{
    audit_oomu_payload, audit_oomu_payload_segments, default_workspace_id, WorkspaceBoundaryAudit,
    WorkspaceBoundaryPayloadSegment, WorkspaceBoundaryViolation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use std::cell::Cell;
use std::path::Path;

#[cfg(test)]
thread_local! {
    static AUTO_APPROVE_MCP_TEST_DEPTH: Cell<usize> = Cell::new(0);
}

#[cfg(test)]
pub(crate) struct AutoApproveMcpTestGuard;

#[cfg(test)]
impl AutoApproveMcpTestGuard {
    pub(crate) fn enable() -> Self {
        AUTO_APPROVE_MCP_TEST_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

#[cfg(test)]
impl Drop for AutoApproveMcpTestGuard {
    fn drop(&mut self) {
        AUTO_APPROVE_MCP_TEST_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

#[cfg(test)]
pub(crate) fn auto_approve_mcp_test_enabled() -> bool {
    AUTO_APPROVE_MCP_TEST_DEPTH.with(|depth| depth.get() > 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityRiskTier {
    ReadOnly,
    FileRead,
    FileWrite,
    SystemExec,
    Network,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityClassification {
    pub tier: CapabilityRiskTier,
    pub reason: String,
}

impl CapabilityRiskTier {
    pub fn requires_human_approval(self) -> bool {
        !matches!(self, Self::ReadOnly | Self::FileRead)
    }

    pub fn requires_sandbox(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "READ_ONLY",
            Self::FileRead => "FILE_READ",
            Self::FileWrite => "FILE_WRITE",
            Self::SystemExec => "SYSTEM_EXEC",
            Self::Network => "NETWORK",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPolicy {
    pub required: bool,
    pub network_enabled: bool,
    pub reason: String,
}

pub(crate) fn audit_workspace_execution_payload(
    payload: &str,
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    let workspace_id = default_workspace_id();
    audit_oomu_payload(&workspace_id, payload)
}

pub(crate) fn audit_workspace_execution_payload_segments(
    segments: &[WorkspaceBoundaryPayloadSegment<'_>],
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    let workspace_id = default_workspace_id();
    audit_oomu_payload_segments(&workspace_id, segments)
}

impl CapabilityClassification {
    pub fn new(tier: CapabilityRiskTier, reason: impl Into<String>) -> Self {
        Self {
            tier,
            reason: reason.into(),
        }
    }

    pub fn requires_human_approval(&self) -> bool {
        #[cfg(test)]
        {
            if auto_approve_mcp_test_enabled() {
                return false;
            }
        }
        self.tier.requires_human_approval()
    }

    pub fn sandbox_policy(&self) -> SandboxPolicy {
        let required = self.tier.requires_sandbox();
        let reason = if required {
            format!("{}; routed through local code sandbox", self.reason)
        } else {
            format!("{}; native execution allowed", self.reason)
        };
        SandboxPolicy {
            required,
            network_enabled: false,
            reason,
        }
    }
}

pub fn classify_mcp_tool_call(
    server_name: &str,
    tool_name: &str,
    annotations: Option<&Value>,
) -> CapabilityClassification {
    let server_key = normalize_identifier(server_name);
    let tool_key = normalize_identifier(tool_name);
    let tokens = identifier_tokens(tool_name);

    if matches!(
        server_key.as_str(),
        "local_filesystem" | "filesystem" | "file_system"
    ) {
        if matches!(
            tool_key.as_str(),
            "list_directory" | "read_file" | "stat_file" | "search_files"
        ) {
            return CapabilityClassification::new(
                CapabilityRiskTier::ReadOnly,
                "filesystem MCP read-only tool is safe for automatic execution",
            );
        }
        if matches!(
            tool_key.as_str(),
            "write_file"
                | "delete_file"
                | "remove_file"
                | "move_file"
                | "copy_file"
                | "create_directory"
                | "delete_directory"
        ) {
            return CapabilityClassification::new(
                CapabilityRiskTier::FileWrite,
                "filesystem MCP write tool",
            );
        }
    }

    if matches!(
        server_key.as_str(),
        "local_search" | "web_search" | "search"
    ) {
        return CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "local search MCP tool performs outbound public web requests through an isolated profile",
        );
    }

    if server_key == "macos_applescript"
        && matches!(
            tool_key.as_str(),
            "read_apple_app_ui"
                | "read_system_calendar"
                | "read_system_contacts"
                | "read_system_emails"
                | "read_system_music"
                | "read_system_notes"
                | "read_system_photos"
                | "read_system_reminders"
        )
    {
        let reason = match tool_key.as_str() {
            "read_system_photos" => "native PhotoKit tool reads bounded local photo metadata",
            "read_system_music" => {
                "native MediaPlayer tool reads bounded local music library metadata"
            }
            _ => "macOS AppleScript MCP tool reads bounded local personal data",
        };
        return CapabilityClassification::new(CapabilityRiskTier::FileRead, reason);
    }

    if server_key == "macos_applescript" && tool_key == "trigger_system_notification" {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "native local notification changes visible system state",
        );
    }

    if server_key == "macos_applescript" && tool_key == "preview_camera" {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "native camera preview temporarily uses the Mac camera",
        );
    }

    if server_key == "macos_applescript"
        && matches!(
            tool_key.as_str(),
            "add_system_reminder"
                | "create_system_note"
                | "draft_system_email"
                | "prepare_system_message"
                | "send_system_email"
        )
    {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "macOS AppleScript MCP tool modifies local Apple app data",
        );
    }

    if has_any_token(&tokens, SYSTEM_EXEC_TOKENS) {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "MCP tool advertises process execution capability",
        );
    }

    if has_any_token(&tokens, FILE_WRITE_TOKENS) {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "MCP tool advertises modifying file or resource capability",
        );
    }

    if has_any_token(&tokens, NETWORK_TOKENS) {
        return CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "MCP tool advertises network or remote capability",
        );
    }

    // Remote-provided annotations are untrusted metadata. They may raise the
    // risk decision, but `readOnlyHint` can never lower it or waive consent.
    if annotation_bool(annotations, "destructiveHint") == Some(true) {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "MCP tool annotations declare destructive behavior",
        );
    }

    if annotation_bool(annotations, "openWorldHint") == Some(true) {
        return CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "MCP tool annotations declare open-world access",
        );
    }

    CapabilityClassification::new(
        CapabilityRiskTier::Unknown,
        "MCP tool capability is not explicitly classified",
    )
}

pub fn classify_system_action(
    action_type: SystemActionClass,
    command: &str,
    args: &[String],
) -> CapabilityClassification {
    match action_type {
        SystemActionClass::Python => {
            return CapabilityClassification::new(
                CapabilityRiskTier::SystemExec,
                "Python system action executes local code",
            );
        }
        SystemActionClass::Shell if contains_shell_control_syntax(command) => {
            return CapabilityClassification::new(
                CapabilityRiskTier::SystemExec,
                "Shell action contains control syntax",
            );
        }
        SystemActionClass::Shell => {
            let Some((binary, shell_args)) = shell_command_parts(command) else {
                return CapabilityClassification::new(
                    CapabilityRiskTier::SystemExec,
                    "Shell action could not be parsed into a single command",
                );
            };
            let shell_args = shell_args
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            classify_binary_command(binary, &shell_args)
        }
        SystemActionClass::Binary => classify_binary_command(command, args),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemActionClass {
    Shell,
    Python,
    Binary,
}

fn classify_binary_command(command: &str, args: &[String]) -> CapabilityClassification {
    let binary = command_basename(command);
    if binary.trim().is_empty() {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "System action command is empty",
        );
    }
    if !command_fragment_is_safe(command) || args.iter().any(|arg| !command_fragment_is_safe(arg)) {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "System action contains shell control syntax or unsafe characters",
        );
    }

    let binary_key = normalize_identifier(binary);
    match binary_key.as_str() {
        "echo" | "pwd" | "whoami" | "date" | "uname" => CapabilityClassification::new(
            CapabilityRiskTier::ReadOnly,
            "system command is a read-only diagnostic",
        ),
        "cat" | "head" | "tail" | "wc" | "ls" | "grep" => CapabilityClassification::new(
            CapabilityRiskTier::FileRead,
            "system command reads local filesystem state",
        ),
        "rg" => classify_rg_command(args),
        "find" => classify_find_command(args),
        "git" => classify_git_command(args),
        "curl" | "wget" | "ssh" | "scp" | "rsync" => CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "system command can access remote network resources",
        ),
        "npm" | "npx" | "pnpm" | "yarn" | "cargo" | "python" | "python3" | "node" | "bash"
        | "sh" | "zsh" | "osascript" | "open" => CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "system command executes scripts, packages, or macOS system actions",
        ),
        "rm" | "rmdir" | "mv" | "cp" | "mkdir" | "touch" | "tee" | "chmod" | "chown" | "plutil"
        | "defaults" => CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "system command can modify local files or settings",
        ),
        _ => CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "system command is not explicitly classified as read-only",
        ),
    }
}

fn classify_rg_command(args: &[String]) -> CapabilityClassification {
    if args.iter().any(|arg| {
        let arg = trim_command_quotes(arg);
        arg == "--pre" || arg.starts_with("--pre=")
    }) {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "ripgrep arguments can execute a preprocessing command",
        );
    }

    CapabilityClassification::new(
        CapabilityRiskTier::FileRead,
        "ripgrep reads local filesystem state",
    )
}

fn classify_find_command(args: &[String]) -> CapabilityClassification {
    if args
        .iter()
        .filter_map(|arg| find_predicate_key(arg))
        .any(|key| FIND_SYSTEM_EXEC_PREDICATES.contains(&key.as_str()))
    {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "find command can execute local commands through its arguments",
        );
    }

    if args
        .iter()
        .filter_map(|arg| find_predicate_key(arg))
        .any(|key| FIND_FILE_WRITE_PREDICATES.contains(&key.as_str()))
    {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "find command can modify or write local filesystem state through its arguments",
        );
    }

    if args
        .iter()
        .filter_map(|arg| find_predicate_key(arg))
        .any(|key| find_predicate_requires_file_write_fallback(&key))
    {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "find command includes an unclassified predicate that may write local filesystem state",
        );
    }

    CapabilityClassification::new(
        CapabilityRiskTier::FileRead,
        "find command arguments are limited to filesystem traversal or filtering",
    )
}

fn find_predicate_key(arg: &str) -> Option<String> {
    let trimmed = trim_command_quotes(arg);
    if !trimmed.starts_with('-') {
        return None;
    }
    Some(normalize_identifier(trimmed.trim_start_matches('-')))
}

fn find_predicate_requires_file_write_fallback(key: &str) -> bool {
    if FIND_SYSTEM_EXEC_PREDICATES.contains(&key)
        || FIND_FILE_WRITE_PREDICATES.contains(&key)
        || FIND_READ_ONLY_PREDICATES.contains(&key)
    {
        return false;
    }

    if key.starts_with('f') && key != "follow" {
        return true;
    }

    FIND_MUTATING_PREDICATE_HINTS
        .iter()
        .any(|hint| key.contains(hint))
}

fn classify_git_command(args: &[String]) -> CapabilityClassification {
    if args.iter().any(|arg| git_arg_writes_output(arg)) {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "git arguments can write command output to a file",
        );
    }
    if args.iter().any(|arg| git_arg_can_execute_helper(arg)) {
        return CapabilityClassification::new(
            CapabilityRiskTier::SystemExec,
            "git arguments can execute a configured helper, pager, filter, or external command",
        );
    }
    let Some((subcommand_index, subcommand)) = git_subcommand(args) else {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "git command has no explicit read-only subcommand",
        );
    };
    if matches!(
        subcommand.as_str(),
        "fetch" | "pull" | "push" | "clone" | "ls_remote"
    ) {
        return CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "git subcommand can access remote repositories",
        );
    }

    if subcommand == "branch" {
        return classify_git_branch_command(args, subcommand_index);
    }

    if subcommand == "remote" {
        return classify_git_remote_command(args, subcommand_index);
    }

    if matches!(
        subcommand.as_str(),
        "add"
            | "am"
            | "apply"
            | "bisect"
            | "checkout"
            | "cherry_pick"
            | "clean"
            | "commit"
            | "merge"
            | "mv"
            | "rebase"
            | "reset"
            | "restore"
            | "revert"
            | "rm"
            | "stash"
            | "switch"
            | "tag"
            | "write_tree"
    ) {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "git subcommand can modify the working tree or repository",
        );
    }

    if matches!(
        subcommand.as_str(),
        "status" | "diff" | "log" | "show" | "rev_parse" | "ls_files" | "grep"
    ) {
        return CapabilityClassification::new(
            CapabilityRiskTier::ReadOnly,
            "git read-only subcommand",
        );
    }

    CapabilityClassification::new(
        CapabilityRiskTier::FileWrite,
        "git subcommand is not explicitly classified as read-only",
    )
}

fn git_arg_writes_output(arg: &str) -> bool {
    let arg = trim_command_quotes(arg);
    arg == "--output" || arg.starts_with("--output=")
}

fn git_arg_can_execute_helper(arg: &str) -> bool {
    let arg = trim_command_quotes(arg);
    arg == "-c"
        || (arg.starts_with("-c") && arg.len() > 2)
        || arg == "-p"
        || arg == "--paginate"
        || arg == "--config-env"
        || arg.starts_with("--config-env=")
        || arg == "--exec-path"
        || arg.starts_with("--exec-path=")
        || arg == "--ext-diff"
        || arg == "--textconv"
        || arg == "--open-files-in-pager"
        || arg.starts_with("--open-files-in-pager=")
        || arg == "-O"
        || (arg.starts_with("-O") && arg.len() > 2)
        || arg == "--help"
}

fn classify_git_branch_command(
    args: &[String],
    subcommand_index: usize,
) -> CapabilityClassification {
    let mut skip_next_read_only_value = false;
    for arg in args.iter().skip(subcommand_index + 1) {
        let trimmed = trim_command_quotes(arg);
        if skip_next_read_only_value {
            skip_next_read_only_value = false;
            continue;
        }

        if git_branch_arg_is_mutating(trimmed) {
            return CapabilityClassification::new(
                CapabilityRiskTier::FileWrite,
                "git branch arguments can delete or rewrite repository refs",
            );
        }

        if git_branch_read_only_arg_takes_value(trimmed) {
            skip_next_read_only_value = true;
            continue;
        }

        if trimmed.starts_with('-') {
            continue;
        }

        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "git branch can create repository refs when given a branch target",
        );
    }

    CapabilityClassification::new(CapabilityRiskTier::ReadOnly, "git branch read-only listing")
}

fn git_branch_arg_is_mutating(arg: &str) -> bool {
    let key = normalize_identifier(arg.trim_start_matches('-'));
    matches!(
        key.as_str(),
        "d" | "delete"
            | "force_delete"
            | "m"
            | "move"
            | "force_move"
            | "c"
            | "copy"
            | "force_copy"
            | "set_upstream_to"
            | "unset_upstream"
            | "edit_description"
    ) || (!arg.starts_with("--")
        && arg.starts_with('-')
        && arg.chars().any(|ch| ch == 'd' || ch == 'D'))
}

fn git_branch_read_only_arg_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--contains"
            | "--no-contains"
            | "--points-at"
            | "--merged"
            | "--no-merged"
            | "--format"
            | "--sort"
            | "--list"
    )
}

fn classify_git_remote_command(
    args: &[String],
    subcommand_index: usize,
) -> CapabilityClassification {
    let action = args
        .iter()
        .skip(subcommand_index + 1)
        .find(|arg| !trim_command_quotes(arg).starts_with('-'))
        .map(|arg| normalize_identifier(trim_command_quotes(arg)))
        .unwrap_or_default();

    if action.is_empty() {
        return CapabilityClassification::new(CapabilityRiskTier::ReadOnly, "git remote listing");
    }

    if matches!(action.as_str(), "show" | "update") {
        return CapabilityClassification::new(
            CapabilityRiskTier::Network,
            "git remote action can access remote repositories",
        );
    }

    if matches!(
        action.as_str(),
        "add"
            | "remove"
            | "rm"
            | "delete"
            | "rename"
            | "set_url"
            | "set_branches"
            | "set_head"
            | "prune"
    ) {
        return CapabilityClassification::new(
            CapabilityRiskTier::FileWrite,
            "git remote arguments can modify repository remote configuration or refs",
        );
    }

    if matches!(action.as_str(), "get_url") {
        return CapabilityClassification::new(
            CapabilityRiskTier::ReadOnly,
            "git remote get-url reads repository remote configuration",
        );
    }

    CapabilityClassification::new(
        CapabilityRiskTier::FileWrite,
        "git remote action is not explicitly classified as read-only",
    )
}

fn git_subcommand(args: &[String]) -> Option<(usize, String)> {
    let mut skip_next = false;
    for (index, arg) in args.iter().enumerate() {
        let trimmed = trim_command_quotes(arg);
        if skip_next {
            skip_next = false;
            continue;
        }
        if git_global_arg_takes_value(trimmed) {
            skip_next = true;
            continue;
        }
        if git_global_arg_contains_value(trimmed) {
            continue;
        }
        if trimmed.starts_with('-') {
            continue;
        }
        return Some((index, normalize_identifier(trimmed)));
    }
    None
}

fn git_global_arg_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-C" | "-c"
            | "--git-dir"
            | "--work-tree"
            | "--namespace"
            | "--exec-path"
            | "--super-prefix"
    )
}

fn git_global_arg_contains_value(arg: &str) -> bool {
    [
        "--git-dir=",
        "--work-tree=",
        "--namespace=",
        "--exec-path=",
        "--super-prefix=",
    ]
    .iter()
    .any(|prefix| arg.starts_with(prefix))
}

fn shell_command_parts(command: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = command.split_whitespace();
    let binary = parts.next()?;
    let binary = binary.trim_matches(['"', '\'']);
    Some((command_basename(binary), parts.collect()))
}

fn trim_command_quotes(value: &str) -> &str {
    value.trim_matches(['"', '\''])
}

fn command_basename(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

fn command_fragment_is_safe(value: &str) -> bool {
    !contains_shell_control_syntax(value) && value.chars().all(command_fragment_char_is_allowed)
}

fn contains_shell_control_syntax(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '&' | '|' | ';' | '`' | '\n' | '\r' | '\t' | '>' | '<' | '$'
        )
    })
}

fn command_fragment_char_is_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            ' ' | '-'
                | '_'
                | '.'
                | '/'
                | ':'
                | '='
                | ','
                | '@'
                | '%'
                | '+'
                | '*'
                | '?'
                | '"'
                | '\''
        )
}

fn annotation_bool(annotations: Option<&Value>, key: &str) -> Option<bool> {
    annotations
        .and_then(Value::as_object)
        .and_then(|object| object.get(key))
        .and_then(Value::as_bool)
}

fn normalize_identifier(value: &str) -> String {
    identifier_tokens(value).join("_")
}

fn identifier_tokens(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn has_any_token(tokens: &[String], candidates: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| candidates.iter().any(|candidate| token == candidate))
}

const FILE_WRITE_TOKENS: &[&str] = &[
    "apply",
    "copy",
    "create",
    "delete",
    "edit",
    "move",
    "patch",
    "provision",
    "remove",
    "rename",
    "sync",
    "update",
    "upload",
    "write",
];

const SYSTEM_EXEC_TOKENS: &[&str] = &[
    "applescript",
    "command",
    "exec",
    "execute",
    "osascript",
    "process",
    "python",
    "script",
    "shell",
    "terminal",
];

const NETWORK_TOKENS: &[&str] = &[
    "api",
    "deploy",
    "download",
    "fetch",
    "http",
    "network",
    "remote",
    "request",
    "rsync",
    "scp",
    "ssh",
    "terraform",
    "url",
    "web",
];

const FIND_SYSTEM_EXEC_PREDICATES: &[&str] = &["exec", "execdir", "ok", "okdir"];

const FIND_FILE_WRITE_PREDICATES: &[&str] =
    &["delete", "fls", "fprint", "fprint0", "fprintf", "write"];

const FIND_READ_ONLY_PREDICATES: &[&str] = &[
    "a",
    "amin",
    "and",
    "anewer",
    "atime",
    "cmin",
    "cnewer",
    "ctime",
    "daystart",
    "depth",
    "empty",
    "false",
    "flags",
    "follow",
    "fstype",
    "gid",
    "group",
    "ilname",
    "iname",
    "inum",
    "ipath",
    "iregex",
    "iwholename",
    "links",
    "lname",
    "ls",
    "maxdepth",
    "mindepth",
    "mmin",
    "mount",
    "mtime",
    "name",
    "newer",
    "newerxy",
    "nogroup",
    "not",
    "nouser",
    "o",
    "or",
    "path",
    "perm",
    "print",
    "print0",
    "prune",
    "readable",
    "regex",
    "samefile",
    "size",
    "true",
    "type",
    "uid",
    "user",
    "wholename",
    "xdev",
    "xtype",
];

const FIND_MUTATING_PREDICATE_HINTS: &[&str] = &[
    "append", "create", "delete", "output", "remove", "save", "truncate", "unlink", "write",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_mcp_tools_into_security_tiers() {
        assert_eq!(
            classify_mcp_tool_call("local_filesystem", "read_file", None).tier,
            CapabilityRiskTier::ReadOnly
        );
        assert!(
            !classify_mcp_tool_call("local_filesystem", "list_directory", None)
                .requires_human_approval()
        );
        assert_eq!(
            classify_mcp_tool_call("local_filesystem", "write_file", None).tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_mcp_tool_call("local_filesystem", "delete_file", None).tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_mcp_tool_call("remote_shell", "execute_command", None).tier,
            CapabilityRiskTier::SystemExec
        );
        assert_eq!(
            classify_mcp_tool_call("local_search", "search_web", None).tier,
            CapabilityRiskTier::Network
        );
        let read_mail = classify_mcp_tool_call("macos_applescript", "read_system_emails", None);
        assert_eq!(read_mail.tier, CapabilityRiskTier::FileRead);
        assert!(!read_mail.requires_human_approval());
        let read_calendar =
            classify_mcp_tool_call("macos_applescript", "read_system_calendar", None);
        assert_eq!(read_calendar.tier, CapabilityRiskTier::FileRead);
        assert!(!read_calendar.requires_human_approval());
        let read_reminders =
            classify_mcp_tool_call("macos_applescript", "read_system_reminders", None);
        assert_eq!(read_reminders.tier, CapabilityRiskTier::FileRead);
        assert!(!read_reminders.requires_human_approval());
        let read_notes = classify_mcp_tool_call("macos_applescript", "read_system_notes", None);
        assert_eq!(read_notes.tier, CapabilityRiskTier::FileRead);
        assert!(!read_notes.requires_human_approval());
        let read_contacts =
            classify_mcp_tool_call("macos_applescript", "read_system_contacts", None);
        assert_eq!(read_contacts.tier, CapabilityRiskTier::FileRead);
        assert!(!read_contacts.requires_human_approval());
        let read_photos = classify_mcp_tool_call("macos_applescript", "read_system_photos", None);
        assert_eq!(read_photos.tier, CapabilityRiskTier::FileRead);
        assert!(!read_photos.requires_human_approval());
        assert!(read_photos.reason.contains("PhotoKit"));
        let read_music = classify_mcp_tool_call("macos_applescript", "read_system_music", None);
        assert_eq!(read_music.tier, CapabilityRiskTier::FileRead);
        assert!(!read_music.requires_human_approval());
        assert!(read_music.reason.contains("MediaPlayer"));
        let read_ui = classify_mcp_tool_call("macos_applescript", "read_apple_app_ui", None);
        assert_eq!(read_ui.tier, CapabilityRiskTier::FileRead);
        assert!(!read_ui.requires_human_approval());
        let notification =
            classify_mcp_tool_call("macos_applescript", "trigger_system_notification", None);
        assert_eq!(notification.tier, CapabilityRiskTier::FileWrite);
        assert!(notification.requires_human_approval());

        let camera = classify_mcp_tool_call("macos_applescript", "preview_camera", None);
        assert_eq!(camera.tier, CapabilityRiskTier::SystemExec);
        assert!(camera.requires_human_approval());
        let add_reminder = classify_mcp_tool_call("macos_applescript", "add_system_reminder", None);
        assert_eq!(add_reminder.tier, CapabilityRiskTier::FileWrite);
        assert!(add_reminder.requires_human_approval());
        let draft_mail = classify_mcp_tool_call("macos_applescript", "draft_system_email", None);
        assert_eq!(draft_mail.tier, CapabilityRiskTier::FileWrite);
        assert!(draft_mail.requires_human_approval());
        let create_note = classify_mcp_tool_call("macos_applescript", "create_system_note", None);
        assert_eq!(create_note.tier, CapabilityRiskTier::FileWrite);
        assert!(create_note.requires_human_approval());
        assert_eq!(
            classify_mcp_tool_call("unknown", "do_thing", None).tier,
            CapabilityRiskTier::Unknown
        );
        assert_eq!(
            classify_mcp_tool_call(
                "remote_attacker",
                "do_thing",
                Some(&serde_json::json!({"readOnlyHint": true}))
            )
            .tier,
            CapabilityRiskTier::Unknown,
            "server-controlled readOnlyHint must not lower native risk"
        );
        let read_like = classify_mcp_tool_call(
            "remote_attacker",
            "read_list_status",
            Some(&serde_json::json!({"readOnlyHint": true})),
        );
        assert_eq!(
            read_like.tier,
            CapabilityRiskTier::Unknown,
            "an attacker-selected read-like name must not become a native allowlist"
        );
        assert!(read_like.requires_human_approval());
        assert_eq!(
            classify_mcp_tool_call(
                "remote_attacker",
                "read_data",
                Some(&serde_json::json!({"destructiveHint": true}))
            )
            .tier,
            CapabilityRiskTier::FileWrite,
            "server annotations may only increase native risk"
        );
    }

    #[test]
    fn classifies_system_commands_into_security_tiers() {
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "git", &["status".to_string()]).tier,
            CapabilityRiskTier::ReadOnly
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &["write-tree".to_string()]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "npm", &["test".to_string()]).tier,
            CapabilityRiskTier::SystemExec
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Shell, "echo ok && rm -rf /", &[]).tier,
            CapabilityRiskTier::SystemExec
        );
    }

    #[test]
    fn read_only_command_names_do_not_hide_subprocess_or_write_flags() {
        for args in [
            vec![
                "--pre".to_string(),
                "sh -c whoami".to_string(),
                ".".to_string(),
            ],
            vec!["--pre=python3 helper.py".to_string(), ".".to_string()],
        ] {
            assert_eq!(
                classify_system_action(SystemActionClass::Binary, "rg", &args).tier,
                CapabilityRiskTier::SystemExec,
            );
        }

        for args in [
            vec![
                "-c".to_string(),
                "diff.external=helper".to_string(),
                "diff".to_string(),
            ],
            vec!["-cdiff.external=helper".to_string(), "diff".to_string()],
            vec![
                "--config-env=diff.external=OOMU_HELPER".to_string(),
                "diff".to_string(),
            ],
            vec!["diff".to_string(), "--ext-diff".to_string()],
            vec!["show".to_string(), "--textconv".to_string()],
            vec![
                "grep".to_string(),
                "--open-files-in-pager=less".to_string(),
                "term".to_string(),
            ],
            vec!["--paginate".to_string(), "status".to_string()],
        ] {
            assert_eq!(
                classify_system_action(SystemActionClass::Binary, "git", &args).tier,
                CapabilityRiskTier::SystemExec,
            );
        }

        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &["diff".to_string(), "--output=review.patch".to_string()],
            )
            .tier,
            CapabilityRiskTier::FileWrite,
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &["status".to_string(), "--short".to_string()],
            )
            .tier,
            CapabilityRiskTier::ReadOnly,
        );
    }

    #[test]
    fn find_arguments_subclassify_mutating_predicates() {
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[".".to_string(), "-name".to_string(), "*.log".to_string()]
            )
            .tier,
            CapabilityRiskTier::FileRead
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Shell, "find . -name \"*.log\"", &[]).tier,
            CapabilityRiskTier::FileRead
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-name".to_string(),
                    "*.log".to_string(),
                    "-delete".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-fprint".to_string(),
                    "matches.txt".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-fls".to_string(),
                    "matches.txt".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-fprintf".to_string(),
                    "matches.txt".to_string(),
                    "%p".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-fwrite".to_string(),
                    "matches.txt".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[
                    ".".to_string(),
                    "-follow".to_string(),
                    "-name".to_string(),
                    "*.rs".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileRead
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "find",
                &[".".to_string(), "-exec".to_string(), "true".to_string()]
            )
            .tier,
            CapabilityRiskTier::SystemExec
        );
    }

    #[test]
    fn maps_security_tiers_to_sandbox_policies() {
        assert!(
            !classify_system_action(SystemActionClass::Binary, "date", &[])
                .sandbox_policy()
                .required
        );
        assert!(
            classify_system_action(SystemActionClass::Python, "script.py", &[])
                .sandbox_policy()
                .required
        );
        assert!(
            classify_system_action(SystemActionClass::Binary, "cat", &["secret.txt".into()])
                .sandbox_policy()
                .required
        );
        assert!(
            !classify_system_action(SystemActionClass::Binary, "cat", &["secret.txt".into()])
                .requires_human_approval(),
            "read-only filesystem commands should be sandboxed without requiring a manual click"
        );
        assert!(
            classify_system_action(
                SystemActionClass::Binary,
                "curl",
                &["https://example.com".into()]
            )
            .sandbox_policy()
            .required
        );
        assert!(
            !classify_system_action(SystemActionClass::Binary, "npm", &["test".into()])
                .sandbox_policy()
                .network_enabled
        );
    }

    #[test]
    fn workspace_audit_blocks_eldris_credential_requests() {
        let violation = audit_workspace_execution_payload("Eldris database credentials")
            .expect_err("Eldris credential payload must be blocked");

        assert!(violation.message.contains("Cognitive boundary rejected"));
    }

    #[test]
    fn workspace_audit_allows_unscoped_brand_mentions() {
        let audit =
            audit_workspace_execution_payload("Compare the Eldris and OOMU names as brand copy.")
                .expect("unscoped brand mention is allowed");

        assert_eq!(audit.status, "allowed");
    }

    #[test]
    fn git_arguments_subclassify_mutating_subcommands() {
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "git", &["branch".to_string()]).tier,
            CapabilityRiskTier::ReadOnly
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Shell, "git branch -D test", &[]).tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &["branch".to_string(), "-d".to_string(), "test".to_string()]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "git", &["remote".to_string()]).tier,
            CapabilityRiskTier::ReadOnly
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &[
                    "remote".to_string(),
                    "remove".to_string(),
                    "origin".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(
                SystemActionClass::Binary,
                "git",
                &[
                    "remote".to_string(),
                    "set-url".to_string(),
                    "origin".to_string()
                ]
            )
            .tier,
            CapabilityRiskTier::FileWrite
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "git", &["push".to_string()]).tier,
            CapabilityRiskTier::Network
        );
        assert_eq!(
            classify_system_action(SystemActionClass::Binary, "git", &["commit".to_string()]).tier,
            CapabilityRiskTier::FileWrite
        );
    }
}
