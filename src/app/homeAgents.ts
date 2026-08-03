import {
  defaultAgentPersonalityProfile,
  normalizeAgentPersonalityProfile,
  type AgentPersonalityProfile,
} from "@/lib/agentPersonality";

type AgentStatus = "active" | "archived";
type AgentCardIcon = "mark" | "person" | "robot";

export type AgentCardData = {
  id: string;
  name: string;
  description: string;
  systemPrompt: string;
  icon?: AgentCardIcon;
  image?: string | null;
  type?: AgentStatus;
  createdAt?: number;
  favorited?: boolean;
  lastAccessedAt?: number;
  endpoint?: AgentEndpointConfig;
  personalityTemplate?: AgentPersonalityTemplate;
  personalityProfile?: AgentPersonalityProfile;
};

export type AgentModelProvider = string;

type AgentEndpointConfig = {
  provider: AgentModelProvider;
  modelId: string;
  customName?: string;
  customBaseUrl?: string;
};

export const defaultLocalAgentEndpoint: AgentEndpointConfig = {
  provider: "local_model",
  modelId: "",
};

export type AgentConfigRecord = {
  id: string;
  name: string;
  system_prompt: string;
  model_id: string;
  provider_id: AgentModelProvider;
  description: string;
  image?: string | null;
  personality_profile?: AgentPersonalityProfile | string | null;
  favorited: boolean;
  status: AgentStatus;
  created_at_ms: number;
  updated_at_ms: number;
};

export type InstalledModRecord = {
  id: string;
  name: string;
  isActive: boolean;
};

export type AgentModBadge = {
  id: string;
  name: string;
};

export type LocalModelCompatibility = {
  id: string;
  compatibility: "ready" | "unsupported" | "invalid" | "asset_missing";
};

export type DegradedModeStatus = {
  active: boolean;
  reason: string | null;
  hasVolatileStorage: boolean;
  subsystems: Array<{
    subsystem: string;
    active: boolean;
    cause: string | null;
    firstOccurredAtMs: number | null;
    backingStoreClass: "notApplicable" | "persistent" | "recoveryPending" | "volatile";
    recoveryEligible: boolean;
    lastProbeResult: {
      attemptedAtMs: number;
      succeeded: boolean;
      message: string;
    } | null;
    userVisibleImpact: string;
  }>;
};

export function shouldShowDegradedLanding(
  status: DegradedModeStatus | null,
  activeItem: string,
): status is DegradedModeStatus {
  const featureLocalSubsystems = new Set([
    "artifactPipeline",
    "autoRouteClassifier",
    "autoRouteSessionBaselines",
    "backgroundHooks",
    "gateway",
    "identity",
    "mcpRuntime",
    "workflowScheduler",
  ]);
  const activeSubsystems = status?.subsystems.filter((subsystem) => subsystem.active) ?? [];
  const hasApplicationBlocker = Boolean(
    status?.hasVolatileStorage ||
      // Unknown degraded state remains fail-closed. Known feature-local
      // failures stay visible in health/Settings without replacing the app.
      (status?.active && activeSubsystems.length === 0) ||
      activeSubsystems.some(
        (subsystem) => !featureLocalSubsystems.has(subsystem.subsystem),
      ),
  );
  return Boolean(
    status?.active &&
      hasApplicationBlocker &&
      (activeItem !== "settings" || status.hasVolatileStorage),
  );
}

export type AgentPersonalityTemplate = string;

type AgentInstructionAttribute = {
  id: string;
  label: string;
  instruction: string;
};

export type AgentInstructionTemplate = {
  id: AgentPersonalityTemplate;
  name: string;
  description: string;
  instructions: string;
  attributes: string[];
  origin: "system" | "custom";
};

export const instructionAttributeOptions: AgentInstructionAttribute[] = [
  {
    id: "friendly",
    label: "Friendly",
    instruction: "Use a warm, approachable tone that helps the user feel comfortable asking follow-up questions.",
  },
  {
    id: "concise",
    label: "Concise",
    instruction: "Keep responses tight and high-signal unless the user asks for deeper detail.",
  },
  {
    id: "professional",
    label: "Professional",
    instruction: "Maintain polished, workplace-ready language and make recommendations with clear rationale.",
  },
  {
    id: "curious",
    label: "Curious",
    instruction: "Ask thoughtful clarifying questions when the goal is ambiguous, then proceed decisively once context is sufficient.",
  },
  {
    id: "methodical",
    label: "Methodical",
    instruction: "Break complex work into ordered steps, track assumptions, and surface risks before committing to a direction.",
  },
  {
    id: "creative",
    label: "Creative",
    instruction: "Offer imaginative options and unexpected angles while staying anchored to the user's constraints.",
  },
  {
    id: "skeptical",
    label: "Skeptical",
    instruction: "Pressure-test claims, call out uncertainty, and distinguish evidence from inference.",
  },
  {
    id: "supportive",
    label: "Supportive",
    instruction: "Encourage momentum, reduce anxiety, and frame feedback as collaborative next steps.",
  },
];

