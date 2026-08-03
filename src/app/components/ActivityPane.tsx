"use client";

import { invoke, isTauriRuntime } from "@/lib/invoke";
import { useState } from "react";
import { useAppContext } from "@/context/AppContext";

/**
 * How a command is pinged without triggering its side effects:
 *
 * - "live"   — pure state getter with no user arguments; invoked for real.
 * - "args"   — requires user arguments; invoked with `{}`. Tauri resolves the
 *              capability ACL *before* deserializing arguments, so an
 *              "invalid args" rejection proves the permission gate is open
 *              while the command body never runs. "not allowed" proves denial.
 * - "manual" — takes no user arguments but has side effects (dialogs,
 *              lockdown, identity rotation, remote calls). Never probed.
 */
type ProbeStrategy = "live" | "args" | "manual";

type BridgeCommand = {
  name: string;
  strategy: ProbeStrategy;
};

const args = (name: string): BridgeCommand => ({ name, strategy: "args" });
const live = (name: string): BridgeCommand => ({ name, strategy: "live" });
const manual = (name: string): BridgeCommand => ({ name, strategy: "manual" });

/**
 * Every command registered in `generate_handler!` (src-tauri/src/lib.rs).
 * Parity with the Rust side is enforced by src-tauri/tests/capability_parity.rs;
 * if a command is added or removed there, update this registry too.
 */
const COMMANDS: BridgeCommand[] = [
  // Workflow engine
  args("classify_chat_intent_route"),
  args("execute_action_plan"),
  args("execute_agent_action_plan"),
  args("execute_command"),
  args("execute_workflow"),
  args("process_agent_objective"),
  args("process_objective"),
  // Agent registry
  args("delete_agent_config"),
  args("delete_provider_config"),
  args("get_agent_config"),
  live("get_commander_state"),
  live("list_agent_configs"),
  live("list_provider_configs"),
  manual("restore_agent_sessions"),
  args("save_agent_config"),
  args("save_provider_config"),
  args("spawn_agent_session"),
  args("subagent_yield"),
  // Agent import
  manual("choose_agent_import_directory"),
  manual("choose_mod_package_path"),
  args("execute_agent_import"),
  args("scan_agent_import_directory"),
  // Taskflow system
  args("create_taskflow"),
  args("execute_taskflow"),
  live("get_taskflow_state"),
  args("inject_taskflow_override"),
  args("start_taskflow_monitor"),
  // Provider inference
  args("chat_turn"),
  args("execute_queued_messages"),
  args("record_browser_chat_turn"),
  args("sync_provider_models"),
  // Native inference core
  args("infer"),
  live("list_local_models"),
  args("parse_intent"),
  args("stream_native_inference"),
  // Context ingestion
  args("read_local_context"),
  // Knowledge store
  args("choose_knowledge_ingest_directory"),
  live("get_knowledge_state"),
  args("ingest_knowledge"),
  args("remove_knowledge_document"),
  // Memory ledger
  args("capture_agent_chat_memories"),
  args("compact_session_history"),
  args("commit_memory_proposal"),
  live("get_memory_ledger_state"),
  live("get_user_personality_profile"),
  args("hydrate_agent_prompt_context"),
  args("recall_global_memory"),
  args("run_memory_comparative_audit"),
  args("save_user_personality_profile"),
  args("update_agent_soul_manifest"),
  // Chat & persistence
  args("commit_chat_session_deletion"),
  args("create_chat_session"),
  args("delete_chat_session"),
  args("delete_workflow"),
  args("export_logical_certificate"),
  args("bind_mod_to_agent"),
  live("get_agentic_state"),
  args("get_agent_mods"),
  args("get_queued_messages"),
  live("get_recoverable_actions"),
  args("get_routing_preference"),
  live("get_sovereign_ledger_stats"),
  live("get_workflows"),
  args("install_mod_from_path"),
  args("list_chat_messages"),
  live("list_chat_sessions"),
  live("list_installed_mods"),
  args("queue_message"),
  args("save_agentic_state"),
  args("stage_chat_session_deletion"),
  args("save_workflow"),
  args("set_mod_active_state"),
  args("set_routing_preference"),
  args("undo_chat_session_deletion"),
  args("unbind_mod_to_agent"),
  args("uninstall_mod"),
  args("validate_mod_compatibility_for_turn"),
  // Secure identity core
  args("delegate_signing_authority"),
  manual("generate_node_identity"),
  live("get_sovereign_identity"),
  args("verify_artifact_signature"),
  // MCP bridge
  manual("mcp_connect_server"),
  manual("mcp_execute_tool"),
  args("mcp_get_tool_details"),
  args("mcp_list_tools"),
  args("mcp_search_tools"),
  args("execute_system_apple_app_tool"),
  args("prepare_system_apple_app_tool_approval"),
  args("read_system_calendar"),
  args("read_system_emails"),
  // Audit & artifacts
  args("analyze_visual_artifact"),
  args("audit_ark_artifacts"),
  live("get_launch_readiness"),
  live("get_system_hardware_profile"),
  args("run_pre_alpha_audit"),
  args("run_system_diagnostics"),
  // Settings
  manual("choose_local_model_directory"),
  live("get_local_model_directory"),
];

