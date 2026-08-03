import catalogSource from "./oomuModelCatalog2026.json";

export type AuthMethod = "api_key" | "oauth" | "service_account" | "custom";
export type ReasoningLevel = "off" | "on" | "low" | "medium" | "high" | "max";
declare const providerConfigurationIdBrand: unique symbol;
declare const providerTypeIdBrand: unique symbol;
declare const canonicalModelIdBrand: unique symbol;

export type ProviderConfigurationId = string & {
  readonly [providerConfigurationIdBrand]: "ProviderConfigurationId";
};
export type ProviderTypeId = string & {
  readonly [providerTypeIdBrand]: "ProviderTypeId";
};
export type CanonicalModelId = string & {
  readonly [canonicalModelIdBrand]: "CanonicalModelId";
};

export function providerConfigurationId(value: string): ProviderConfigurationId {
  const normalized = value.trim();
  if (!normalized) throw new Error("provider_configuration_id_missing");
  return normalized as ProviderConfigurationId;
}

export function providerTypeId(value: string): ProviderTypeId {
  const normalized = value.trim();
  if (!normalized) throw new Error("provider_type_id_missing");
  return normalized as ProviderTypeId;
}

export function canonicalModelId(value: string): CanonicalModelId {
  const normalized = value.trim();
  if (!normalized) throw new Error("canonical_model_id_missing");
  return normalized as CanonicalModelId;
}

export const DEFAULT_REASONING_LEVELS: ReasoningLevel[] = ["off", "low", "medium", "high", "max"];

// On-device models expose a plain thinking on/off toggle rather than a graded
// cloud "effort" budget. "on" sits just above "off" on the shared ladder so it
// resolves cleanly when switching between a local model and a graded one.
const LOCAL_REASONING_LEVELS: ReasoningLevel[] = ["off", "on"];

const REASONING_LEVEL_RANKS: Record<ReasoningLevel, number> = {
  off: 0,
  on: 1,
  low: 1,
  medium: 2,
  high: 3,
  max: 4,
};

export type ConfiguredProvider = {
  id: string;
  providerId: string;
  providerName: string;
  authMethod: AuthMethod;
  baseUrl: string;
  apiKeyLabel: string;
  apiKey?: string;
  credentialConfigured?: boolean;
  customModelIds: string;
  autoRouteTarget?: boolean;
  createdAtMs?: number;
  updatedAtMs?: number;
};

export type ConfiguredModelOption = {
  providerId: string;
  providerConfigId: ProviderConfigurationId;
  providerType: ProviderTypeId;
  providerName: string;
  modelId: string;
  label: string;
  context: string;
  supportedReasoningLevels: ReasoningLevel[];
};

export type ModelThinkingSupport = {
  type:
    | "none"
    | "thinking_level"
    | "reasoning_effort"
    | "thinking_budget"
    | "include_reasoning";
  parameterName: string | null;
  levels: string[];
  defaultLevel: string;
};

export type ModelTemplate = {
  modelId: string;
  name: string;
  providerId: string;
  contextBudget: number;
  maxOutputTokens: number;
  reasoningSupport: boolean;
  thinkingSupport: ModelThinkingSupport;
  pricingPer1M: {
    input: number;
    output: number;
  };
};

export type RemoteModelCatalog = {
  version: string;
  updatedAt: string;
  description: string;
  providers: Array<{
    providerId: string;
    providerName: string;
    baseUrl: string;
    models: ModelTemplate[];
  }>;
};

type CatalogSource = typeof catalogSource;

function catalogThinkingSupportType(value: string): ModelThinkingSupport["type"] {
  switch (value) {
    case "none":
    case "thinking_level":
    case "reasoning_effort":
    case "thinking_budget":
    case "include_reasoning":
      return value;
    default:
      throw new Error(`Unsupported model thinking mode: ${value}`);
  }
}

