/* eslint-disable @next/next/no-img-element */

import { useState, useRef, useEffect, useMemo } from "react";
import { invoke } from "@/lib/invoke";
import {
  MAX_AGENT_MAX_OUTPUT_TOKENS,
  MIN_AGENT_MAX_OUTPUT_TOKENS,
  OUTPUT_TOKEN_STEP,
  defaultAgentPersonalityProfile,
  normalizeAgentMaxOutputTokens,
  normalizeAgentPersonalityProfile,
  type AgentPersonalityProfile,
} from "@/lib/agentPersonality";
import {
  modelsForProvider,
  providerOptionsFromConfigured,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import { useI18n } from "@/context/I18nContext";

type AgentProfileAgent = {
  id: string;
  name: string;
  description: string;
  systemPrompt: string;
  createdAt?: number;
  favorited?: boolean;
  image?: string | null;
  lastAccessedAt?: number;
  type?: "active" | "archived";
  endpoint?: {
    provider: string;
    modelId: string;
    customName?: string;
    customBaseUrl?: string;
  };
  personalityTemplate?: string;
  personalityProfile?: AgentPersonalityProfile;
};

type AgentModBadge = {
  id: string;
  name: string;
};

type InstalledMod = {
  id: string;
  name: string;
  description: string;
  isActive: boolean;
  agentConfigSchema?: AgentConfigSchema | null;
};

type AgentConfigSchemaPropertyType = "string" | "number" | "integer" | "boolean";

type AgentConfigSchemaProperty = {
  type?: AgentConfigSchemaPropertyType | AgentConfigSchemaPropertyType[];
  title?: string;
  description?: string;
  default?: unknown;
  enum?: unknown[];
  minimum?: number;
  maximum?: number;
  step?: number;
  "ui:widget"?: "grid-3x3" | string;
};

type AgentConfigSchema = {
  title?: string;
  properties?: Record<string, AgentConfigSchemaProperty>;
};

type GenericModConfigPanelProps = {
  mod: InstalledMod;
  currentValues: Record<string, unknown>;
  onChange: (key: string, value: unknown) => void;
};

type AgentProfileTemplateOption = {
  id: string;
  name: string;
  description: string;
  instructions: string;
  attributes: string[];
  origin: "system" | "custom";
};

type AgentSoulManifest = {
  display_name: string;
  origin_story: string;
  role: string;
  values: string[];
  hard_boundaries: string[];
  communication_style: string;
  self_description: string;
  immutable_truths: string[];
  version: number;
};

type AgentMemoryEntry = {
  id: number;
  memory_kind: string;
  scope: string;
  content: string;
  confidence: number;
};

type AgentIdentityContext = {
  soul: AgentSoulManifest;
  memories: AgentMemoryEntry[];
};

type PersonalityPartId = "identity" | "personality" | "relationship" | "modelBehavior" | "soul";

type AgentProfileViewProps = {
  agent: AgentProfileAgent;
  onBack: () => void;
  onUpdate: (agent: AgentProfileAgent) => void;
  onToggleArchive: (agent: AgentProfileAgent) => void;
  onDelete: (agent: AgentProfileAgent) => void;
  onRefreshImportedMemory?: () => void;
  onModBindingsChange?: (agentId: string, badges: AgentModBadge[]) => void;
  onOpenMods?: () => void;
  configuredProviders: ConfiguredProvider[];
  templateOptions: AgentProfileTemplateOption[];
};

const personalityPartOptions: { id: PersonalityPartId; label: string }[] = [
  { id: "identity", label: "Identity" },
  { id: "personality", label: "Personality" },
  { id: "relationship", label: "Relationship" },
  { id: "modelBehavior", label: "Model Behavior" },
  { id: "soul", label: "Soul Manifest" },
];

function getAgentConfigSchema(mod: InstalledMod) {
  return mod.agentConfigSchema ?? null;
}

function hasAgentConfigSchema(mod: InstalledMod) {
  const schema = getAgentConfigSchema(mod);
  return !!schema?.properties && Object.keys(schema.properties).length > 0;
}

function schemaTypeMatches(
  field: AgentConfigSchemaProperty,
  type: AgentConfigSchemaPropertyType,
) {
  return Array.isArray(field.type) ? field.type.includes(type) : field.type === type;
}

function valueOrDefault(
  currentValues: Record<string, unknown>,
  key: string,
  field: AgentConfigSchemaProperty,
) {
  return Object.prototype.hasOwnProperty.call(currentValues, key)
    ? currentValues[key]
    : field.default;
}

function safeFieldId(modId: string, key: string) {
  return `mod-config-${modId}-${key}`.replace(/[^a-zA-Z0-9_-]/g, "-");
}

function numericValue(value: unknown, fallback: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof fallback === "number" && Number.isFinite(fallback)) {
    return fallback;
  }
  return 0;
}

