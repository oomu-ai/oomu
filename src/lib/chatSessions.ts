export type ChatSession = {
  id: string;
  projectId?: string | null;
  agentId: string;
  title: string;
  titleSource?: "auto" | "user" | string;
  providerId: string;
  modelId: string;
  webGroundingOverride?: boolean | null;
  dynamicRoutingOverride?: boolean | null;
  unreadCompletion?: boolean;
  createdAtMs: number;
  updatedAtMs: number;
};

export type StoredChatMessage = {
  id: number;
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  providerId?: string | null;
  modelId?: string | null;
  metadataJson?: string | null;
  isCompacted?: boolean;
  compactionType?: string | null;
  createdAtMs: number;
};
