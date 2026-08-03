"use client";

import { useEffect, useRef } from "react";
import { useI18n } from "@/context/I18nContext";
import type { MissingCapabilityDetail } from "./workflowCapabilityCatalog";
import {
  composerNoticeClasses,
  normalizeNoticeCapabilities,
  type NoticeCapability,
} from "./workflowComposerNotice";
import {
  type CompiledInstruction,
  type WorkflowPreflightMode,
} from "./workflowPersistence";
import { WorkflowDeveloperFields } from "./WorkflowStoryboard";
import type { WorkflowIr } from "./workflowIr";
import { useWorkflowRun, type WorkflowRunProgress } from "./useWorkflowRun";
import { formatWorkflowOutput } from "./WorkflowRunReport";

export type ComposerNotice = {
  action?: ComposerNoticeAction;
  missingCapabilities?: string[];
  missingCapabilityDetails?: MissingCapabilityDetail[];
  message: string;
  title?: string;
  tone: "info" | "success" | "warning" | "error";
};

export type ComposerNoticeAction = {
  kind: "insert_report_save_step";
  label: string;
};

type InspectDrawerProps = {
  compiledInstructions: CompiledInstruction[];
  instructionDrafts: Record<string, string>;
  lastRun: ReturnType<typeof useWorkflowRun>["lastRun"];
  onClose: () => void;
  onDraftChange: (value: Record<string, string>) => void;
  onPreflightModeChange: (mode: WorkflowPreflightMode) => void;
  onSaveInstruction: (instruction: CompiledInstruction) => void;
  onWorkflowIrChange: (workflowIr: WorkflowIr) => void;
  preflightMode: WorkflowPreflightMode;
  savingInstructionId: string | null;
  workflowIr?: WorkflowIr;
};

export function InspectDrawer({
  compiledInstructions,
  instructionDrafts,
  lastRun,
  onClose,
  onDraftChange,
  onPreflightModeChange,
  onSaveInstruction,
  onWorkflowIrChange,
  preflightMode,
  savingInstructionId,
  workflowIr,
}: InspectDrawerProps) {
  const { t } = useI18n();
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  return (
    <div
      aria-labelledby="workflow-inspect-title"
      aria-modal="true"
      className="fixed inset-0 z-40 flex justify-end bg-black/20"
      role="dialog"
    >
      <button
        aria-label={t("common.close")}
        className="absolute inset-0 cursor-default"
        onClick={onClose}
        type="button"
      />
      <aside className="relative z-10 flex h-full w-full max-w-xl flex-col border-l border-[var(--border-strong)] bg-[var(--background)] shadow-[var(--shadow-raised)]">
        <header className="flex shrink-0 items-center justify-between border-b border-[var(--border-soft)] px-4 py-3">
          <div>
            <h2
              className="text-sm font-semibold text-[var(--foreground)]"
              id="workflow-inspect-title"
            >
              {t("workflows.inspect.title")}
            </h2>
            <p className="mt-1 text-xs text-[var(--foreground-muted)]">
              {t("workflows.inspect.description")}
            </p>
          </div>
          <button
            className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-1.5 text-xs font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)]"
            onClick={onClose}
            ref={closeButtonRef}
            type="button"
          >
            {t("common.close")}
          </button>
        </header>
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4">
          {workflowIr && (
            <WorkflowDeveloperFields
              onWorkflowIrChange={onWorkflowIrChange}
              workflowIr={workflowIr}
            />
          )}

          <InspectPreflightSection
            onPreflightModeChange={onPreflightModeChange}
            preflightMode={preflightMode}
          />
          <CompiledInstructionsSection
            compiledInstructions={compiledInstructions}
            instructionDrafts={instructionDrafts}
            onDraftChange={onDraftChange}
            onSaveInstruction={onSaveInstruction}
            savingInstructionId={savingInstructionId}
          />
          <ThoughtTrackSection lastRun={lastRun} />
        </div>
      </aside>
    </div>
  );
}