function GenericModConfigPanel({
  mod,
  currentValues,
  onChange,
}: GenericModConfigPanelProps) {
  const schema = getAgentConfigSchema(mod);
  const properties = schema?.properties;

  if (!properties || Object.keys(properties).length === 0) {
    return (
      <div className="py-4 text-center text-xs font-medium text-[var(--foreground-muted)]">
        This mod requires no additional configuration.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
      <div className="border-b border-[var(--border-soft)] pb-2">
        <h4 className="text-xs font-bold uppercase text-[var(--foreground)]">
          {schema.title || `${mod.name} Configuration`}
        </h4>
      </div>
      <div className="grid gap-4">
        {Object.entries(properties).map(([key, field]) => {
          const value = valueOrDefault(currentValues, key, field);
          const enumOptions = Array.isArray(field.enum)
            ? field.enum.filter((option): option is string => typeof option === "string")
            : [];
          const isNumberField = schemaTypeMatches(field, "number") || schemaTypeMatches(field, "integer");
          const isBooleanField = schemaTypeMatches(field, "boolean");
          const isStringField = schemaTypeMatches(field, "string");
          const fieldId = safeFieldId(mod.id, key);
          const label = field.title || key;
          const displayValue = numericValue(value, field.default);
          const stringValue = typeof value === "string" ? value : String(field.default ?? enumOptions[0] ?? "");
          const shouldRenderGrid3x3 =
            isStringField && field["ui:widget"] === "grid-3x3" && enumOptions.length >= 9;

          return (
            <div className="flex flex-col gap-1.5" key={key}>
              {/* Booleans carry their label beside the checkbox instead, so the
                  toggle names what it controls rather than its own on/off state. */}
              {!isBooleanField ? (
                <div className="flex items-center justify-between gap-3">
                  <label
                    className="text-xs font-semibold text-[var(--foreground)]"
                    htmlFor={fieldId}
                    id={`${fieldId}-label`}
                  >
                    {label}
                  </label>
                  {isNumberField ? (
                    <span className="shrink-0 text-[11px] font-bold text-[var(--foreground-muted)]">
                      {displayValue}
                    </span>
                  ) : null}
                </div>
              ) : null}
              {field.description ? (
                <p className="text-[11px] font-medium leading-relaxed text-[var(--foreground-muted)]">
                  {field.description}
                </p>
              ) : null}

              {shouldRenderGrid3x3 ? (
                <div
                  aria-labelledby={`${fieldId}-label`}
                  className="mt-2 grid grid-cols-3 gap-2"
                  role="group"
                >
                  {enumOptions.map((option) => {
                    const [firstWord, ...remainingWords] = option.split(" ");
                    const isActive = stringValue === option;
                    return (
                      <button
                        aria-label={option}
                        aria-pressed={isActive}
                        className={`flex min-h-[56px] flex-col items-center justify-center rounded-[var(--radius-sm)] border p-3.5 text-center text-[10px] font-bold leading-tight transition-all ${
                          isActive
                            ? "border-[var(--accent)] bg-[var(--fill-selected)] text-[var(--foreground)] shadow-[0_0_0_1px_var(--accent)] ring-1 ring-[var(--accent)]"
                            : "border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                        }`}
                        key={option}
                        onClick={() => onChange(key, option)}
                        type="button"
                      >
                        <span className="block">{firstWord}</span>
                        <span className="mt-0.5 block text-[8px] font-medium opacity-80">
                          {remainingWords.join(" ")}
                        </span>
                      </button>
                    );
                  })}
                </div>
              ) : null}

              {isStringField && enumOptions.length > 0 && !shouldRenderGrid3x3 ? (
                <select
                  className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-2.5 py-1.5 text-xs text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  id={fieldId}
                  onChange={(event) => onChange(key, event.target.value)}
                  value={stringValue}
                >
                  {enumOptions.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              ) : null}

              {isNumberField && typeof field.minimum === "number" && typeof field.maximum === "number" ? (
                <input
                  className="h-1 w-full cursor-pointer appearance-none rounded-lg bg-[var(--border-soft)] accent-[var(--accent)]"
                  id={fieldId}
                  max={field.maximum}
                  min={field.minimum}
                  onChange={(event) => onChange(key, Number(event.target.value))}
                  step={typeof field.step === "number" ? field.step : schemaTypeMatches(field, "integer") ? 1 : 0.05}
                  type="range"
                  value={numericValue(value, field.default)}
                />
              ) : null}

              {isNumberField && (typeof field.minimum !== "number" || typeof field.maximum !== "number") ? (
                <input
                  className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  id={fieldId}
                  onChange={(event) => onChange(key, Number(event.target.value))}
                  step={typeof field.step === "number" ? field.step : schemaTypeMatches(field, "integer") ? 1 : 0.05}
                  type="number"
                  value={numericValue(value, field.default)}
                />
              ) : null}

              {isBooleanField ? (
                <label className="mt-1 flex cursor-pointer items-center gap-3" htmlFor={fieldId}>
                  <input
                    checked={value === true}
                    className="h-4 w-4 accent-[var(--accent)]"
                    id={fieldId}
                    onChange={(event) => onChange(key, event.target.checked)}
                    type="checkbox"
                  />
                  <span className="text-xs font-semibold text-[var(--foreground)]">{label}</span>
                </label>
              ) : null}

              {isStringField && enumOptions.length === 0 ? (
                <input
                  className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  id={fieldId}
                  onChange={(event) => onChange(key, event.target.value)}
                  type="text"
                  value={typeof value === "string" ? value : String(value ?? "")}
                />
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function templateTraitLabel(value: string) {
  return value
    .replaceAll("-", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function buildTemplateInstructions(template: AgentProfileTemplateOption) {
  const traits = template.attributes.map((attribute) => `- ${templateTraitLabel(attribute)}`);
  return ["Core Instructions", template.instructions, "", "Style and Behavior", ...traits].join("\n");
}

function getPersonalityPartJson(
  part: PersonalityPartId,
  profile: AgentPersonalityProfile,
  identityContext: AgentIdentityContext | null,
) {
  const value = part === "soul" ? identityContext?.soul ?? null : profile[part as Exclude<PersonalityPartId, "soul">];
  return JSON.stringify(value, null, 2);
}

function activeBadgesForBoundMods(mods: InstalledMod[], boundModIds: string[]) {
  const bound = new Set(boundModIds);
  return mods
    .filter((mod) => mod.isActive && bound.has(mod.id))
    .map((mod) => ({ id: mod.id, name: mod.name }));
}

function AgentProfileTopBar({
  onBack,
  onRefreshImportedMemory,
  t,
}: {
  onBack: () => void;
  onRefreshImportedMemory?: () => void;
  t: (key: string) => string;
}) {
  return <div className="flex items-center justify-between gap-4">
    <button
      aria-label={t("common.back")}
      className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-sm font-medium transition-colors hover:bg-[var(--fill-hover)]"
      onClick={onBack}
    >
      ← {t("common.back")}
    </button>
    <div className="flex items-center gap-3">
      {onRefreshImportedMemory ? (
        <button
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm font-medium transition-colors hover:bg-[var(--fill-hover)]"
          onClick={onRefreshImportedMemory}
          type="button"
        >
          {t("sprint_299.import_refresh.profile_action")}
        </button>
      ) : null}
      <p className="text-xs text-[var(--foreground-subtle)]">
        {t("sprint_299.import_refresh.changes_saved")}
      </p>
    </div>
  </div>;
}

export default function AgentProfileView({
  agent,
  onBack,
  onUpdate,
  onToggleArchive,
  onDelete,
  onRefreshImportedMemory,
  onModBindingsChange,
  onOpenMods,
  configuredProviders,
  templateOptions,
}: AgentProfileViewProps) {
  const { t } = useI18n();
  const [name, setName] = useState(agent.name);
  const [isEditingName, setIsEditingName] = useState(false);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const isArchived = agent.type === "archived";
  const [imagePreview, setImagePreview] = useState<string | null>(agent.image || null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [description, setDescription] = useState(agent.description);
  const [provider, setProvider] = useState(agent.endpoint?.provider ?? "gemini");
  const [modelId, setModelId] = useState(agent.endpoint?.modelId ?? "gemini-3-flash");
  const [templateId, setTemplateId] = useState(
    agent.personalityProfile?.template?.id ?? agent.personalityTemplate ?? templateOptions[0]?.id ?? "",
  );
  const [isEditingDesc, setIsEditingDesc] = useState(false);
  const descInputRef = useRef<HTMLTextAreaElement>(null);

  const [identityContext, setIdentityContext] = useState<AgentIdentityContext | null>(null);
  const currentPersonalityProfile = useMemo(() => {
    const selectedTemplate = templateOptions.find((template) => template.id === templateId);
    if (agent.personalityProfile) {
      return normalizeAgentPersonalityProfile({
        name: agent.name,
        description: agent.description,
        profile: agent.personalityProfile,
        providerId: provider,
      });
    }

    return defaultAgentPersonalityProfile({
        name: agent.name,
        description: agent.description,
        templateId,
        templateName: selectedTemplate?.name,
        templateOrigin: selectedTemplate?.origin,
        providerId: provider,
      });
  }, [agent.description, agent.name, agent.personalityProfile, provider, templateId, templateOptions]);
  const [isPersonalityOpen, setIsPersonalityOpen] = useState(false);
  const [personalityPart, setPersonalityPart] = useState<PersonalityPartId>("identity");
  const [personalityDraft, setPersonalityDraft] = useState(
    getPersonalityPartJson("identity", currentPersonalityProfile, null),
  );
  const [isDirectEditEnabled, setIsDirectEditEnabled] = useState(false);
  const [personalityEditError, setPersonalityEditError] = useState("");
  const [personalitySaveMessage, setPersonalitySaveMessage] = useState("");
  const [isSavingPersonalityPart, setIsSavingPersonalityPart] = useState(false);
  const [installedMods, setInstalledMods] = useState<InstalledMod[]>([]);
  const [boundModIds, setBoundModIds] = useState<string[]>([]);
  const [modBindingMessage, setModBindingMessage] = useState("");
  const [pendingModId, setPendingModId] = useState<string | null>(null);
  const [activeConfigModId, setActiveConfigModId] = useState<string | null>(null);
  const activeTemplate = templateOptions.find((template) => template.id === templateId);
  const globallyEnabledMods = useMemo(
    () => installedMods.filter((mod) => mod.isActive),
    [installedMods],
  );
  const activeModBadges = useMemo(
    () => activeBadgesForBoundMods(installedMods, boundModIds),
    [boundModIds, installedMods],
  );
  const activeConfigMod = useMemo(() => {
    if (!activeConfigModId || !boundModIds.includes(activeConfigModId)) {
      return null;
    }

    const mod = globallyEnabledMods.find((candidate) => candidate.id === activeConfigModId);
    return mod && hasAgentConfigSchema(mod) ? mod : null;
  }, [activeConfigModId, boundModIds, globallyEnabledMods]);
  const configuredProviderOptions = providerOptionsFromConfigured(configuredProviders);
  const providerOptions = configuredProviderOptions.some((option) => option.id === provider) || !provider
    ? configuredProviderOptions
    : [{ id: provider, label: t("common.saved_not_configured", { name: provider }) }, ...configuredProviderOptions];
  const configuredModels = modelsForProvider(configuredProviders, provider);
  const modelOptions = configuredModels.some((model) => model.modelId === modelId)
    ? configuredModels
    : modelId
      ? [
          {
            providerId: provider,
            providerName: provider,
            modelId,
            label: t("common.saved_not_configured", { name: modelId }),
            context: "provider-defined",
          },
          ...configuredModels,
        ]
      : configuredModels;
  const maxOutputTokens = normalizeAgentMaxOutputTokens(
    currentPersonalityProfile.modelBehavior?.maxOutputTokens,
    provider,
  );

  useEffect(() => {
    if (isEditingName && nameInputRef.current) {
      nameInputRef.current.focus();
    }
  }, [isEditingName]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setName(agent.name);
      setDescription(agent.description);
      setProvider(agent.endpoint?.provider ?? "gemini");
      setModelId(agent.endpoint?.modelId ?? "gemini-3-flash");
      setTemplateId(agent.personalityProfile?.template?.id ?? agent.personalityTemplate ?? templateOptions[0]?.id ?? "");
      setImagePreview(agent.image || null);
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [agent.description, agent.endpoint?.modelId, agent.endpoint?.provider, agent.id, agent.image, agent.name, agent.personalityProfile?.template?.id, agent.personalityTemplate, templateOptions]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setPersonalityDraft(getPersonalityPartJson(personalityPart, currentPersonalityProfile, identityContext));
      setIsDirectEditEnabled(false);
      setPersonalityEditError("");
      setPersonalitySaveMessage("");
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [personalityPart, identityContext, currentPersonalityProfile]);

  useEffect(() => {
    let cancelled = false;
    void invoke<AgentIdentityContext | null>("hydrate_agent_prompt_context", {
      request: {
        agent_id: agent.id,
        display_name: agent.name,
        role: agent.description,
        description: agent.description,
        system_prompt: agent.systemPrompt || agent.description,
        latest_message: "",
        provider_id: agent.endpoint?.provider ?? "",
        model_id: agent.endpoint?.modelId ?? "",
      },
    }).then((context) => {
      if (!cancelled && context) {
        setIdentityContext(context);
      }
    }).catch((error) => {
      if (!cancelled) {
        setPersonalityEditError(
          error instanceof Error ? error.message : "Unable to load native agent context.",
        );
      }
    });
    return () => {
      cancelled = true;
    };
  }, [agent.description, agent.endpoint?.modelId, agent.endpoint?.provider, agent.id, agent.name, agent.systemPrompt]);

  useEffect(() => {
    let cancelled = false;

    async function loadAgentMods() {
      setModBindingMessage("");
      try {
        const [mods, boundIds] = await Promise.all([
          invoke<InstalledMod[]>("list_installed_mods"),
          invoke<string[]>("get_agent_mods", { agentId: agent.id }),
        ]);
        if (!cancelled) {
          setInstalledMods(mods);
          setBoundModIds(boundIds);
        }
      } catch (error) {
        if (!cancelled) {
          setInstalledMods([]);
          setBoundModIds([]);
          setModBindingMessage(
            error instanceof Error ? error.message : "Unable to load capability mods.",
          );
        }
      }
    }

    void loadAgentMods();

    return () => {
      cancelled = true;
    };
  }, [agent.id]);

  const handleImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    const reader = new FileReader();
    reader.onload = (event) => {
      const img = new Image();
      img.src = event.target?.result as string;
      img.onload = () => {
        const canvas = document.createElement('canvas');
        const size = Math.min(img.width, img.height);
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        if (!ctx) return;

        ctx.drawImage(
          img,
          (img.width - size) / 2,
          (img.height - size) / 2,
          size,
          size,
          0,
          0,
          size,
          size
        );

        const dataUrl = canvas.toDataURL('image/jpeg', 0.9);
        setImagePreview(dataUrl);
        onUpdate({ ...agent, image: dataUrl });
      };
    };
    reader.readAsDataURL(file);
  };

  const handleImageDelete = () => {
    setImagePreview(null);
    onUpdate({ ...agent, image: null });
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const handleNameCommit = () => {
    setIsEditingName(false);
    if (name.trim() && name !== agent.name) {
      onUpdate({ ...agent, name: name.trim() });
    } else {
      setName(agent.name);
    }
  };

  useEffect(() => {
    if (isEditingDesc && descInputRef.current) {
      descInputRef.current.focus();
    }
  }, [isEditingDesc]);

  const handleDescCommit = () => {
    setIsEditingDesc(false);
    if (description !== agent.description) {
      onUpdate({ ...agent, description });
    }
  };

  const handleProviderChange = (
    nextProvider: string,
  ) => {
    const fallbackModel = modelsForProvider(configuredProviders, nextProvider)[0]?.modelId ?? "";
    setProvider(nextProvider);
    setModelId(fallbackModel);
    onUpdate({
      ...agent,
      endpoint: {
        ...agent.endpoint,
        provider: nextProvider,
        modelId: fallbackModel,
      },
    });
  };

  const handleModelChange = (nextModelId: string) => {
    setModelId(nextModelId);
    onUpdate({
      ...agent,
      endpoint: {
        ...agent.endpoint,
        provider,
        modelId: nextModelId,
      },
    });
  };

  const handleTemplateChange = (nextTemplateId: string) => {
    const nextTemplate = templateOptions.find((template) => template.id === nextTemplateId);
    if (!nextTemplate) return;
    const modConfigurations = currentPersonalityProfile.mod_configurations;
    const nextProfile: AgentPersonalityProfile = {
      ...defaultAgentPersonalityProfile({
        name: agent.name,
        description,
        templateId: nextTemplate.id,
        templateName: nextTemplate.name,
        templateOrigin: nextTemplate.origin,
        traits: nextTemplate.attributes.map(templateTraitLabel),
        providerId: provider,
        maxOutputTokens,
      }),
      ...(modConfigurations ? { mod_configurations: modConfigurations } : {}),
    };
    setTemplateId(nextTemplate.id);
    onUpdate({
      ...agent,
      description,
      systemPrompt: buildTemplateInstructions(nextTemplate),
      personalityTemplate: nextTemplate.id,
      personalityProfile: nextProfile,
    });
  };

  const handleToggleConfigureMod = (modId: string) => {
    setActiveConfigModId((currentModId) => (currentModId === modId ? null : modId));
  };

  const handleAgentModConfigUpdate = (modId: string, key: string, value: unknown) => {
    const currentModConfig = currentPersonalityProfile.mod_configurations?.[modId] ?? {};
    const nextProfile: AgentPersonalityProfile = {
      ...currentPersonalityProfile,
      mod_configurations: {
        ...currentPersonalityProfile.mod_configurations,
        [modId]: {
          ...currentModConfig,
          [key]: value,
        },
      },
    };

    setModBindingMessage("");
    onUpdate({
      ...agent,
      personalityTemplate: nextProfile.template?.id ?? agent.personalityTemplate,
      personalityProfile: nextProfile,
    });
  };

  const handleMaxOutputTokensChange = (nextValue: number) => {
    const nextMaxOutputTokens = normalizeAgentMaxOutputTokens(nextValue, provider);
    const nextProfile: AgentPersonalityProfile = {
      ...currentPersonalityProfile,
      modelBehavior: {
        ...currentPersonalityProfile.modelBehavior,
        maxOutputTokens: nextMaxOutputTokens,
      },
    };

    onUpdate({
      ...agent,
      personalityTemplate: nextProfile.template?.id ?? agent.personalityTemplate,
      personalityProfile: nextProfile,
    });
  };

  const handleSavePersonalityPart = async () => {
    setPersonalityEditError("");
    setPersonalitySaveMessage("");
    setIsSavingPersonalityPart(true);
    try {
      const parsed = JSON.parse(personalityDraft) as unknown;
      if (personalityPart === "soul") {
        const manifest = parsed as AgentSoulManifest;
        const savedManifest = await invoke<AgentSoulManifest | null>("update_agent_soul_manifest", {
          request: { manifest },
        });
        if (!savedManifest) {
          throw new Error("Native runtime did not return the saved soul manifest.");
        }
        setIdentityContext((current) => current ? { ...current, soul: savedManifest } : current);
      } else {
        const nextProfile = {
          ...currentPersonalityProfile,
          [personalityPart]: parsed,
        } as AgentPersonalityProfile;
        onUpdate({
          ...agent,
          personalityTemplate: nextProfile.template?.id ?? agent.personalityTemplate,
          personalityProfile: nextProfile,
        });
      }
      setPersonalitySaveMessage("Saved.");
      setIsDirectEditEnabled(false);
    } catch (error) {
      setPersonalityEditError(error instanceof Error ? error.message : "Invalid JSON. Check the section and try again.");
    } finally {
      setIsSavingPersonalityPart(false);
    }
  };

  const handleToggleAgentMod = async (mod: InstalledMod, shouldBind: boolean) => {
    setPendingModId(mod.id);
    setModBindingMessage("");
    const nextBoundModIds = shouldBind
      ? [...new Set([...boundModIds, mod.id])]
      : boundModIds.filter((id) => id !== mod.id);

    try {
      await invoke<void>(
      shouldBind ? "bind_mod_to_agent" : "unbind_mod_to_agent",
        { agentId: agent.id, modId: mod.id },
      );
      setBoundModIds(nextBoundModIds);
      if (!shouldBind && activeConfigModId === mod.id) {
        setActiveConfigModId(null);
      }
      onModBindingsChange?.(agent.id, activeBadgesForBoundMods(installedMods, nextBoundModIds));
    } catch (error) {
      setModBindingMessage(
        error instanceof Error ? error.message : "Unable to update capability mod binding.",
      );
    } finally {
      setPendingModId(null);
    }
  };

  return (
    <div className="mx-auto flex h-full w-full max-w-2xl flex-col gap-8 p-8 animate-in fade-in zoom-in-95 duration-200">
      <AgentProfileTopBar onBack={onBack} onRefreshImportedMemory={onRefreshImportedMemory} t={t} />

      {/* Identity header */}
      <div className="flex items-start gap-5">
        <div className="flex shrink-0 flex-col items-center gap-2">
          <div className="relative h-24 w-24 overflow-hidden rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--accent-background)]">
            {imagePreview ? (
              <img src={imagePreview} alt={name} className="h-full w-full object-cover" />
            ) : (
              <div className="flex h-full w-full items-center justify-center text-[var(--foreground-subtle)]">
                <svg aria-hidden="true" className="h-12 w-12" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
                  <rect height="12" width="16" x="4" y="8" rx="1" />
                  <path d="M9 13h.01M15 13h.01M12 21v-1M12 8V5m0 0a2 2 0 1 1 0-4 2 2 0 0 1 0 4Z" />
                </svg>
              </div>
            )}
            {imagePreview && (
              <button
                aria-label={t("agents.profile.remove_image")}
                onClick={handleImageDelete}
                className="absolute right-1 top-1 flex h-6 w-6 items-center justify-center rounded-full border border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground)] shadow transition-colors hover:bg-[var(--fill-hover)]"
              >
                <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M6 18L18 6M6 6l12 12" /></svg>
              </button>
            )}
          </div>
          <input
            type="file"
            accept="image/*"
            className="hidden"
            ref={fileInputRef}
            onChange={handleImageUpload}
          />
          <button
            onClick={() => fileInputRef.current?.click()}
            className="text-xs font-medium text-[var(--accent)] transition-colors hover:underline"
          >
            {imagePreview ? t("agents.profile.change_image") : t("agents.profile.upload_image")}
          </button>
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-2 pt-1">
          {isEditingName ? (
            <input
              ref={nameInputRef}
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={handleNameCommit}
              onKeyDown={(e) => { if (e.key === 'Enter') handleNameCommit(); }}
              className="border-b border-[var(--border-strong)] bg-transparent text-xl font-semibold tracking-tight focus:outline-none"
            />
          ) : (
            <div className="group flex cursor-pointer items-center gap-2" onClick={() => setIsEditingName(true)}>
              <h1 className="truncate text-xl font-semibold tracking-tight">{name}</h1>
              <button aria-label={t("agents.profile.edit_name")} className="rounded p-1 opacity-0 transition-colors hover:bg-[var(--fill-hover)] group-hover:opacity-100">
                <svg className="h-4 w-4 text-[var(--foreground-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" /></svg>
              </button>
            </div>
          )}
          {activeModBadges.length > 0 ? (
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              {activeModBadges.map((mod) => (
                <span
                  className="max-w-36 truncate rounded-full border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-0.5 text-[10px] font-semibold text-[var(--foreground-muted)]"
                  key={mod.id}
                  title={mod.name}
                >
                  {mod.name}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      </div>

      {/* Editable sections */}
      <div className="flex flex-col gap-8">
        <div className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold text-[var(--foreground)]">{t("agents.new_agent_dialog.description_label")}</h3>
          <div className="relative rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
            <textarea
              ref={descInputRef}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              onBlur={handleDescCommit}
              onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleDescCommit(); descInputRef.current?.blur(); } }}
              aria-label={t("agents.new_agent_dialog.description_label")}
              className="h-24 w-full resize-none bg-transparent text-sm font-medium leading-relaxed tracking-tight text-[var(--foreground)] focus:outline-none"
              placeholder={t("agents.new_agent_dialog.description_placeholder")}
            />
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold text-[var(--foreground)]">{t("agents.profile.configuration")}</h3>
          <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
            <div className="grid gap-3">
              <label className="grid gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.profile.provider")}
                </span>
                <select
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  onChange={(event) =>
                    handleProviderChange(
                      event.target.value,
                    )
                  }
                  value={provider}
                  disabled={providerOptions.length === 0}
                >
                  {providerOptions.length === 0 ? (
                    <option value={provider}>{t("agents.profile.configure_model_first")}</option>
                  ) : (
                    providerOptions.map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.label}
                      </option>
                    ))
                  )}
                </select>
              </label>

              <label className="grid gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.profile.model_id")}
                </span>
                {modelOptions.length === 0 ? (
                  <input
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                    disabled
                    onChange={(event) => setModelId(event.target.value)}
                    placeholder={t("agents.profile.configure_model_first")}
                    value={modelId}
                  />
                ) : (
                  <select
                    className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                    onChange={(event) => handleModelChange(event.target.value)}
                    value={modelId}
                  >
                    {modelOptions.map((model) => (
                      <option key={`${model.providerId}-${model.modelId}`} value={model.modelId}>
                        {model.label}
                      </option>
                    ))}
                  </select>
                )}
              </label>

              <label className="grid gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.profile.template")}
                </span>
                <select
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  onChange={(event) => handleTemplateChange(event.target.value)}
                  value={templateId}
                  disabled={templateOptions.length === 0}
                >
                  {templateOptions.length === 0 ? (
                    <option value="">{t("agents.profile.no_templates")}</option>
                  ) : (
                    templateOptions.map((template) => (
                      <option key={template.id} value={template.id}>
                        {template.name} ({template.origin})
                      </option>
                    ))
                  )}
                </select>
                {activeTemplate && (
                  <p className="text-[10px] font-medium leading-relaxed text-[var(--foreground-muted)]">
                    {activeTemplate.description}
                  </p>
                )}
              </label>

              <div className="mt-2 grid gap-2 border-t border-[var(--border-soft)] pt-4">
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <h4 className="text-xs font-semibold text-[var(--foreground)]">
                      {t("agents.profile.maximum_output")}
                    </h4>
                    <p className="mt-1 text-[10px] font-medium leading-relaxed text-[var(--foreground-muted)]">
                      {t("agents.profile.maximum_output_help")}
                    </p>
                  </div>
                  <span className="shrink-0 rounded-[var(--radius-sm)] bg-[var(--fill-active)] px-2 py-0.5 text-[10px] font-bold text-[var(--accent)]">
                    {t("agents.profile.tokens", { count: maxOutputTokens.toLocaleString() })}
                  </span>
                </div>
                <input
                  aria-label={t("agents.profile.maximum_output_tokens")}
                  className="h-1 w-full cursor-pointer appearance-none rounded-lg bg-[var(--border-soft)] accent-[var(--accent)]"
                  max={MAX_AGENT_MAX_OUTPUT_TOKENS}
                  min={MIN_AGENT_MAX_OUTPUT_TOKENS}
                  onChange={(event) => handleMaxOutputTokensChange(Number(event.target.value))}
                  step={OUTPUT_TOKEN_STEP}
                  type="range"
                  value={maxOutputTokens}
                />
                <div className="flex justify-between text-[9px] font-semibold text-[var(--foreground-subtle)]">
                  <span>1K</span>
                  <span>2K</span>
                  <span>4K</span>
                  <span>8K</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold text-[var(--foreground)]">{t("agents.profile.active_mods")}</h3>
          <p className="text-xs font-medium leading-relaxed text-[var(--foreground-muted)]">
            {t("agents.profile.active_mods_help")}
          </p>
          <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
            {globallyEnabledMods.length > 0 ? (
              <div className="grid gap-2">
                {globallyEnabledMods.map((mod) => {
                  const checked = boundModIds.includes(mod.id);
                  const hasConfig = hasAgentConfigSchema(mod);
                  const isCurrentlyConfiguring = checked && hasConfig && activeConfigModId === mod.id;
                  return (
                    <div
                      className={`grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 rounded-[var(--radius-sm)] border px-3 py-2 transition-all ${
                        isCurrentlyConfiguring
                          ? "border-[var(--accent)] bg-[var(--fill-selected)]"
                          : "border-[var(--border-soft)] bg-[var(--accent-background)]"
                      }`}
                      key={mod.id}
                    >
                      <input
                        aria-label={t(checked ? "agents.profile.unbind_mod" : "agents.profile.bind_mod", { name: mod.name })}
                        checked={checked}
                        className="h-4 w-4 accent-[var(--accent)]"
                        disabled={pendingModId === mod.id}
                        onChange={(event) => handleToggleAgentMod(mod, event.target.checked)}
                        type="checkbox"
                      />
                      <div className="min-w-0 pr-2">
                        <span className="block truncate text-xs font-semibold text-[var(--foreground)]">
                          {mod.name}
                        </span>
                        <span className="mt-1 line-clamp-1 block text-[11px] font-medium leading-relaxed text-[var(--foreground-muted)]">
                          {mod.description}
                        </span>
                      </div>
                      {checked && hasConfig ? (
                        <button
                          aria-label={
                            isCurrentlyConfiguring
                              ? t("agents.profile.done_configuring_mod", { name: mod.name })
                              : t("agents.profile.configure_mod", { name: mod.name })
                          }
                          className={`rounded-[var(--radius-sm)] border px-2 py-1 text-[11px] font-semibold transition-colors ${
                            isCurrentlyConfiguring
                              ? "border-[var(--accent)] bg-[var(--accent)] text-white hover:bg-[var(--accent-hover)]"
                              : "border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                          }`}
                          disabled={pendingModId === mod.id}
                          onClick={() => handleToggleConfigureMod(mod.id)}
                          type="button"
                        >
                          {isCurrentlyConfiguring ? t("common.confirm") : t("agents.profile.configure")}
                        </button>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="flex flex-col items-start gap-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-3">
                <p className="text-xs font-medium leading-relaxed text-[var(--foreground-muted)]">
                  {t("agents.profile.no_active_mods")}
                </p>
                {onOpenMods ? (
                  <button
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                    onClick={onOpenMods}
                    type="button"
                  >
                    {t("agents.profile.open_mods")}
                  </button>
                ) : null}
              </div>
            )}
            {modBindingMessage ? (
              <p className="mt-3 text-xs font-medium leading-relaxed text-[var(--destructive)]">
                {modBindingMessage}
              </p>
            ) : null}
            {activeConfigMod ? (
              <div className="mt-4 border-t border-[var(--border-soft)] pt-4">
                <GenericModConfigPanel
                  currentValues={currentPersonalityProfile.mod_configurations?.[activeConfigMod.id] ?? {}}
                  mod={activeConfigMod}
                  onChange={(key, value) => handleAgentModConfigUpdate(activeConfigMod.id, key, value)}
                />
              </div>
            ) : null}
          </div>
        </div>

        {identityContext && (
          <div className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold text-[var(--foreground)]">Inner Life</h3>
            <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
              <div className="grid gap-4">
                <div>
                  <div className="text-[10px] font-semibold text-[var(--foreground-muted)]">
                    Soul v{identityContext.soul.version}
                  </div>
                  <p className="mt-1 text-sm font-semibold leading-relaxed text-[var(--foreground)]">
                    {identityContext.soul.self_description}
                  </p>
                  <p className="mt-2 text-xs font-medium leading-relaxed text-[var(--foreground-muted)]">
                    {identityContext.soul.communication_style}
                  </p>
                </div>

                <div className="grid gap-2">
                  <div className="text-[10px] font-semibold text-[var(--foreground-muted)]">
                    Immutable Truths
                  </div>
                  <div className="grid gap-1.5">
                    {identityContext.soul.immutable_truths.slice(0, 4).map((truth) => (
                      <div key={truth} className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2 text-xs font-medium leading-relaxed text-[var(--foreground)]">
                        {truth}
                      </div>
                    ))}
                  </div>
                </div>

                <div className="grid gap-2">
                  <div className="text-[10px] font-semibold text-[var(--foreground-muted)]">
                    Durable Memories
                  </div>
                  <div className="grid gap-1.5">
                    {identityContext.memories.length ? (
                      identityContext.memories.slice(0, 5).map((memory) => (
                        <div key={memory.id} className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2">
                          <div className="text-xs font-medium leading-relaxed text-[var(--foreground)]">
                            {memory.content}
                          </div>
                        </div>
                      ))
                    ) : (
                      <div className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2 text-xs font-medium text-[var(--foreground-muted)]">
                        No durable memories have been captured yet.
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        <div className="flex flex-col gap-2">
          <button
            className="flex items-center justify-between border border-[var(--border-soft)] bg-[var(--background)] px-4 py-3 text-left text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--accent-background)]"
            onClick={() => setIsPersonalityOpen((value) => !value)}
            type="button"
          >
            <span>{t("agents.profile.advanced_personality")}</span>
            <span aria-hidden="true">{isPersonalityOpen ? "-" : "+"}</span>
          </button>

          {isPersonalityOpen && (
            <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
              <div className="grid gap-3">
                <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
                  <label className="grid gap-1.5">
                    <span className="text-xs font-medium text-[var(--foreground-muted)]">
                      Personality part
                    </span>
                    <select
                      className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                      onChange={(event) => setPersonalityPart(event.target.value as PersonalityPartId)}
                      value={personalityPart}
                    >
                      {personalityPartOptions.map((part) => (
                        <option key={part.id} value={part.id}>
                          {part.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button
                    className="inline-flex h-10 items-center justify-center gap-2 border border-[var(--border-strong)] bg-[var(--background)] px-3 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--accent-background)]"
                    onClick={() => {
                      setIsDirectEditEnabled(false);
                      setPersonalityDraft(getPersonalityPartJson(personalityPart, currentPersonalityProfile, identityContext));
                      setPersonalityEditError("");
                      setPersonalitySaveMessage("");
                    }}
                    type="button"
                  >
                    Reset
                  </button>
                </div>

                {!isDirectEditEnabled ? (
                  <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--destructive-background)] p-3">
                    <div className="text-[11px] font-semibold text-[var(--destructive)]">
                      Before you edit
                    </div>
                    <p className="mt-2 text-xs font-medium leading-relaxed text-[var(--foreground-muted)]">
                      These records shape the agent&apos;s identity, boundaries, and memory context. Direct edits can change behavior in surprising ways if the JSON is malformed or the meaning becomes inconsistent.
                    </p>
                    <button
                      className="mt-3 border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-[11px] font-medium text-[var(--destructive)] transition-colors hover:bg-[var(--fill-hover)]"
                      onClick={() => setIsDirectEditEnabled(true)}
                      type="button"
                    >
                      Enable direct editing
                    </button>
                  </div>
                ) : (
                  <div className="border border-[var(--border-strong)] bg-[var(--accent-background)] p-3 text-xs font-medium leading-relaxed text-[var(--foreground-muted)]">
                    Editing is enabled for this section. Keep valid JSON and preserve the fields this agent relies on.
                  </div>
                )}

                <textarea
                  className="min-h-64 w-full resize-y border border-[var(--border-strong)] bg-[var(--background)] p-3 font-mono text-xs leading-5 text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)] disabled:text-[var(--foreground-muted)]"
                  disabled={!isDirectEditEnabled}
                  onChange={(event) => {
                    setPersonalityDraft(event.target.value);
                    setPersonalityEditError("");
                    setPersonalitySaveMessage("");
                  }}
                  value={personalityDraft}
                />

                <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-h-5 text-xs font-medium leading-relaxed">
                    {personalityEditError ? (
                      <span className="text-[var(--destructive)]">{personalityEditError}</span>
                    ) : personalitySaveMessage ? (
                      <span className="text-[var(--accent)]">{personalitySaveMessage}</span>
                    ) : (
                      <span className="text-[var(--foreground-muted)]">
                        {personalityPart === "soul"
                          ? "Soul manifest edits are re-signed before saving."
                          : "Agent personality profile edits save through the agent configuration record."}
                      </span>
                    )}
                  </div>
                  {isDirectEditEnabled && personalityDraft !== getPersonalityPartJson(personalityPart, currentPersonalityProfile, identityContext) && (
                    <button
                      className="bg-[var(--inverse-background)] px-4 py-3 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:bg-[var(--accent-background)] disabled:text-[var(--foreground-muted)]"
                      disabled={isSavingPersonalityPart}
                      onClick={handleSavePersonalityPart}
                      type="button"
                    >
                      {isSavingPersonalityPart ? "Saving..." : "Save personality part"}
                    </button>
                  )}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Footer */}
      <div className="mt-auto border-t border-[var(--border-soft)] pt-6">
        <div className="flex flex-col gap-3 sm:flex-row sm:justify-end">
          <button
            onClick={() => onToggleArchive(agent)}
            className={`text-sm font-medium px-4 py-3 border transition-colors ${
              isArchived
                ? "bg-[var(--inverse-background)] text-[var(--inverse-foreground)] border-transparent hover:bg-[var(--accent-hover)]"
                : "bg-[var(--background)] text-[var(--foreground)] border-[var(--border-strong)] hover:bg-[var(--fill-hover)]"
            }`}
          >
            {isArchived ? t("agents.profile.reactivate") : t("agents.profile.archive")}
          </button>

          <button
            onClick={() => onDelete(agent)}
            className="text-sm font-medium px-4 py-3 border border-[var(--border-strong)] bg-[var(--background)] text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)]"
          >
            {t("agents.profile.delete")}
          </button>
        </div>
      </div>
    </div>
  );
}
