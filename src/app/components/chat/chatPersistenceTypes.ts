import type { ReasoningLevel } from "@/lib/modelRegistry";
import type { ChatAttachment } from "./attachments";

export type CompactSessionHistoryResponse = {
  session_id: string;
  agent_id: string;
  analyzed_turns: number;
  skipped_messages: number;
  captured_memories: unknown[];
};

export type QueuedMessageRecord = {
  id: number;
  sessionId?: string | null;
  agentId: string;
  message: string;
  attachments: ChatAttachment[];
  providerId?: string | null;
  modelId?: string | null;
  reasoning?: ReasoningLevel | string | null;
  context?: string | null;
  steering?: string | null;
  status: string;
  createdAtMs: number;
  updatedAtMs: number;
  executedAtMs?: number | null;
  errorMessage?: string | null;
};

export type QueuedMessageExecutionRecord = {
  queueId: number;
  status: string;
  sessionId?: string | null;
  text?: string | null;
  error?: string | null;
};
