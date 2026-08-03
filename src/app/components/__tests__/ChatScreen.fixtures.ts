import type { ChatAgent } from "../ChatScreen";
import type { ChatSession, StoredChatMessage } from "@/lib/chatSessions";
import type { ConfiguredProvider } from "@/lib/modelRegistry";

export function approvedFilePreparation(
  displayName: string,
  content: string,
  mimeType = "text/plain",
  byteCount = new TextEncoder().encode(content).byteLength,
) {
  return {
    displayName,
    mimeType,
    byteCount,
    receipt: {
      payload: "signed-approved-file-payload",
      signature: {
        public_key: "test-public-key",
        signature: "test-receipt-signature",
        payload_hash: "test-receipt-hash",
        signed_at_ms: 1,
      },
    },
  };
}

export function resolveDeferred<T>(
  resolver: ((value: T) => void) | null,
  value: T,
): void {
  if (!resolver) throw new Error("Deferred test resolver was not initialized");
  resolver(value);
}

export function rejectDeferred(
  rejecter: ((reason?: unknown) => void) | null,
  reason: unknown,
): void {
  if (!rejecter) throw new Error("Deferred test rejecter was not initialized");
  rejecter(reason);
}

export function token(
  request: Record<string, string>,
  sessionId: string,
  sequence: number,
  token: string,
) {
  return {
    stream_id: request.stream_id,
    session_id: sessionId,
    turn_id: request.turn_id,
    generation_token: request.generation_token,
    sequence,
    token,
    elapsed_ms: sequence,
    delivery_state: "validated",
  };
}

export async function terminal(
  request: Record<string, string>,
  sessionId: string,
  text: string,
  chunkCount: number,
) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  const textSha256 = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("");
  return {
    stream_id: request.stream_id,
    session_id: sessionId,
    turn_id: request.turn_id,
    generation_token: request.generation_token,
    last_sequence: chunkCount,
    chunk_count: chunkCount,
    text_sha256: textSha256,
    delivery_state: "validated",
  };
}

export const agents: ChatAgent[] = [
  {
    id: "agent-1",
    name: "OOMU",
    description: "Test agent",
    endpoint: { provider: "provider-1", modelId: "model-1" },
  },
];

export const configuredProviders: ConfiguredProvider[] = [
  {
    id: "provider-1",
    providerId: "local",
    providerName: "Local",
    authMethod: "api_key",
    baseUrl: "",
    apiKeyLabel: "",
    customModelIds: "model-1",
  },
];

export const cloudConfiguredProviders: ConfiguredProvider[] = [
  {
    id: "cloud-provider-1",
    providerId: "openai",
    providerName: "OpenAI",
    authMethod: "api_key",
    baseUrl: "https://api.openai.com/v1",
    apiKeyLabel: "TEST_OPENAI_KEY",
    customModelIds: "gpt-5.5",
  },
];

export const geminiConfiguredProviders: ConfiguredProvider[] = [
  {
    id: "gemini-provider-1",
    providerId: "google",
    providerName: "Google Gemini",
    authMethod: "api_key",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta",
    apiKeyLabel: "TEST_GEMINI_KEY",
    customModelIds: "gemini-3.5-flash",
  },
];

export const cloudAgents: ChatAgent[] = [
  {
    id: "agent-1",
    name: "OOMU",
    description: "Test agent",
    endpoint: { provider: "cloud-provider-1", modelId: "gpt-5.5" },
  },
];

export const sessions: ChatSession[] = [
  {
    id: "session-1",
    agentId: "agent-1",
    title: "Debug chat",
    providerId: "provider-1",
    modelId: "model-1",
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
];

export const cloudSessions: ChatSession[] = [
  { ...sessions[0], providerId: "cloud-provider-1", modelId: "gpt-5.5" },
];

export const searchEnabledSessions: ChatSession[] = [
  { ...sessions[0], webGroundingOverride: true },
];

export const storedMessages: StoredChatMessage[] = [
  {
    id: 1,
    sessionId: "session-1",
    role: "user",
    content:
      "This TypeScript file has a failing test error. Please review the implementation plan.",
    createdAtMs: 1,
  },
];

export function testBypassNotice() {
  return {
    title: "Security preflight",
    body: "Bypassed local check due to payload size (150K tokens, over the 6K local threshold). Routed directly to gemini-3.5-flash.",
    detail: "Local security preflight did not complete.",
  };
}
