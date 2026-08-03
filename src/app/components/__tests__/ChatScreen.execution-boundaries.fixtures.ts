import type { ChatSession } from "@/lib/chatSessions";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import type { ChatAgent } from "../ChatScreen";

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
    title: "Execution boundaries",
    providerId: "provider-1",
    modelId: "model-1",
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
];

export const cloudSessions: ChatSession[] = [
  {
    ...sessions[0],
    providerId: "cloud-provider-1",
    modelId: "gpt-5.5",
  },
];

export const dynamicSessions: ChatSession[] = [
  {
    ...sessions[0],
    providerId: "dynamic",
    modelId: "dynamic",
    dynamicRoutingOverride: true,
  },
];
