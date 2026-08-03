"use client";

import { useMemo } from "react";
import { useI18n } from "@/context/I18nContext";
import type { WorkflowIr, WorkflowIrNode } from "./workflowIr";
import {
  buildWorkflowStoryboardModel,
  WorkflowNatureIcon,
  type StoryboardNature,
} from "./WorkflowStoryboard";
import { firstSentenceForWorkflowPreview } from "./workflowPreviewText";

const CONFIGURED_ROUTINE_DELIVERY = "configured_private_channel";

type TrustSummaryModel = {
  actions: string[];
  approvals: string[];
  touches: string[];
};

type WorkflowStoryModel = {
  beats: {
    detail: string;
    id: string;
    nature: StoryboardNature;
  }[];
  touches: string[];
};

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export function TrustSummary({ workflowIr }: { workflowIr: WorkflowIr }) {
  const { t } = useI18n();
  const model = useMemo(() => buildWorkflowStoryModel(workflowIr, t), [workflowIr, t]);
  const touches = model.touches.length
    ? t("workflows.trust.touches_summary", {
        targets: model.touches.join(", "),
      })
    : t("workflows.trust.no_external_touch");

  return (
    <section
      aria-labelledby="workflow-story-title"
      className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4"
    >
      <h3
        className="text-sm font-semibold text-[var(--foreground)]"
        id="workflow-story-title"
      >
        {t("workflows.trust.title")}
      </h3>
      <ol
        aria-label={t("workflows.trust.story_aria")}
        className="mt-3 grid gap-2"
      >
        {model.beats.map((beat) => (
          <li
            className={`flex items-center gap-3 rounded-[var(--radius-sm)] border px-3 py-3 ${
              beat.nature === "approve"
                ? "border-[var(--border-strong)] bg-[var(--accent-background)]"
                : "border-[var(--border-soft)] bg-[var(--background)]"
            }`}
            key={beat.id}
          >
            <span
              className={`grid h-7 w-7 shrink-0 place-items-center rounded-full ${
                beat.nature === "approve"
                  ? "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                  : "bg-[var(--accent-background)] text-[var(--foreground-muted)]"
              }`}
            >
              <WorkflowNatureIcon nature={beat.nature} />
            </span>
            <p className="min-w-0 flex-1 text-sm leading-5 text-[var(--foreground)]">
              {beat.detail}
            </p>
            {beat.nature === "approve" && (
              <span className="shrink-0 rounded-full border border-[var(--border-strong)] bg-[var(--background)] px-2 py-0.5 text-[11px] font-semibold text-[var(--foreground-muted)]">
                {t("workflows.storyboard.natures.approve")}
              </span>
            )}
          </li>
        ))}
      </ol>
      <p className="mt-3 border-t border-[var(--border-soft)] pt-3 text-xs leading-5 text-[var(--foreground-muted)]">
        {touches}
      </p>
    </section>
  );
}

export function buildWorkflowStoryModel(
  workflowIr: WorkflowIr,
  t: TranslateFn,
): WorkflowStoryModel {
  const includesConfiguredRoutineDelivery =
    workflowIr.metadata?.oomuRoutineDelivery === CONFIGURED_ROUTINE_DELIVERY;
  const beats = buildWorkflowStoryboardModel(workflowIr, t)
    .filter(
      (item) =>
        item.node.kind !== "input" &&
        (item.node.kind !== "output" ||
          (includesConfiguredRoutineDelivery && item.node.id === "output")),
    )
    .map((item) => ({
      detail:
        item.node.kind === "output" && includesConfiguredRoutineDelivery
          ? t("workflows.trust.actions.deliver_configured_channel")
          : item.detail,
      id: item.id,
      nature: item.nature,
    }));
  return {
    beats,
    touches: buildTrustSummaryModel(workflowIr, t).touches,
  };
}

export function buildTrustSummaryModel(
  workflowIr: WorkflowIr,
  t: TranslateFn = defaultTrustTranslate,
): TrustSummaryModel {
  const actions = dedupe(
    workflowIr.nodes
      .filter((node) => node.kind !== "input" && node.kind !== "output")
      .map((node) => actionDisclosure(node, t))
      .filter((value): value is string => Boolean(value)),
  );
  const approvals = dedupe(
    workflowIr.nodes
      .filter((node): node is Extract<WorkflowIrNode, { kind: "permission" }> =>
        node.kind === "permission",
      )
      .map((node) =>
        t("workflows.trust.actions.permission", {
          reason: firstSentenceForWorkflowPreview(node.reason),
        }),
      ),
  );
  const touches = dedupe(
    workflowIr.nodes
      .map((node) => touchDisclosure(node, t))
      .filter((value): value is string => Boolean(value)),
  );

  return {
    actions: actions.slice(0, 6),
    approvals,
    touches,
  };
}

