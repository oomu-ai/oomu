"use client";

export type AgentPersonalityProfile = {
  schemaVersion: 1;
  template?: {
    id: string;
    name: string;
    origin?: "system" | "custom";
    updatedAtMs?: number;
  };
  identity: {
    displayName: string;
    role: string;
    pronouns?: string;
  };
  personality: {
    summary: string;
    traits: string[];
    tone: string;
  };
  relationship: {
    userAddress: string;
    boundaries: string[];
  };
  modelBehavior: {
    baseModelDisclosure: "runtime_only";
    nameQuestionBehavior: "agent_name";
    maxOutputTokens?: number;
    dynamicRoutingDefault?: boolean;
  };
  mod_configurations?: AgentModConfigurationMap;
};

type AgentModConfigurationMap = Record<string, Record<string, unknown> | undefined>;

type AgentPromptProfile = {
  name: string;
  description: string;
  systemPrompt?: string;
  personalityProfile?: AgentPersonalityProfile;
  route?: {
    providerId: string;
    modelId: string;
  };
};

const everydayAgentTemplate = {
  id: "everyday_agent",
  name: "Everyday Agent",
  origin: "system" as const,
  traits: ["friendly", "concise", "supportive"],
};

export const MIN_AGENT_MAX_OUTPUT_TOKENS = 1024;
export const MAX_AGENT_MAX_OUTPUT_TOKENS = 8192;
export const OUTPUT_TOKEN_STEP = 1024;
const DEFAULT_CLOUD_MAX_OUTPUT_TOKENS = 4096;
const DEFAULT_LOCAL_MAX_OUTPUT_TOKENS = 2048;

function isLocalProviderId(providerId?: string) {
  const providerKey = providerId?.trim().toLowerCase().replaceAll("-", "_");
  return providerKey === "local" || providerKey === "local_model" || providerKey === "local_gemma";
}

function defaultMaxOutputTokensForProvider(providerId?: string) {
  return isLocalProviderId(providerId)
    ? DEFAULT_LOCAL_MAX_OUTPUT_TOKENS
    : DEFAULT_CLOUD_MAX_OUTPUT_TOKENS;
}

export function normalizeAgentMaxOutputTokens(value: unknown, providerId?: string) {
  const numericValue =
    typeof value === "number" && Number.isFinite(value)
      ? value
      : defaultMaxOutputTokensForProvider(providerId);
  const snappedValue = Math.round(numericValue / OUTPUT_TOKEN_STEP) * OUTPUT_TOKEN_STEP;
  return Math.min(
    MAX_AGENT_MAX_OUTPUT_TOKENS,
    Math.max(MIN_AGENT_MAX_OUTPUT_TOKENS, snappedValue),
  );
}

export function defaultAgentPersonalityProfile(args: {
  name: string;
  description: string;
  templateId?: string;
  templateName?: string;
  templateOrigin?: "system" | "custom";
  traits?: string[];
  providerId?: string;
  maxOutputTokens?: number;
}): AgentPersonalityProfile {
  const requestedTemplateName = args.templateName?.trim();
  const templateId = args.templateId?.trim() || requestedTemplateName || everydayAgentTemplate.id;
  const usesEverydayTemplate = templateId === everydayAgentTemplate.id;
  const templateName =
    (usesEverydayTemplate ? everydayAgentTemplate.name : requestedTemplateName) || templateId;
  const templateOrigin =
    args.templateOrigin ?? (usesEverydayTemplate ? everydayAgentTemplate.origin : undefined);
  const traits =
    args.traits?.filter(Boolean) ??
    (usesEverydayTemplate ? [...everydayAgentTemplate.traits] : ["helpful", "clear", "steady"]);

  return {
    schemaVersion: 1,
    template: {
      id: templateId,
      name: templateName,
      origin: templateOrigin,
      updatedAtMs: Date.now(),
    },
    identity: {
      displayName: args.name,
      role: templateName,
    },
    personality: {
      summary: args.description,
      traits,
      tone: "Natural, grounded, and aligned with the agent's configured role.",
    },
    relationship: {
      userAddress: "the user",
      boundaries: [
        "Do not claim to be the base model as your personal name.",
        "Treat model/provider details as runtime metadata, not identity.",
      ],
    },
    modelBehavior: {
      baseModelDisclosure: "runtime_only",
      nameQuestionBehavior: "agent_name",
      maxOutputTokens: normalizeAgentMaxOutputTokens(args.maxOutputTokens, args.providerId),
      dynamicRoutingDefault: false,
    },
  };
}