function remoteCatalogFromSource(source: CatalogSource): RemoteModelCatalog {
  return {
    version: source.version,
    updatedAt: source.updated_at,
    description: source.description,
    providers: source.providers.map((provider) => ({
      providerId: provider.provider_id,
      providerName: provider.provider_name,
      baseUrl: provider.base_url,
      models: provider.models.map((model) => ({
        modelId: model.model_id,
        name: model.display_name,
        providerId: provider.provider_id,
        contextBudget: model.context_window,
        maxOutputTokens: model.max_output_tokens,
        reasoningSupport: model.thinking_support.type !== "none",
        thinkingSupport: {
          type: catalogThinkingSupportType(model.thinking_support.type),
          parameterName: model.thinking_support.parameter_name,
          levels: [...model.thinking_support.levels],
          defaultLevel: model.thinking_support.default,
        },
        pricingPer1M: {
          input: model.pricing_per_1m.input,
          output: model.pricing_per_1m.output,
        },
      })),
    })),
  };
}

export const REMOTE_MODEL_CATALOG = remoteCatalogFromSource(catalogSource);

export const DEFAULT_LOCAL_MODEL_ID = "gemma-4-E2B-it-qat-q4_0-gguf";

const LOCAL_MODEL_TEMPLATES: ModelTemplate[] = [
  {
    modelId: DEFAULT_LOCAL_MODEL_ID,
    name: "Gemma 4 E2B IT QAT Q4_0 GGUF",
    providerId: "local_model",
    contextBudget: 12_288,
    maxOutputTokens: 8192,
    reasoningSupport: false,
    thinkingSupport: {
      type: "none",
      parameterName: null,
      levels: [],
      defaultLevel: "off",
    },
    pricingPer1M: { input: 0, output: 0 },
  },
  {
    modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
    name: "Gemma 4 E4B (Local)",
    providerId: "local_model",
    contextBudget: 12_288,
    maxOutputTokens: 8192,
    reasoningSupport: false,
    thinkingSupport: {
      type: "none",
      parameterName: null,
      levels: [],
      defaultLevel: "off",
    },
    pricingPer1M: { input: 0, output: 0 },
  },
  {
    modelId: "gemma-4-12B-it-q8_0-gguf",
    name: "Gemma 4 12B (Local)",
    providerId: "local_model",
    contextBudget: 16384,
    maxOutputTokens: 16384,
    reasoningSupport: false,
    thinkingSupport: {
      type: "none",
      parameterName: null,
      levels: [],
      defaultLevel: "off",
    },
    pricingPer1M: { input: 0, output: 0 },
  },
];

export const SYSTEM_MODEL_TEMPLATES: ModelTemplate[] = [
  ...LOCAL_MODEL_TEMPLATES,
  ...REMOTE_MODEL_CATALOG.providers.flatMap((provider) => provider.models),
];

export function systemModelTemplatesForProvider(providerId: string) {
  const providerKey = providerId.trim().toLowerCase();
  return SYSTEM_MODEL_TEMPLATES.filter((template) => template.providerId === providerKey);
}

function systemModelTemplateForModel(providerId: string, modelId: string) {
  const providerKey = providerId.trim().toLowerCase();
  const modelKey = modelId.trim().toLowerCase();
  return SYSTEM_MODEL_TEMPLATES.find(
    (template) =>
      template.providerId === providerKey &&
      template.modelId.toLowerCase() === modelKey,
  );
}

export function parseModelIds(value: string) {
  return value
    .split(/[\n,]+/)
    .map((modelId) => modelId.trim())
    .filter(Boolean);
}

export function configuredProviderIsRunnable(provider: ConfiguredProvider) {
  const normalizedProviderId = provider.providerId.trim().toLowerCase().replace(/[\s-]+/g, "_");
  const isLocal = ["local", "local_model", "local_gemma"].includes(normalizedProviderId);
  return parseModelIds(provider.customModelIds).length > 0 &&
    (isLocal || provider.credentialConfigured !== false);
}