function actionDisclosure(node: WorkflowIrNode, t: TranslateFn) {
  if (node.kind === "mcp_tool") {
    const copy = mcpTrustCopy(node.serverName, node.toolName, t);
    return isWriteLike(node.toolName)
      ? t("workflows.trust.actions.mcp_write", { action: copy.action })
      : t("workflows.trust.actions.mcp_read", { action: copy.action });
  }
  if (node.kind === "agent") {
    return agentTrustCopy(node, t);
  }
  if (node.kind === "permission") {
    return t("workflows.trust.actions.permission", {
      reason: firstSentenceForWorkflowPreview(node.reason),
    });
  }
  if (node.kind === "system_action") {
    return node.command === "open"
      ? t("workflows.trust.actions.system_open")
      : t("workflows.trust.actions.system_action");
  }
  if (node.kind === "conditional" || node.kind === "router") {
    return t("workflows.trust.actions.decision");
  }
  if (node.kind === "loop") {
    return t("workflows.trust.actions.loop");
  }
  return null;
}

function touchDisclosure(node: WorkflowIrNode, t: TranslateFn) {
  if (node.kind === "mcp_tool") {
    const copy = mcpTrustCopy(node.serverName, node.toolName, t);
    return isWriteLike(node.toolName)
      ? t("workflows.trust.touches.write", { target: copy.target })
      : t("workflows.trust.touches.read", { target: copy.target });
  }
  if (node.kind === "system_action") {
    return t("workflows.trust.touches.local_mac");
  }
  return null;
}

function isWriteLike(toolName: string) {
  const normalized = toolName.toLowerCase();
  return /(write|draft|send|create|update|delete|patch|open|trigger)/.test(
    normalized,
  );
}

function agentTrustCopy(node: Extract<WorkflowIrNode, { kind: "agent" }>, t: TranslateFn) {
  const text = `${node.label} ${node.objective}`.toLowerCase();
  if (/(draft|reply|email|message)/.test(text)) {
    return t("workflows.trust.actions.agent_draft");
  }
  if (/(decide|recommend|priority|risk|route|branch)/.test(text)) {
    return t("workflows.trust.actions.agent_decide");
  }
  return t("workflows.trust.actions.agent_summary");
}

function mcpTrustCopy(serverName: string, toolName: string, t: TranslateFn) {
  const key = `${serverName}.${toolName}`;
  const known = {
    "local_filesystem.list_directory": "workflow_folder",
    "local_filesystem.read_file": "workflow_file",
    "local_filesystem.write_file": "workflow_file",
    "macos_applescript.read_system_calendar": "calendar",
    "macos_applescript.trigger_system_notification": "notification",
    "macos_applescript.draft_system_email": "mail_draft",
    "macos_applescript.read_system_emails": "mail",
    "macos_applescript.read_system_reminders": "reminders",
    "taskflow_native.folder_read": "project_folder",
    "taskflow_native.write_markdown_report": "project_report",
    "taskflow_native.preview_report": "project_report",
  }[key];
  if (known) {
    return {
      action: t(`workflows.trust.action_targets.${known}`),
      target: t(`workflows.trust.touch_targets.${known}`),
    };
  }
  const fallback = humanize(toolName || serverName);
  return { action: fallback, target: fallback };
}

function humanize(value: string) {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function dedupe(values: string[]) {
  return [...new Set(values)];
}

function defaultTrustTranslate(
  key: string,
  variables?: Record<string, string | number>,
) {
  const fallbacks: Record<string, string> = {
    "workflows.trust.actions.agent_decide": "Summarizes what it found and suggests the next step.",
    "workflows.trust.actions.agent_draft": "Drafts text from what it read.",
    "workflows.trust.actions.agent_summary": "Summarizes what it found.",
    "workflows.trust.actions.decision": "Chooses a path from the workflow results.",
    "workflows.trust.actions.deliver_configured_channel":
      "Delivers the verified result and exact filename through this Routine’s configured private channel.",
    "workflows.trust.actions.loop": "Repeats the same step for each item.",
    "workflows.trust.actions.mcp_read": "Reads {action}.",
    "workflows.trust.actions.mcp_write": "Opens or changes {action}.",
    "workflows.trust.actions.permission": "Asks you first: {reason}",
    "workflows.trust.actions.system_action": "Uses a limited local action.",
    "workflows.trust.actions.system_open": "Opens the result on this Mac.",
    "workflows.trust.touches.approval": "Your approval",
    "workflows.trust.touches.local_mac": "This Mac",
    "workflows.trust.touches.read": "{target} (read)",
    "workflows.trust.touches.write": "{target} (opens or writes)",
  };
  let value = fallbacks[key] ?? key;
  Object.entries(variables ?? {}).forEach(([name, replacement]) => {
    value = value.split(`{${name}}`).join(String(replacement));
  });
  return value;
}