export const personalityTemplateOptions: AgentInstructionTemplate[] = [
  {
    id: "everyday_agent",
    name: "Everyday Agent",
    description: "A balanced general-purpose helper for everyday coordination, writing, and clear task support.",
    instructions:
      "Help the user organize everyday tasks, draft clear copy, and provide simple next steps. Avoid complex jargon and ensure responses are warm, steady, and high-signal.",
    attributes: ["friendly", "concise", "supportive"],
    origin: "system",
  },
  {
    id: "researcher",
    name: "Researcher",
    description: "Gathers context, compares sources, and turns findings into clear summaries.",
    instructions:
      "Investigate the topic carefully, compare available evidence, note uncertainty, and summarize findings in a way the user can act on.",
    attributes: ["curious", "methodical", "skeptical"],
    origin: "system",
  },
  {
    id: "coder",
    name: "Coder",
    description: "Focuses on implementation, debugging, code review, and technical tradeoffs.",
    instructions:
      "Work like a senior engineering collaborator. Inspect the code before changing it, prefer small safe edits, verify behavior, and explain tradeoffs clearly.",
    attributes: ["methodical", "concise", "professional"],
    origin: "system",
  },
  {
    id: "marketer",
    name: "Marketer",
    description: "Shapes positioning, messaging, launch copy, and audience-aware campaign ideas.",
    instructions:
      "Translate product intent into sharp positioning, audience-aware copy, and concrete campaign ideas with clear success signals.",
    attributes: ["creative", "professional", "concise"],
    origin: "system",
  },
  {
    id: "tutor",
    name: "Tutor",
    description: "Explains concepts patiently, adapts pacing, and helps users build confidence.",
    instructions:
      "Teach by meeting the user at their current level. Explain concepts plainly, check for understanding, and build confidence through examples.",
    attributes: ["friendly", "curious", "supportive"],
    origin: "system",
  },
];

export function normalizeAgentCards(agents: AgentCardData[], type: AgentStatus) {
  const baseTime = createAgentTimestamp();

  return agents.map((agent, index) => {
    const inferredTime = baseTime - index;
    const createdAt = agent.createdAt ?? agent.lastAccessedAt ?? inferredTime;
    const personalityTemplate =
      agent.personalityTemplate?.trim() ||
      agent.personalityProfile?.template?.id?.trim() ||
      "everyday_agent";
    const templateOption = personalityTemplateOptions.find(
      (template) => template.id === personalityTemplate,
    );

    return {
      ...agent,
      type,
      systemPrompt: agent.systemPrompt || agent.description,
      createdAt,
      favorited: type === "active" ? Boolean(agent.favorited) : false,
      lastAccessedAt: agent.lastAccessedAt ?? createdAt,
      personalityTemplate,
      personalityProfile: agent.personalityProfile ??
        defaultAgentPersonalityProfile({
          name: agent.name,
          description: agent.description,
          templateId: personalityTemplate,
          templateName: templateOption?.name,
          templateOrigin: templateOption?.origin,
          traits: templateOption?.attributes.map((attributeId) => attributeLabel(attributeId)),
          providerId: agent.endpoint?.provider,
        }),
    };
  });
}

function agentSortValue(agent: AgentCardData) {
  return agent.lastAccessedAt ?? agent.createdAt ?? 0;
}

export function sortAgentCards(agents: AgentCardData[]) {
  return [...agents].sort((a, b) => {
    if (Boolean(a.favorited) !== Boolean(b.favorited)) {
      return a.favorited ? -1 : 1;
    }

    return agentSortValue(b) - agentSortValue(a);
  });
}

export function createAgentTimestamp() {
  return Date.now();
}