export function normalizeAgentPersonalityProfile(args: {
  name: string;
  description: string;
  profile?: AgentPersonalityProfile | null;
  providerId?: string;
}) {
  const fallback = defaultAgentPersonalityProfile(args);
  const profile = args.profile;
  if (!profile) {
    return fallback;
  }

  return {
    ...fallback,
    ...profile,
    template: profile.template ?? fallback.template,
    identity: {
      ...fallback.identity,
      ...profile.identity,
      displayName: profile.identity?.displayName?.trim() || args.name,
    },
    personality: {
      ...fallback.personality,
      ...profile.personality,
      summary: profile.personality?.summary?.trim() || args.description,
      traits: profile.personality?.traits?.filter(Boolean) ?? fallback.personality.traits,
    },
    relationship: {
      ...fallback.relationship,
      ...profile.relationship,
      boundaries: profile.relationship?.boundaries?.filter(Boolean) ?? fallback.relationship.boundaries,
    },
    modelBehavior: {
      ...fallback.modelBehavior,
      ...profile.modelBehavior,
      maxOutputTokens: normalizeAgentMaxOutputTokens(
        profile.modelBehavior?.maxOutputTokens,
        args.providerId,
      ),
      dynamicRoutingDefault: Boolean(
        profile.modelBehavior?.dynamicRoutingDefault ?? fallback.modelBehavior.dynamicRoutingDefault,
      ),
    },
  };
}

export function buildAgentPersonalityPrompt(args: AgentPromptProfile) {
  const profile = normalizeAgentPersonalityProfile({
    name: args.name,
    description: args.description,
    profile: args.personalityProfile,
  });
  const displayName = profile.identity.displayName || args.name;
  const route = args.route
    ? [
        "Runtime Model Route",
        `provider_id: ${args.route.providerId}`,
        `model_id: ${args.route.modelId}`,
        "Use this only when explicitly asked about the runtime model or provider.",
      ].join("\n")
    : "";

  return [
    "Identity Persistence Contract",
    `You are speaking as ${displayName}, the OOMU agent described below.`,
    "OOMU may inject your SQLite-backed soul manifest, durable memories, and the user's saved personality profile before a chat turn.",
    "When that context is present, treat it as your available long-term context and use it naturally.",
    "If the user asks whether you can remember or update preferences, explain that OOMU can persist useful preferences, relationship notes, and agent self-updates into its signed SQLite memory ledger after a chat turn.",
    "Do not say you only have temporary session memory unless persistence is explicitly unavailable.",
    "",
    "Agent Identity",
    `Your active conversational name is ${displayName}.`,
    `You are operating as the OOMU agent named ${displayName}.`,
    `If the user asks your name, answer that your name is ${displayName}.`,
    `In ordinary conversation, refer to yourself in first person as "I", "me", and "my"; do not use ${displayName} as a third-person substitute for yourself.`,
    "Do not answer that your name is Gemma, Gemma 4, the base model, or the provider.",
    "",
    "Agent Role",
    profile.identity.role,
    args.description,
    "",
    "Personality",
    profile.personality.summary,
    `Traits: ${profile.personality.traits.join(", ")}.`,
    `Tone: ${profile.personality.tone}`,
    "",
    "Relationship",
    `Address the user as ${profile.relationship.userAddress || "the user"}.`,
    ...profile.relationship.boundaries.map((boundary) => `- ${boundary}`),
    "",
    "Operating Instructions",
    args.systemPrompt?.trim() || args.description,
    "",
    route,
    "",
    "Answer only the latest user message. Be clear, concise, and natural.",
  ].filter(Boolean).join("\n");
}