export function configuredModelOptions(configuredProviders: ConfiguredProvider[]) {
  const seen = new Set<string>();
  const options: ConfiguredModelOption[] = [];

  for (const provider of configuredProviders) {
    if (!configuredProviderIsRunnable(provider)) {
      continue;
    }
    for (const modelId of parseModelIds(provider.customModelIds)) {
      const key = `${provider.id}:${modelId}`;
      if (seen.has(key)) {
        continue;
      }

      seen.add(key);
      options.push({
        providerId: providerConfigurationId(provider.id),
        providerConfigId: providerConfigurationId(provider.id),
        providerType: providerTypeId(provider.providerId),
        providerName: provider.providerName || provider.providerId,
        modelId,
        label: `${provider.providerName || provider.providerId} / ${modelId}`,
        context: contextLabelForModel(provider.providerId, modelId),
        supportedReasoningLevels: supportedReasoningLevelsForModel(provider.providerId, modelId),
      });
    }
  }

  return options;
}

function providerRouteIdentity(value: string) {
  const normalized = value.trim().toLowerCase().replace(/[\s-]+/g, "_");
  return ["local", "local_gemma", "local_model", "gemma"].includes(normalized)
    ? "local_model"
    : normalized;
}

export function resolveConfiguredModelRoute(
  configuredProviders: ConfiguredProvider[],
  requestedProviderId: string,
  requestedModelId: string,
): ConfiguredModelOption | null {
  const providerId = requestedProviderId.trim();
  const modelId = requestedModelId.trim();
  if (!providerId || !modelId) return null;

  const options = configuredModelOptions(configuredProviders);
  const modelMatches = (option: ConfiguredModelOption) =>
    option.modelId.toLowerCase() === modelId.toLowerCase();
  const exact = options.find(
    (option) => option.providerConfigId === providerId && modelMatches(option),
  );
  if (exact) return exact;

  const requestedProviderType = providerRouteIdentity(providerId);
  const matches = options.filter(
    (option) =>
      providerRouteIdentity(option.providerType) === requestedProviderType &&
      modelMatches(option),
  );
  return matches.length === 1 ? matches[0] : null;
}

export function providerOptionsFromConfigured(configuredProviders: ConfiguredProvider[]) {
  const providers = new Map<string, string>();

  for (const provider of configuredProviders) {
    if (!configuredProviderIsRunnable(provider)) {
      continue;
    }
    if (!providers.has(provider.id)) {
      providers.set(provider.id, provider.providerName || provider.providerId);
    }
  }

  return [...providers.entries()].map(([id, label]) => ({ id, label }));
}

export function modelsForProvider(
  configuredProviders: ConfiguredProvider[],
  providerId: string,
) {
  return configuredModelOptions(configuredProviders).filter(
    (model) => model.providerId === providerId,
  );
}

export function defaultReasoningLevelForProvider(providerId: string): ReasoningLevel {
  const providerKey = reasoningCapabilityKey(providerId);
  const catalogDefault = REMOTE_MODEL_CATALOG.providers
    .find((provider) => reasoningCapabilityKey(provider.providerId) === providerKey)
    ?.models[0]?.thinkingSupport.defaultLevel;
  const normalizedCatalogDefault = normalizeReasoningLevel(catalogDefault);
  if (normalizedCatalogDefault) {
    return normalizedCatalogDefault;
  }
  if (providerKey.includes("google") || providerKey.includes("gemini")) {
    return "medium";
  }
  if (
    providerKey.includes("openai") ||
    providerKey.includes("gpt") ||
    providerKey.includes("anthropic") ||
    providerKey.includes("claude")
  ) {
    return "high";
  }
  if (
    providerKey.includes("local") ||
    providerKey.includes("native") ||
    providerKey.includes("gemma")
  ) {
    return "low";
  }
  return "medium";
}