export function cropAgentImage(file: File, onComplete: (dataUrl: string) => void) {
  const reader = new FileReader();
  reader.onload = (event) => {
    const img = new Image();
    img.onload = () => {
      const minDimension = Math.min(img.width, img.height);
      const startX = (img.width - minDimension) / 2;
      const startY = (img.height - minDimension) / 2;
      const targetSize = Math.min(Math.max(minDimension, 500), 2056);
      const canvas = document.createElement("canvas");
      const ctx = canvas.getContext("2d");

      if (!ctx) {
        return;
      }

      canvas.width = targetSize;
      canvas.height = targetSize;
      ctx.drawImage(img, startX, startY, minDimension, minDimension, 0, 0, targetSize, targetSize);
      onComplete(canvas.toDataURL("image/jpeg", 0.9));
    };
    img.src = event.target?.result as string;
  };
  reader.readAsDataURL(file);
}

export function createAgentId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `agent-${crypto.randomUUID()}`;
  }

  return `agent-${createAgentTimestamp().toString(36)}`;
}

export function createAgentTemplateId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `template-${crypto.randomUUID()}`;
  }

  return `template-${createAgentTimestamp().toString(36)}`;
}

export function attributeLabel(attributeId: string) {
  return instructionAttributeOptions.find((attribute) => attribute.id === attributeId)?.label ?? attributeId;
}

export function buildTemplatePreview(template: AgentInstructionTemplate) {
  const selectedAttributes = template.attributes
    .map((attributeId) => instructionAttributeOptions.find((attribute) => attribute.id === attributeId))
    .filter((attribute): attribute is AgentInstructionAttribute => Boolean(attribute));

  return [
    "Core Instructions",
    template.instructions,
    "",
    "Style and Behavior",
    ...selectedAttributes.map((attribute) => `- ${attribute.instruction}`),
  ].join("\n");
}

export function configToAgent(config: AgentConfigRecord): AgentCardData {
  const personalityProfile = parseAgentPersonalityProfile(
    config.personality_profile,
    config.name,
    config.description,
    config.provider_id,
  );
  return {
    id: config.id,
    name: config.name,
    description: config.description,
    systemPrompt: config.system_prompt,
    icon: "mark",
    image: config.image ?? null,
    type: config.status,
    createdAt: config.created_at_ms,
    favorited: config.status === "active" && Boolean(config.favorited),
    lastAccessedAt: config.updated_at_ms,
    endpoint: {
      provider: config.provider_id,
      modelId: config.model_id,
    },
    personalityTemplate: personalityProfile.template?.id,
    personalityProfile,
  };
}

export function agentToConfigRequest(agent: AgentCardData) {
  const personalityProfile = agent.personalityProfile ??
    defaultAgentPersonalityProfile({
      name: agent.name,
      description: agent.description,
      templateId: agent.personalityTemplate,
      templateName: agent.personalityTemplate,
      providerId: agent.endpoint?.provider,
    });
  const normalizedPersonalityProfile = normalizeAgentPersonalityProfile({
    name: agent.name,
    description: agent.description,
    profile: personalityProfile,
    providerId: agent.endpoint?.provider,
  });

  return {
    id: agent.id,
    name: agent.name,
    system_prompt: agent.systemPrompt || agent.description,
    model_id: agent.endpoint?.modelId ?? "",
    provider_id: agent.endpoint?.provider || defaultLocalAgentEndpoint.provider,
    description: agent.description,
    image: agent.image ?? null,
    personality_profile: normalizedPersonalityProfile,
    favorited: agent.type === "archived" ? false : Boolean(agent.favorited),
    status: agent.type ?? "active",
  };
}

const LOCAL_AGENT_PROVIDER_IDS = new Set([
  "local",
  "local_gemma",
  "local_model",
]);

export function agentToStartupConfigRequest(
  agent: AgentCardData,
  verifiedStartupModelId: string | null,
) {
  const request = agentToConfigRequest(agent);
  const providerId = request.provider_id.trim().toLowerCase().replaceAll("-", "_");
  if (!LOCAL_AGENT_PROVIDER_IDS.has(providerId) || request.model_id.trim()) {
    return request;
  }

  const modelId = verifiedStartupModelId?.trim();
  return modelId ? { ...request, model_id: modelId } : null;
}