const PROBE_CONCURRENCY = 8;

type ProbeIssue = {
  name: string;
  detail: string;
};

type DiagnosticsState =
  | { phase: "idle" }
  | { phase: "running"; done: number; total: number }
  | { phase: "done"; checked: number; issues: ProbeIssue[]; finishedAt: Date };

function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
    try {
      return JSON.stringify(error);
    } catch {
      // fall through
    }
  }
  return String(error);
}

/** Returns an issue description, or null when the permission gate is open. */
function classifyFailure(command: BridgeCommand, message: string): ProbeIssue | null {
  const lower = message.toLowerCase();
  if (lower.includes("not allowed")) {
    return {
      name: command.name,
      detail: "Denied by capability ACL — no active allow-* permission in capabilities/default.json.",
    };
  }
  if (lower.includes(`command ${command.name} not found`)) {
    return {
      name: command.name,
      detail: "No handler registered — command is missing from generate_handler! in lib.rs.",
    };
  }
  // "invalid args" means the gate passed and the argument probe stopped before
  // execution; any other message means the command itself answered.
  return null;
}

async function probeCommand(command: BridgeCommand): Promise<ProbeIssue | null> {
  try {
    await invoke<unknown>(command.name, command.strategy === "args" ? {} : undefined);
    return null;
  } catch (error) {
    return classifyFailure(command, describeError(error));
  }
}

export function ActivityPane() {
  return (
    <div className="flex flex-col gap-4">
      <ConnectionSection />
    </div>
  );
}

function ConnectionSection() {
  const { isInitializing, isSecureEnvironment } = useAppContext();
  const [diagnostics, setDiagnostics] = useState<DiagnosticsState>({ phase: "idle" });

  const summary = isInitializing
    ? "Connecting to the secure environment..."
    : isSecureEnvironment
      ? "Everything is running normally."
      : "Some services may be unavailable.";

  async function runDiagnostics() {
    const pending = COMMANDS.filter((command) => command.strategy !== "manual");
    setDiagnostics({ phase: "running", done: 0, total: pending.length });

    const issues: ProbeIssue[] = [];
    const queue = [...pending];
    let done = 0;
    const workers = Array.from({ length: PROBE_CONCURRENCY }, async () => {
      for (;;) {
        const command = queue.shift();
        if (!command) return;
        const issue = await probeCommand(command);
        if (issue) {
          issues.push(issue);
        }
        done += 1;
        setDiagnostics({ phase: "running", done, total: pending.length });
      }
    });
    await Promise.all(workers);

    issues.sort((left, right) => left.name.localeCompare(right.name));
    setDiagnostics({ phase: "done", checked: pending.length, issues, finishedAt: new Date() });

  }

  const isRunning = diagnostics.phase === "running";

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2.5">
            <span
              className={`h-2 w-2 shrink-0 rounded-full ${
                isInitializing
                  ? "animate-pulse bg-[var(--warning)]"
                  : isSecureEnvironment
                    ? "bg-[var(--success)]"
                    : "bg-[var(--warning)]"
              }`}
            />
            <h2 className="text-sm font-semibold text-[var(--foreground)]">Connection</h2>
          </div>
          <p className="mt-1.5 text-sm leading-6 text-[var(--foreground-muted)]">{summary}</p>
        </div>
        <button
          className="shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:opacity-50"
          disabled={isRunning}
          onClick={() => void runDiagnostics()}
          type="button"
        >
          {isRunning ? "Checking..." : "Run Diagnostics"}
        </button>
      </div>

      {diagnostics.phase === "running" ? (
        <p className="mt-3 text-xs text-[var(--foreground-muted)]" aria-live="polite">
          Checking {diagnostics.done} of {diagnostics.total} services...
        </p>
      ) : null}

      {diagnostics.phase === "done" ? (
        diagnostics.issues.length === 0 ? (
          <p className="mt-3 text-xs text-[var(--foreground-muted)]" aria-live="polite">
            All {diagnostics.checked} checks passed at{" "}
            {diagnostics.finishedAt.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}.
          </p>
        ) : (
          <div className="mt-4">
            <p className="text-sm font-medium text-[var(--destructive)]" aria-live="polite">
              {diagnostics.issues.length} of {diagnostics.checked} checks failed.
            </p>
            <ul className="mt-2 divide-y divide-[var(--border-soft)] rounded-[var(--radius-md)] border border-[var(--border-soft)]">
              {diagnostics.issues.map((issue) => (
                <li className="px-3 py-2.5" key={issue.name}>
                  <code className="font-mono text-xs text-[var(--foreground)]">{issue.name}</code>
                  <p className="mt-0.5 text-xs leading-5 text-[var(--foreground-muted)]">{issue.detail}</p>
                </li>
              ))}
            </ul>
          </div>
        )
      ) : null}

      {!isTauriRuntime ? (
        <p className="mt-3 text-xs leading-5 text-[var(--foreground-subtle)]">
          Diagnostics in the browser test the development bridge, not the desktop app.
        </p>
      ) : null}
    </section>
  );
}
