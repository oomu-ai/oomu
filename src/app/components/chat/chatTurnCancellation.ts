import type { ChatMessageMetadata } from "./messageMetadata";

type TranscriptEntry = {
  id: number;
  role: string;
  content: string;
  metadata?: ChatMessageMetadata | null;
};

const stoppedTurnCodes = new Set(["local_inference_cancelled", "auto_route_choice_cancelled", "project_cloud_choice_cancelled"]);

export function visibleCancelledTurnMessages<T extends TranscriptEntry>(messages: T[], assistantMessageId: number | null, turnId: string, errorCode: string, content: string, createId: () => number): T[] {
  const remaining = assistantMessageId === null ? messages : messages.filter((entry) => entry.id !== assistantMessageId);
  if (remaining.some((entry) => entry.metadata?.terminalResultForTurnId === turnId)) {
    return remaining;
  }
  return [
    ...remaining,
    {
      id: createId(),
      role: "system",
      content,
      metadata: {
        terminalResultForTurnId: turnId,
        terminalErrorCode: errorCode,
      },
    } as T,
  ];
}

export async function surfaceStoppedChatTurn<
  TContext extends {
    sessionId: string;
    turnId: string;
  },
  TMessage extends TranscriptEntry,
>(options: { errorCode: string; context: TContext; assistantMessageId: number | null; content: string; status: string; finalize: (context: TContext) => Promise<unknown>; refresh: (sessionId: string) => Promise<unknown>; updateMessages: (context: TContext, update: (messages: TMessage[]) => TMessage[]) => void; updateStatus: (context: TContext, status: string) => void; createId: () => number }) {
  if (!stoppedTurnCodes.has(options.errorCode)) return false;
  if (options.errorCode !== "local_inference_cancelled") {
    await options.finalize(options.context).catch(() => undefined);
  }
  await options.refresh(options.context.sessionId).catch(() => undefined);
  options.updateMessages(options.context, (messages) => visibleCancelledTurnMessages(messages, options.assistantMessageId, options.context.turnId, options.errorCode, options.content, options.createId));
  options.updateStatus(options.context, options.status);
  return true;
}
