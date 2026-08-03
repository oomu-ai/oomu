import type { StoredChatMessage } from "@/lib/chatSessions";
import { normalizeChatMessageMetadata } from "./messageMetadata";

const DEFAULT_RECONCILIATION_DELAYS_MS = [
  0,
  250,
  500,
  1_000,
  ...Array.from({ length: 150 }, () => 2_000),
];

type TerminalChatTurnWaitResult =
  | { status: "terminal"; messages: StoredChatMessage[] }
  | { status: "cancelled" }
  | { status: "timed_out" };

function waitForReconciliationDelay(delayMs: number, signal?: AbortSignal) {
  if (signal?.aborted) return Promise.resolve(false);
  if (delayMs <= 0) return Promise.resolve(true);
  return new Promise<boolean>((resolve) => {
    let settled = false;
    const finish = (completed: boolean) => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", onAbort);
      resolve(completed);
    };
    const timerId = window.setTimeout(() => finish(true), delayMs);
    const onAbort = () => {
      window.clearTimeout(timerId);
      finish(false);
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

export function hasTerminalChatTurnResult(
  messages: StoredChatMessage[],
  turnId: string,
) {
  return messages.some(
    (message) =>
      normalizeChatMessageMetadata(message.metadataJson)
        ?.terminalResultForTurnId === turnId,
  );
}

export async function waitForTerminalChatTurnResult(
  fetchMessages: () => Promise<StoredChatMessage[]>,
  turnId: string,
  options?: {
    delaysMs?: number[];
    signal?: AbortSignal;
    shouldContinue?: () => boolean;
  },
): Promise<TerminalChatTurnWaitResult> {
  for (const delayMs of options?.delaysMs ?? DEFAULT_RECONCILIATION_DELAYS_MS) {
    if (options?.signal?.aborted || (options?.shouldContinue && !options.shouldContinue())) {
      return { status: "cancelled" };
    }
    if (!(await waitForReconciliationDelay(delayMs, options?.signal))) {
      return { status: "cancelled" };
    }
    if (options?.signal?.aborted || (options?.shouldContinue && !options.shouldContinue())) {
      return { status: "cancelled" };
    }
    try {
      const messages = await fetchMessages();
      if (hasTerminalChatTurnResult(messages, turnId)) {
        return { status: "terminal", messages };
      }
    } catch {
      // A transient hydration read must not turn an active response into a failure.
    }
  }
  return { status: "timed_out" };
}