function InspectPreflightSection({
  onPreflightModeChange,
  preflightMode,
}: Pick<InspectDrawerProps, "onPreflightModeChange" | "preflightMode">) {
  const { t } = useI18n();
  return (
    <section>
      <h3 className="text-xs font-semibold uppercase text-[var(--foreground-subtle)]">
        {t("workflows.inspect.preflight")}
      </h3>
      <div className="mt-2 inline-flex overflow-hidden rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)]">
        {(["skipped", "taskflow_audit"] as const).map((mode) => (
          <button
            aria-pressed={preflightMode === mode}
            className={`px-3 py-2 text-xs font-semibold transition-colors ${
              preflightMode === mode
                ? "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                : "text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
            }`}
            key={mode}
            onClick={() => onPreflightModeChange(mode)}
            type="button"
          >
            {mode === "skipped"
              ? t("workflows.inspect.fast")
              : t("workflows.inspect.audit")}
          </button>
        ))}
      </div>
    </section>
  );
}

function CompiledInstructionsSection({
  compiledInstructions,
  instructionDrafts,
  onDraftChange,
  onSaveInstruction,
  savingInstructionId,
}: Pick<
  InspectDrawerProps,
  | "compiledInstructions"
  | "instructionDrafts"
  | "onDraftChange"
  | "onSaveInstruction"
  | "savingInstructionId"
>) {
  const { t } = useI18n();
  return (
    <section>
      <h3 className="text-xs font-semibold uppercase text-[var(--foreground-subtle)]">
        {t("workflows.inspect.compiled_prompts")}
      </h3>
      {compiledInstructions.length === 0 ? (
        <p className="mt-2 rounded-[var(--radius-sm)] border border-dashed border-[var(--border-strong)] bg-[var(--accent-background)] p-3 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("workflows.inspect.save_to_inspect")}
        </p>
      ) : (
        <div className="mt-2 space-y-3">
          {compiledInstructions.map((instruction) => (
            <div
              className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3"
              key={instruction.id}
            >
              <p className="font-mono text-[10px] text-[var(--foreground-subtle)]">
                {instruction.nodeId}
              </p>
              <textarea
                className="mt-2 min-h-32 w-full resize-y rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-2 font-mono text-[11px] leading-5 text-[var(--foreground)]"
                onChange={(event) =>
                  onDraftChange({
                    ...instructionDrafts,
                    [instruction.id]: event.target.value,
                  })
                }
                value={instructionDrafts[instruction.id] ?? instruction.systemPrompt}
              />
              <button
                className="mt-2 rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-2.5 py-1.5 text-[11px] font-medium text-[var(--foreground)] disabled:opacity-50"
                disabled={
                  savingInstructionId === instruction.id ||
                  (instructionDrafts[instruction.id] ?? instruction.systemPrompt) ===
                    instruction.systemPrompt
                }
                onClick={() => onSaveInstruction(instruction)}
                type="button"
              >
                {savingInstructionId === instruction.id
                  ? t("workflows.inspect.saving_prompt")
                  : t("workflows.inspect.save_prompt")}
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function ThoughtTrackSection({
  lastRun,
}: Pick<InspectDrawerProps, "lastRun">) {
  const { t } = useI18n();
  return (
    <section>
      <h3 className="text-xs font-semibold uppercase text-[var(--foreground-subtle)]">
        {t("workflows.inspect.thought_track")}
      </h3>
      {lastRun ? (
        <ol className="mt-2 grid max-h-80 gap-2 overflow-y-auto">
          {Object.entries(lastRun.instance.nodePayloads).map(([nodeId, payload], index) => (
            <li
              className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3"
              key={nodeId}
            >
              <p className="text-xs font-semibold text-[var(--foreground)]">
                {t("workflows.inspect.step_result", { index: index + 1 })}
              </p>
              <p className="mt-1 whitespace-pre-wrap text-[11px] leading-5 text-[var(--foreground-muted)]">
                {formatWorkflowOutput(payload.output) || t("workflows.run.no_output")}
              </p>
            </li>
          ))}
        </ol>
      ) : (
        <p className="mt-2 rounded-[var(--radius-sm)] border border-dashed border-[var(--border-strong)] bg-[var(--accent-background)] p-3 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("workflows.inspect.no_thought_track")}
        </p>
      )}
    </section>
  );
}

export function NoticePanel({
  action,
  canBuildWithout = false,
  message,
  missingCapabilities,
  missingCapabilityDetails,
  onBuildWithoutCapability,
  onConnectCapability,
  onNoticeAction,
  title,
  tone,
}: ComposerNotice & {
  canBuildWithout?: boolean;
  onBuildWithoutCapability?: (capability: NoticeCapability) => void;
  onConnectCapability?: () => void;
  onNoticeAction?: (action: ComposerNoticeAction) => void;
}) {
  const { t } = useI18n();
  const isUrgent = tone === "error" || tone === "warning";
  const capabilities = normalizeNoticeCapabilities(
    missingCapabilities,
    missingCapabilityDetails,
  );
  return (
    <section
      aria-live={isUrgent ? "assertive" : "polite"}
      className={`rounded-[var(--radius-md)] border p-4 ${composerNoticeClasses(tone)}`}
      role={isUrgent ? "alert" : "status"}
    >
      <p className="text-sm font-semibold">
        {title ??
          (tone === "success"
            ? t("workflows.composer.notice_success")
            : tone === "warning"
              ? t("workflows.composer.notice_warning")
              : tone === "info"
                ? t("workflows.composer.notice_info")
                : t("workflows.composer.notice_error"))}
      </p>
      <p className="mt-1 text-sm leading-6">{message}</p>
      {action && (
        <div className="mt-3">
          <button
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
            onClick={() => onNoticeAction?.(action)}
            type="button"
          >
            {action.label}
          </button>
        </div>
      )}
      {capabilities.length > 0 && (
        <div className="mt-3 flex flex-col gap-2">
          {capabilities.map((capability) => (
            <div
              className="rounded-[var(--radius-sm)] border border-current/20 bg-[var(--background)] p-3"
              key={capability.id ?? capability.title}
            >
              <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                <div className="min-w-0">
                  <p className="text-sm font-semibold">{capability.title}</p>
                  <p className="mt-1 text-xs leading-5 opacity-80">
                    {capability.outcome ||
                      capability.reason ||
                      t("workflows.composer.connection_unlocks_fallback")}
                  </p>
                </div>
                <div className="flex shrink-0 flex-wrap gap-2">
                  <button
                    className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
                    onClick={onConnectCapability}
                    type="button"
                  >
                    {t("workflows.composer.open_mods")}
                  </button>
                  <button
                    className="rounded-[var(--radius-sm)] border border-current/30 bg-transparent px-3 py-2 text-xs font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                    disabled={!canBuildWithout}
                    onClick={() => onBuildWithoutCapability?.(capability)}
                    type="button"
                  >
                    {t("workflows.composer.build_without_step")}
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

export function RunProgressPanel({
  fallbackLabel,
  nodes,
  progress,
}: {
  fallbackLabel: string;
  nodes: WorkflowIr["nodes"];
  progress: WorkflowRunProgress | null;
}) {
  const { t } = useI18n();
  const activeNode = progress
    ? nodes.find((node) => node.id === progress.nodeId)
    : undefined;
  const activeLabel = activeNode?.label?.trim() || fallbackLabel;
  return (
    <section
      aria-live="polite"
      className="mt-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4"
    >
      <div className="flex items-center gap-2 text-sm font-semibold text-[var(--foreground)]">
        <SpinnerIcon />
        {t("workflows.composer.running")}
      </div>
      <p className="mt-1 truncate text-xs text-[var(--foreground-muted)]">
        {activeLabel}
      </p>
      <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-[var(--border-soft)]">
        <div className="oomu-progress-indeterminate h-full w-1/3 rounded-full bg-[var(--accent)]" />
      </div>
    </section>
  );
}

export function SpinnerIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
    </svg>
  );
}
