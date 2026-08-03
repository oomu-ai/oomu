import type { P0EventEnvelope } from "@/lib/p0Contracts";
import type { TaskRun } from "./taskClient";

export type TaskEffectVerificationDecision =
  | "did_not_happen"
  | "happened"
  | "stop_without_repeating";

export type TaskEffectVerification = {
  effectKind: string;
  idempotencyKey: string;
  nodeId: string;
  retrySupported: boolean;
  surface: "calendar" | "mail_draft" | "mail_send" | "protected_action";
  verificationSequence: number;
  calendarName?: string;
  title?: string;
  recipient?: string;
  subject?: string;
};

export function taskEffectVerificationFromEvents(
  task: TaskRun,
  events: P0EventEnvelope[],
): TaskEffectVerification | null {
  if (!task.effectVerificationRequired) return null;
  const required = events
    .filter(
      (event) =>
        event.eventType === "workflow.effect.verification_required" &&
        event.taskId === task.taskId &&
        event.taskRunId === task.taskRunId,
    )
    .sort((left, right) => right.sequence - left.sequence)
    .find((event) => {
      const payload = asRecord(event.payload);
      if (!payload) return false;
      return !events.some((candidate) => {
        if (
          candidate.eventType !== "workflow.effect.verification_resolved" ||
          candidate.sequence <= event.sequence ||
          candidate.taskId !== task.taskId ||
          candidate.taskRunId !== task.taskRunId
        ) {
          return false;
        }
        const resolved = asRecord(candidate.payload);
        return Boolean(
          resolved &&
            resolved.nodeId === payload.nodeId &&
            resolved.idempotencyKey === payload.idempotencyKey &&
            resolved.effectKind === payload.effectKind,
        );
      });
    });
  if (!required) return null;

  const payload = asRecord(required.payload);
  const summary = asRecord(payload?.effectSummary);
  const nodeId = boundedIdentity(payload?.nodeId, 256);
  const idempotencyKey = boundedIdentity(payload?.idempotencyKey, 1_024);
  const effectKind = boundedIdentity(payload?.effectKind, 256);
  const surface = recoverySurface(summary?.surface);
  if (!nodeId || !idempotencyKey || !effectKind || !surface) return null;

  return {
    effectKind,
    idempotencyKey,
    nodeId,
    retrySupported: payload?.retrySupported === true,
    surface,
    verificationSequence: required.sequence,
    calendarName: boundedText(summary?.calendarName, 160),
    title: boundedText(summary?.title, 240),
    recipient: boundedText(summary?.recipient, 4_096),
    subject: boundedText(summary?.subject, 998),
  };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function boundedIdentity(value: unknown, limit: number) {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized && normalized.length <= limit && !hasControlCharacters(normalized)
    ? normalized
    : null;
}

function boundedText(value: unknown, limit: number) {
  return boundedIdentity(value, limit) ?? undefined;
}

function hasControlCharacters(value: string) {
  return [...value].some((character) => {
    const code = character.charCodeAt(0);
    return code < 32 || code === 127;
  });
}

function recoverySurface(
  value: unknown,
): TaskEffectVerification["surface"] | null {
  return value === "calendar" ||
    value === "mail_draft" ||
    value === "mail_send" ||
    value === "protected_action"
    ? value
    : null;
}