export function resolveReasoningFallback(
  requested: ReasoningLevel | string | null | undefined,
  supported: readonly (ReasoningLevel | string)[],
): ReasoningLevel {
  const requestedRank = reasoningLevelRank(requested);
  let resolved: ReasoningLevel = "off";
  let resolvedRank = 0;

  const supportedLevels = supported.length > 0 ? supported : DEFAULT_REASONING_LEVELS;
  for (const level of supportedLevels) {
    const normalized = normalizeReasoningLevel(level);
    if (!normalized) {
      continue;
    }
    const rank = REASONING_LEVEL_RANKS[normalized];
    if (rank <= requestedRank && rank >= resolvedRank) {
      resolved = normalized;
      resolvedRank = rank;
    }
  }

  return resolved;
}

export function supportedReasoningLevelsForModel(providerId: string, modelId: string): ReasoningLevel[] {
  const providerKey = reasoningCapabilityKey(providerId);
  const modelKey = reasoningCapabilityKey(modelId);
  const template = systemModelTemplateForModel(providerId, modelId);

  // On-device models (the built-in `local_model` provider) get a plain
  // thinking on/off toggle — not a graded cloud effort budget.
  if (
    providerKey.includes("local_model") ||
    providerKey.includes("native") ||
    providerKey === "local" ||
    modelKey.includes("gemma_4") ||
    modelKey.includes("gemma4")
  ) {
    return [...LOCAL_REASONING_LEVELS];
  }

  if (template && !template.reasoningSupport) {
    return ["off"];
  }

  if (template && template.thinkingSupport.levels.length > 0) {
    const normalizedLevels = template.thinkingSupport.levels
      .map(normalizeReasoningLevel)
      .filter((level): level is ReasoningLevel => Boolean(level));
    return [...new Set(normalizedLevels)];
  }

  // Anthropic Claude — extended thinking across the full budget ladder.
  if (
    providerKey.includes("anthropic") ||
    providerKey.includes("claude") ||
    modelKey.includes("claude")
  ) {
    return [...DEFAULT_REASONING_LEVELS];
  }

  // OpenAI GPT / o-series — reasoning effort (minimal/low/medium/high).
  if (
    providerKey.includes("openai") ||
    modelKey.includes("gpt") ||
    modelKey.startsWith("o1") ||
    modelKey.startsWith("o3") ||
    modelKey.startsWith("o4")
  ) {
    return [...DEFAULT_REASONING_LEVELS];
  }

  // Google Gemini — thinking levels.
  if (
    providerKey.includes("gemini") ||
    providerKey.includes("google") ||
    modelKey.includes("gemini")
  ) {
    return [...DEFAULT_REASONING_LEVELS];
  }

  return [...DEFAULT_REASONING_LEVELS];
}

export function contextLabelForModel(providerId: string, modelId: string) {
  const template = systemModelTemplateForModel(providerId, modelId);
  return template ? formatContextBudget(template.contextBudget) : "provider-defined";
}

function reasoningLevelRank(level: ReasoningLevel | string | null | undefined) {
  return REASONING_LEVEL_RANKS[normalizeReasoningLevel(level) ?? "medium"];
}

function normalizeReasoningLevel(level: ReasoningLevel | string | null | undefined): ReasoningLevel | null {
  switch ((level ?? "").trim().toLowerCase()) {
    case "off":
      return "off";
    case "on":
      return "on";
    case "low":
      return "low";
    case "medium":
      return "medium";
    case "high":
      return "high";
    case "max":
      return "max";
    case "xhigh":
    case "x-high":
    case "extreme":
    case "ultra":
      return "max";
    default:
      return null;
  }
}

function reasoningCapabilityKey(value: string) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function formatContextBudget(tokens: number) {
  // Model vendors publish token windows in decimal units (for example,
  // 1,000,000 tokens). Reflect the catalog exactly instead of presenting
  // those values as binary memory sizes.
  if (tokens >= 1_000_000) {
    return `${Math.round(tokens / 1_000_000)}M`;
  }
  if (tokens >= 1_000) {
    return `${Math.round(tokens / 1_000)}K`;
  }
  return String(tokens);
}