export function startupAgentConfigRequests(
  agents: AgentCardData[],
  verifiedStartupModelId: string | null,
) {
  const requests: Array<ReturnType<typeof agentToConfigRequest>> = [];
  for (const agent of agents) {
    const request = agentToStartupConfigRequest(agent, verifiedStartupModelId);
    if (!request) return null;
    requests.push(request);
  }
  return requests;
}

function parseAgentPersonalityProfile(
  value: AgentConfigRecord["personality_profile"],
  name: string,
  description: string,
  providerId?: string,
) {
  const normalize = (profile: AgentPersonalityProfile | null) =>
    normalizeAgentPersonalityProfile({
      name,
      description,
      profile,
      providerId,
    });

  if (value && typeof value === "object") {
    if (Object.keys(value).length === 0) {
      return normalize(null);
    }
    return normalize(value);
  }

  if (typeof value === "string" && value.trim()) {
    try {
      const parsed = JSON.parse(value) as AgentPersonalityProfile;
      if (!parsed || Object.keys(parsed).length === 0) {
        return normalize(null);
      }
      return normalize(parsed);
    } catch {
      return normalize(null);
    }
  }

  return normalize(null);
}

export function activeBadgesForBoundMods(mods: InstalledModRecord[], boundModIds: string[]) {
  const bound = new Set(boundModIds);
  return mods
    .filter((mod) => mod.isActive && bound.has(mod.id))
    .map((mod) => ({ id: mod.id, name: mod.name }));
}

const templateCapabilities: Record<
  string,
  {
    characteristics: string[];
    examples: string[];
  }
> = {
  everyday_agent: {
    characteristics: [
      "Goal Organizer: Helps you turn big ideas and intents into easy, structured next steps.",
      "Friendly Support: Encourages progress and keeps communication warm and helpful.",
      "Everyday Helper: Perfect for general writing, planning your week, or organizing ideas."
    ],
    examples: [
      "Draft a weekly planning routine to help me stay on track.",
      "Help me write a warm congratulations message to a team member."
    ]
  },
  researcher: {
    characteristics: [
      "Thorough Analysis: Looks at topics from multiple angles to find the most balanced perspective.",
      "Fact Checker: Focuses on verifiable facts and clearly flags any areas of uncertainty.",
      "Clear Summaries: Condenses long articles or complicated topics into quick, easy-to-read digests."
    ],
    examples: [
      "Summarize the main theories about how plants communicate with each other.",
      "What are the key differences between various organic coffee beans?"
    ]
  },
  coder: {
    characteristics: [
      "Step-by-Step Problem Solving: Breaks down programming tasks and explains how everything works.",
      "Clear Code Explanations: Writes clean, readable code and comments so you can easily learn along.",
      "Helpful Debugger: Carefully inspects error messages to guide you directly to the solution."
    ],
    examples: [
      "Explain how a React state hook works with a simple counting example.",
      "Help me find the typo in this HTML structure."
    ]
  },
  marketer: {
    characteristics: [
      "Creative Writing: Helps you draft engaging posts, emails, and campaign ideas that stand out.",
      "Audience-Focused: Adapts the tone to match exactly who you are writing for.",
      "Brainstorming Partner: Provides fresh perspectives and fun angles for your projects."
    ],
    examples: [
      "Help me brainstorm three fun names for a local neighborhood bookstore.",
      "Draft a friendly social post announcing our local community garden startup."
    ]
  },
  tutor: {
    characteristics: [
      "Patient Teacher: Explains complicated concepts step-by-step, adapting to your personal pace.",
      "Concept Checkpoints: Gently asks questions along the way to make sure we are on the same page.",
      "Simple Analogies: Uses everyday examples to make complex ideas feel approachable."
    ],
    examples: [
      "Explain how gravity works using a simple trampoline analogy.",
      "Teach me some basic Spanish phrases to use when ordering food."
    ]
  }
};

export const getCapabilitiesForTemplate = (template: AgentInstructionTemplate) => {
  const customCap = templateCapabilities[template.id];
  if (customCap) return customCap;

  const characteristics = template.attributes.map((attrId) => {
    const attr = instructionAttributeOptions.find((a) => a.id === attrId);
    return attr ? `${attr.label}: ${attr.instruction}` : `${attrId}: Custom trait baseline.`;
  });

  return {
    characteristics: characteristics.length > 0 ? characteristics : ["General: Adapts to whatever you're working on."],
    examples: [
      `Help me get started on a project that suits the ${template.name} personality.`,
      `Review this document and suggest improvements.`
    ]
  };
};
