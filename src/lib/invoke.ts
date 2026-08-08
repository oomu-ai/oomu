import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
  redactSensitiveText,
  redactSensitiveValue,
  safeErrorMessage,
} from "@/lib/redaction";

export const isTauriRuntime =
  typeof window !== "undefined" &&
  ("__TAURI_IPC__" in window || "__TAURI_INTERNALS__" in window);

/**
 * Error thrown when a Tauri command (native runtime)
 * fails. `.message` always carries a human-readable description so UI surfaces,
 * `console`, and the Next.js dev overlay never collapse to an opaque "{}" — the
 * symptom you get when a raw Tauri rejection object (or an Error instance, whose
 * fields are non-enumerable) is logged/stringified directly. A recursively
 * redacted copy of the rejection is preserved on `.detail` for debugging.
 */
class InvokeError extends Error {
  readonly command: string;
  readonly detail: unknown;
  readonly code?: string;

  constructor(command: string, message: string, detail?: unknown, code?: string) {
    super(message);
    this.name = "InvokeError";
    this.command = command;
    this.detail = detail;
    this.code = code;
  }
}

const HANDLED_OPERATIONAL_ERROR_CODES = new Set([
  "credential_unavailable",
  "provider_network_error",
  "provider_stream_interrupted_after_tokens",
  "provider_stream_duration_exceeded",
  "inference_retry_exhausted",
  "provider_rate_limited",
  "provider_response_error",
  "chat_turn_already_running",
  "chat_turn_persistence_failed",
  "planner_output_unusable",
  "planner_objective_too_large",
  "planner_prompt_compilation_failed",
  "project_provider_blocked",
  "project_provider_consent_required",
  "project_provider_confirmation_invalid",
  "local_model_primary_gguf_ambiguous",
  "setup_storage_recovery_required",
  "search_not_authorized",
  "search_query_invalid",
  "search_unavailable",
]);

const EXPECTED_CONTROL_FLOW_ERROR_CODES = new Set([
  "agent_objective_not_executable",
  "private_egress_confirmation_required",
]);

/** Pull the most useful human-readable string out of an arbitrary error value. */
function describeInvokeError(error: unknown): string {
  if (typeof error === "string") {
    return redactSensitiveText(error.trim());
  }

  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    const message =
      typeof record.message === "string" ? record.message.trim() : "";
    const code = typeof record.code === "string" ? record.code.trim() : "";

    if (message && code && !message.includes(code)) {
      return redactSensitiveText(`${message} (${code})`);
    }
    if (message) {
      return redactSensitiveText(message);
    }
    if (code) {
      return code;
    }

    return safeErrorMessage(error, "");
  }

  return "";
}

function extractInvokeErrorCode(error: unknown, message: string): string | undefined {
  if (typeof error === "string") {
    try {
      return extractInvokeErrorCode(JSON.parse(error), message);
    } catch {
      const value = error.trim();
      if (/^[a-z][a-z0-9_]*$/iu.test(value)) return value;
    }
  }

  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.code === "string" && record.code.trim()) {
      return record.code.trim();
    }
  }

  const codeMatch = message.match(/\(([a-z][a-z0-9_]*?)\)\s*$/i);
  return codeMatch?.[1];
}

function isExpectedCancellation(code: string | undefined, message: string): boolean {
  return code === "local_inference_cancelled" || message.includes("local_inference_cancelled");
}

function isHandledOperationalError(code: string | undefined): boolean {
  return Boolean(
    code &&
      (HANDLED_OPERATIONAL_ERROR_CODES.has(code) ||
        code.startsWith("model_install_") ||
        code.startsWith("application_update_") ||
        code.startsWith("external_url_")),
  );
}

function isExpectedControlFlowError(code: string | undefined): boolean {
  return Boolean(code && EXPECTED_CONTROL_FLOW_ERROR_CODES.has(code));
}

function expectedControlFlowMessage(code: string | undefined) {
  return code === "agent_objective_not_executable"
    ? "the request does not need an action plan."
    : "an expected confirmation or routing decision is required.";
}

function operationalWarningMessage(code: string | undefined, message: string) {
  if (
    code === "planner_output_unusable" ||
    code === "planner_objective_too_large" ||
    code === "planner_prompt_compilation_failed"
  ) {
    return "OOMU could not prepare a safe action plan.";
  }
  if (code === "setup_storage_recovery_required") {
    return "OOMU must finish secure storage recovery before setup can continue.";
  }
  if (
    code === "search_not_authorized" ||
    code === "search_query_invalid" ||
    code === "search_unavailable"
  ) {
    return "Web search could not complete.";
  }
  return message;
}

export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauriRuntime) {
    try {
      return await tauriInvoke<T>(command, args);
    } catch (e) {
      const message =
        describeInvokeError(e) ||
        `Command "${command}" failed without a message — check the terminal running \`tauri dev\` for the backend error.`;
      const code = extractInvokeErrorCode(e, message);
      // Log a readable string (not the raw object) so the dev overlay shows the
      // actual error instead of "{}".
      if (isExpectedCancellation(code, message)) {
        console.debug(`Tauri invoke cancelled for command "${command}": ${message}`);
      } else if (isExpectedControlFlowError(code)) {
        console.debug(
          `Tauri invoke control flow for command "${command}": ${expectedControlFlowMessage(code)}`,
        );
      } else if (isHandledOperationalError(code)) {
        console.warn(
          `Tauri invoke warning for command "${command}": ${operationalWarningMessage(code, message)}`,
        );
      } else {
        console.error(`Tauri invoke failed for command "${command}": ${message}`);
      }
      // Re-throw a normalized error so the native application behaves correctly
      // and database errors are visible to callers via `.message`.
      throw new InvokeError(command, message, redactSensitiveValue(e), code);
    }
  }

  throw new InvokeError(
    command,
    `Tauri command "${command}" requires the native desktop runtime. Run \`npm run dev\` to launch it.`,
    undefined,
    "native_runtime_required",
  );
}
