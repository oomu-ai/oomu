"use client";

import { invoke } from "@/lib/invoke";
import { safeErrorMessage } from "@/lib/redaction";
import { useModelRoutingPreferences, type ModelRouteSlot, type PersistedModelRoute } from "@/app/hooks/useModelRoute";
import {
  REMOTE_MODEL_CATALOG,
  configuredProviderIsRunnable,
  contextLabelForModel,
  parseModelIds,
  systemModelTemplatesForProvider,
  type AuthMethod,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import { useI18n } from "@/context/I18nContext";
import { RecommendedModelSettingsSetup } from "./integrations/RecommendedModelSettingsSetup";
import {
  useCallback,
  useMemo,
  useRef,
  useState,
  useEffect,
  useLayoutEffect,
  type RefObject,
} from "react";

type ModelEntry = {
  id: string;
  name: string;
  family: string;
  context: string;
  status: string;
  selectable?: boolean;
  detail?: string;
};

type ProviderCatalog = {
  id: string;
  name: string;
  nameKey?: string;
  type: "frontier" | "aggregator" | "custom" | "local";
  baseUrl: string;
  authMethods: AuthMethod[];
  defaultAuthMethod: AuthMethod;
  models: ModelEntry[];
};

type LocalModelOption = {
  id: string;
  name: string;
  path: string;
  weightsBytes: number;
  format: string;
  architecture: string;
  compatibility: "ready" | "unsupported" | "invalid" | "asset_missing";
  compatibilityMessage: string;
  chatCapability: "chat" | "base_or_unknown" | "unknown";
};

const LOCAL_MODEL_PROVIDER_ID = "local_model";
const AGGREGATOR_PROVIDER_IDS = new Set(["openrouter", "synthetic"]);

function modelEntriesForProvider(providerId: string): ModelEntry[] {
  return systemModelTemplatesForProvider(providerId).map((template) => ({
    id: template.modelId,
    name: template.name,
    family: "",
    context: contextLabelForModel(providerId, template.modelId),
    status: "current",
  }));
}

const providerCatalogs: ProviderCatalog[] = [
  {
    id: LOCAL_MODEL_PROVIDER_ID,
    name: "Local Model",
    nameKey: "models.provider_names.local_model",
    type: "local",
    baseUrl: "",
    authMethods: ["custom"],
    defaultAuthMethod: "custom",
    models: modelEntriesForProvider(LOCAL_MODEL_PROVIDER_ID),
  },
  ...REMOTE_MODEL_CATALOG.providers.map((provider): ProviderCatalog => ({
    id: provider.providerId,
    name: provider.providerName,
    nameKey: `models.provider_names.${provider.providerId}`,
    type: AGGREGATOR_PROVIDER_IDS.has(provider.providerId) ? "aggregator" : "frontier",
    baseUrl: provider.baseUrl,
    authMethods: ["api_key"],
    defaultAuthMethod: "api_key",
    models: modelEntriesForProvider(provider.providerId),
  })),
  {
    id: "custom",
    name: "Custom Provider",
    nameKey: "models.provider_names.custom",
    type: "custom",
    baseUrl: "",
    authMethods: ["api_key"],
    defaultAuthMethod: "api_key",
    models: [],
  },
];

const orderedProviderCatalogs = [...providerCatalogs].sort((left, right) => {
  if (left.id === LOCAL_MODEL_PROVIDER_ID) return -1;
  if (right.id === LOCAL_MODEL_PROVIDER_ID) return 1;
  return left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
});

function canonicalRemoteProvider(providerId: string) {
  return REMOTE_MODEL_CATALOG.providers.find(
    (provider) => provider.providerId === providerId,
  );
}

function savedProviderBaseUrl(providerId: string, enteredBaseUrl: string) {
  return canonicalRemoteProvider(providerId)?.baseUrl ?? enteredBaseUrl.trim();
}

const inputClass =
  "w-full border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]";
const labelClass = "block text-xs font-semibold text-[var(--foreground-muted)] mb-2";
const btnClass = "rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]";

function normalizeSelectedModelId(providerId: string, value: string) {
  const modelIds = parseModelIds(value);
  const catalog = canonicalRemoteProvider(providerId);
  if (catalog) {
    return modelIds[0] ?? catalog.models[0]?.modelId ?? "";
  }
  return value;
}

type TranslateFn = (key: string, variables?: Record<string, string | number>) => string;

function methodLabel(method: AuthMethod, t: TranslateFn) {
  return t(`models.auth_methods.${method}`);
}

function providerAuthHint(catalog: ProviderCatalog, t: TranslateFn) {
  const catalogId = providerCatalogs.some((entry) => entry.id === catalog.id)
    ? catalog.id
    : "custom";
  return t(`models.provider_auth_hints.${catalogId}`);
}

function providerCatalogName(catalog: ProviderCatalog, t: TranslateFn) {
  return catalog.nameKey ? t(catalog.nameKey) : catalog.name;
}

function ConfiguredProviderButton({
  fallbackRouteId, onSelect, primaryRouteId, provider, selected, t,
}: {
  fallbackRouteId: string;
  onSelect: (id: string) => void;
  primaryRouteId: string;
  provider: ConfiguredProvider;
  selected: boolean;
  t: TranslateFn;
}) {
  const runnable = configuredProviderIsRunnable(provider);
  const primary = runnable && primaryRouteId === provider.id;
  const fallback = runnable && fallbackRouteId === provider.id;
  const autoRoute = runnable && Boolean(provider.autoRouteTarget);
  return (
    <button
      aria-label={provider.providerName}
      onClick={() => onSelect(provider.id)}
      className={`flex flex-col items-start rounded-[var(--radius-md)] border p-4 text-left transition-colors ${selected
        ? "border-[var(--border-strong)] bg-[var(--fill-selected)] text-[var(--foreground)]"
        : "border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--accent-background)]"}`}
    >
      <span className="text-sm font-bold">{provider.providerName}</span>
      <span className="mt-1 text-xs text-[var(--foreground-muted)]">
        {methodLabel(provider.authMethod, t)}
      </span>
      {provider.credentialConfigured === false && provider.providerId !== LOCAL_MODEL_PROVIDER_ID && (
        <span className="mt-2 text-xs font-semibold text-[var(--destructive)]">
          {t("models.api_key_needed")}
        </span>
      )}
      {(primary || fallback || autoRoute) && (
        <div className="mt-3 flex gap-2">
          {primary && <span className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${selected ? "bg-[var(--background)] text-[var(--foreground)]" : "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"}`}>{t("models.primary_badge")}</span>}
          {fallback && <span className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${selected ? "border-[var(--background)]" : "border-[var(--foreground-muted)] text-[var(--foreground-muted)]"}`}>{t("models.fallback_badge")}</span>}
          {autoRoute && <span className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${selected ? "border-[var(--background)]" : "border-[var(--accent)] text-[var(--accent)]"}`}>{t("models.auto_route_badge")}</span>}
        </div>
      )}
    </button>
  );
}

function localModelToEntry(model: LocalModelOption, t: TranslateFn): ModelEntry {
  const capability =
    model.chatCapability === "chat"
      ? t("models.local_model.chat")
      : t("models.local_model.base_unknown");
  const diskStatus =
    model.weightsBytes > 0
      ? t("models.local_model.on_disk", {
          size: formatBytes(model.weightsBytes, t),
        })
      : t("models.local_model.no_asset");
  const statusLabel =
    model.compatibility === "ready"
      ? t("models.local_model.ready")
      : model.compatibility === "asset_missing"
        ? t("models.local_model.asset_missing")
        : t("models.local_model.incompatible");
  return {
    id: model.id,
    name: model.name,
    family: `${model.architecture} ${model.format}`,
    context: capability,
    status: `${statusLabel}: ${diskStatus}`,
    selectable: model.compatibility === "ready",
    detail: model.compatibilityMessage,
  };
}

function formatBytes(bytes: number, t: TranslateFn) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return t("models.local_model.ready_size");
  }
  const gib = bytes / 1024 / 1024 / 1024;
  if (gib >= 1) {
    return `${gib.toFixed(gib >= 10 ? 0 : 1)} GB`;
  }
  const mib = bytes / 1024 / 1024;
  return `${mib.toFixed(0)} MB`;
}

function persistedRouteForProvider(provider: ConfiguredProvider): PersistedModelRoute | null {
  const modelId = parseModelIds(provider.customModelIds)[0] ?? "";
  if (!provider.id || !provider.providerId || !modelId) {
    return null;
  }

  return {
    providerConfigId: provider.id,
    providerId: provider.id,
    modelId,
    label: `${provider.providerName || provider.providerId} / ${modelId}`,
    updatedAt: Date.now(),
  };
}

function mergeSavedProviderConfig(providers: ConfiguredProvider[], saved: ConfiguredProvider) {
  const normalizedSaved = {
    ...saved,
    autoRouteTarget: saved.providerId === LOCAL_MODEL_PROVIDER_ID ? false : Boolean(saved.autoRouteTarget),
  };
  const nextProviders = providers.some((provider) => provider.id === normalizedSaved.id)
    ? providers.map((provider) => provider.id === normalizedSaved.id ? normalizedSaved : provider)
    : [normalizedSaved, ...providers];

  if (!normalizedSaved.autoRouteTarget) {
    return nextProviders;
  }
  return nextProviders.map((provider) => ({
    ...provider,
    autoRouteTarget: provider.id === normalizedSaved.id,
  }));
}

function nextProviderConfigId(providers: ConfiguredProvider[]): string {
  const maxProviderId = providers.reduce((maxId, provider) => {
    const match = /^prov-(\d+)$/.exec(provider.id);
    if (!match) {
      return maxId;
    }

    const numericId = Number(match[1]);
    return Number.isFinite(numericId) ? Math.max(maxId, numericId) : maxId;
  }, 0);

  return `prov-${maxProviderId + 1}`;
}

type ProviderConnectionFieldsProps = {
  activeCatalog: ProviderCatalog;
  apiKey: string;
  apiKeyInputRef: RefObject<HTMLInputElement | null>;
  apiKeyLabel: string;
  authMethod: AuthMethod;
  baseUrl: string;
  canonicalBaseUrl: string | null;
  customModelIds: string;
  isLocalModelProvider: boolean;
  onApiKeyChange: (value: string) => void;
  onApiKeyLabelChange: (value: string) => void;
  onAuthMethodChange: (method: AuthMethod) => void;
  onBaseUrlChange: (value: string) => void;
  onCustomModelIdsChange: (value: string) => void;
  onToggleAdvanced: () => void;
  showAdvanced: boolean;
  t: TranslateFn;
};

function ProviderConnectionFields({
  activeCatalog,
  apiKey,
  apiKeyInputRef,
  apiKeyLabel,
  authMethod,
  baseUrl,
  canonicalBaseUrl,
  customModelIds,
  isLocalModelProvider,
  onApiKeyChange,
  onApiKeyLabelChange,
  onAuthMethodChange,
  onBaseUrlChange,
  onCustomModelIdsChange,
  onToggleAdvanced,
  showAdvanced,
  t,
}: ProviderConnectionFieldsProps) {
  if (isLocalModelProvider) {
    return (
      <p className="text-xs leading-5 text-[var(--foreground-muted)]">
        {providerAuthHint(activeCatalog, t)}
      </p>
    );
  }

  return (
    <>
      <div className="grid grid-cols-2 gap-4">
        <div>
          <label className={labelClass}>{t("models.auth_method")}</label>
          <select
            className={inputClass}
            value={authMethod}
            onChange={(event) => onAuthMethodChange(event.target.value as AuthMethod)}
          >
            {activeCatalog.authMethods.map((method) => (
              <option key={method} value={method}>{methodLabel(method, t)}</option>
            ))}
          </select>
        </div>
        <div>
          <label className={labelClass}>
            {authMethod === "oauth"
              ? t("models.oauth_client")
              : authMethod === "service_account"
                ? t("models.credential_label")
                : t("models.secret_label")}
          </label>
          <input
            className={inputClass}
            value={apiKeyLabel}
            onChange={(event) => onApiKeyLabelChange(event.target.value)}
            placeholder={t("models.key_label_placeholder")}
          />
        </div>
      </div>

      <p className="text-xs leading-5 text-[var(--foreground-muted)]">
        {providerAuthHint(activeCatalog, t)}
      </p>

      {authMethod === "api_key" && (
        <div>
          <label className={labelClass}>{t("models.api_key")}</label>
          <input
            className={inputClass}
            value={apiKey}
            onChange={(event) => onApiKeyChange(event.target.value)}
            ref={apiKeyInputRef}
            placeholder={t("models.api_key_placeholder")}
            type="password"
          />
          <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
            {t("models.saved_keys_note")}
          </p>
        </div>
      )}

      <div className="pt-4">
        <button
          className="text-xs font-semibold text-[var(--foreground)] underline underline-offset-4"
          onClick={onToggleAdvanced}
        >
          {showAdvanced
            ? t("models.hide_advanced")
            : t("models.show_advanced")}
        </button>
      </div>

      {showAdvanced && (
        <div className="grid gap-6 border-l-2 border-[var(--border-strong)] pl-4">
          <div>
            <label className={labelClass} htmlFor="provider-base-url">
              {t(canonicalBaseUrl ? "models.base_url" : "models.base_url_override")}
            </label>
            {canonicalBaseUrl ? (
              <>
                <input
                  aria-describedby="canonical-provider-url-help"
                  className={`${inputClass} cursor-default text-[var(--foreground-muted)]`}
                  id="provider-base-url"
                  readOnly
                  value={canonicalBaseUrl}
                />
                <p
                  className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]"
                  id="canonical-provider-url-help"
                >
                  {t("models.canonical_base_url_help")}
                </p>
              </>
            ) : (
              <input
                className={inputClass}
                id="provider-base-url"
                value={baseUrl}
                onChange={(event) => onBaseUrlChange(event.target.value)}
              />
            )}
          </div>
          {activeCatalog.type === "custom" && (
            <div>
              <label className={labelClass}>{t("models.manual_model_ids")}</label>
              <textarea
                className={`${inputClass} min-h-[5rem] resize-none`}
                value={customModelIds}
                onChange={(event) => onCustomModelIdsChange(event.target.value)}
                placeholder={t("models.manual_model_ids_placeholder")}
              />
            </div>
          )}
        </div>
      )}
    </>
  );
}

type ModelsScreenProps = {
  configuredProviders?: ConfiguredProvider[];
  onConfiguredProvidersChange?: (providers: ConfiguredProvider[]) => void;
};

export function ModelsScreen({
  configuredProviders: controlledConfiguredProviders,
  onConfiguredProvidersChange,
}: ModelsScreenProps = {}) {
  const { t } = useI18n();
  const [localConfiguredProviders, setLocalConfiguredProviders] = useState<ConfiguredProvider[]>([]);
  const configuredProviders = controlledConfiguredProviders ?? localConfiguredProviders;
  const setConfiguredProviders = useCallback((updater: ConfiguredProvider[] | ((current: ConfiguredProvider[]) => ConfiguredProvider[])) => {
    const nextProviders =
      typeof updater === "function" ? updater(configuredProviders) : updater;
    if (controlledConfiguredProviders === undefined) {
      setLocalConfiguredProviders(nextProviders);
    }
    onConfiguredProvidersChange?.(nextProviders);
  }, [configuredProviders, controlledConfiguredProviders, onConfiguredProvidersChange]);
  const [selectedConfiguredId, setSelectedConfiguredId] = useState<string | null>(null);
  const [isAdding, setIsAdding] = useState(false);
  const [deletingProviderId, setDeletingProviderId] = useState<string | null>(null);
  const [providerDeleteNotice, setProviderDeleteNotice] = useState<{
    message: string;
    tone: "success" | "error";
  } | null>(null);

  useEffect(() => {
    if (controlledConfiguredProviders !== undefined) {
      return;
    }

    async function loadConfigs() {
      try {
        const configs = await invoke<ConfiguredProvider[]>("list_provider_configs");
        setLocalConfiguredProviders(configs);
      } catch (e) {
        console.error(safeErrorMessage(e, "Failed to load provider configurations."));
      }
    }
    loadConfigs();
  }, [controlledConfiguredProviders]);

  const { primaryRoute, fallbackRoute, setRoutePreference } = useModelRoutingPreferences();
  const primaryRouteId = primaryRoute?.providerConfigId ?? "";
  const fallbackRouteId = fallbackRoute?.providerConfigId ?? "";

  // Form State
  const [catalogId, setCatalogId] = useState(orderedProviderCatalogs[0].id);
  const [customProviderName, setCustomProviderName] = useState("");
  const [baseUrl, setBaseUrl] = useState(orderedProviderCatalogs[0].baseUrl);
  const [authMethod, setAuthMethod] = useState<AuthMethod>(orderedProviderCatalogs[0].defaultAuthMethod);
  const [apiKeyLabel, setApiKeyLabel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const apiKeyDraftRef = useRef(apiKey);
  const apiKeyInputRef = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => {
    apiKeyDraftRef.current = apiKey;
  }, [apiKey]);
  const [customModelIds, setCustomModelIds] = useState("");
  const [autoRouteTarget, setAutoRouteTarget] = useState(false);
  const [syncedModelIds, setSyncedModelIds] = useState<string[]>([]);
  const [localModelEntries, setLocalModelEntries] = useState<ModelEntry[]>([]);
  const [syncStatus, setSyncStatus] = useState("");
  const [isSyncing, setIsSyncing] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const clearApiKeyDraft = useCallback(() => {
    apiKeyDraftRef.current = "";
    if (apiKeyInputRef.current) apiKeyInputRef.current.value = "";
    setApiKey("");
  }, []);

  useLayoutEffect(() => {
    const input = apiKeyInputRef.current;
    return () => {
      apiKeyDraftRef.current = "";
      if (input) input.value = "";
    };
  }, [authMethod, catalogId, isAdding, selectedConfiguredId]);

  function handleAddClick() {
    setIsAdding(true);
    setSelectedConfiguredId(null);
    const cat = orderedProviderCatalogs[0];
    setCatalogId(cat.id);
    setCustomProviderName("");
    setBaseUrl(cat.baseUrl);
    setAuthMethod(cat.defaultAuthMethod);
    setApiKeyLabel("");
    clearApiKeyDraft();
    setCustomModelIds("");
    setAutoRouteTarget(false);
    setSyncedModelIds([]);
    setSyncStatus("");
    setShowAdvanced(false);
    if (cat.id === LOCAL_MODEL_PROVIDER_ID) {
      void loadLocalModels(true);
    }
  }

  async function loadLocalModels(selectFirst: boolean) {
    setIsSyncing(true);
    setSyncStatus("");

    try {
      const localModels = await invoke<LocalModelOption[]>("list_local_models");
      const entries = localModels.map((model) => localModelToEntry(model, t));
      const runnableEntries = localModels
        .filter((model) => model.compatibility === "ready")
        .map((model) => localModelToEntry(model, t));
      const runnableIds = new Set(runnableEntries.map((entry) => entry.id));
      setLocalModelEntries(entries);
      setSyncedModelIds(runnableEntries.map((entry) => entry.id));

      if (entries.length === 0) {
        setCustomModelIds("");
        setSyncStatus(t("models.status.no_local_models"));
        return;
      }

      setCustomModelIds((current) => {
        const retainedIds = parseModelIds(current).filter((modelId) => runnableIds.has(modelId));
        if (retainedIds.length > 0) {
          return retainedIds.join("\n");
        }
        return selectFirst || current.trim()
          ? runnableEntries[0]?.id ?? ""
          : current;
      });
      const incompatibleCount = entries.length - runnableEntries.length;
      setSyncStatus(
        incompatibleCount
          ? t("models.status.local_models_found_with_unavailable", {
              count: entries.length,
              ready: runnableEntries.length,
              unavailable: incompatibleCount,
            })
          : t("models.status.local_models_found", {
              count: entries.length,
              ready: runnableEntries.length,
            }),
      );
    } catch (error) {
      setSyncStatus(safeErrorMessage(error, t("models.status.local_inspect_failed")));
    } finally {
      setIsSyncing(false);
    }
  }

  function handleSelectProvider(id: string) {
    const prov = configuredProviders.find(p => p.id === id);
    if (!prov) return;
    setIsAdding(false);
    setSelectedConfiguredId(id);
    setCatalogId(prov.providerId);
    setCustomProviderName(prov.providerName);
    setBaseUrl(canonicalRemoteProvider(prov.providerId)?.baseUrl ?? prov.baseUrl);
    setAuthMethod(prov.authMethod);
    setApiKeyLabel(prov.apiKeyLabel);
    clearApiKeyDraft();
    setCustomModelIds(normalizeSelectedModelId(prov.providerId, prov.customModelIds));
    setAutoRouteTarget(Boolean(prov.autoRouteTarget) && prov.providerId !== LOCAL_MODEL_PROVIDER_ID);
    setSyncedModelIds(parseModelIds(prov.customModelIds));
    setSyncStatus("");
    setShowAdvanced(false);
    if (prov.providerId === LOCAL_MODEL_PROVIDER_ID) {
      void loadLocalModels(false);
    }
  }

  function handleCatalogChange(newCatalogId: string) {
    const cat = providerCatalogs.find(c => c.id === newCatalogId) || orderedProviderCatalogs[0];
    setCatalogId(cat.id);
    setBaseUrl(cat.baseUrl);
    setAuthMethod(cat.defaultAuthMethod);
    setCustomProviderName("");
    clearApiKeyDraft();
    setApiKeyLabel("");
    setCustomModelIds(cat.models[0]?.id ?? "");
    setAutoRouteTarget(false);
    setSyncedModelIds([]);
    setSyncStatus("");
    if (cat.id === LOCAL_MODEL_PROVIDER_ID) {
      setAuthMethod("custom");
      void loadLocalModels(true);
    }
  }

  function handleRouteAssignment(slot: ModelRouteSlot, checked: boolean) {
    if (!selectedConfiguredId) return;
    const currentRouteId = slot === "primary" ? primaryRouteId : fallbackRouteId;
    if (!checked) {
      if (currentRouteId === selectedConfiguredId) {
        setRoutePreference(slot, null);
      }
      return;
    }

    const provider = configuredProviders.find((entry) => entry.id === selectedConfiguredId);
    const route = provider ? persistedRouteForProvider(provider) : null;
    if (route) {
      setRoutePreference(slot, route);
    }
  }

  async function handleSave() {
    const cat = providerCatalogs.find(c => c.id === catalogId) ?? {
      id: catalogId,
      name: customProviderName.trim() || catalogId,
      type: "custom" as const,
      baseUrl,
      authMethods: [authMethod],
      defaultAuthMethod: authMethod,
      models: [],
    };
    const name = customProviderName.trim() || providerCatalogName(cat, t);
    const modelIds = normalizeSelectedModelId(catalogId, customModelIds);
    const savedAuthMethod = catalogId === LOCAL_MODEL_PROVIDER_ID ? "custom" : authMethod;
    const savedBaseUrl = catalogId === LOCAL_MODEL_PROVIDER_ID
      ? ""
      : savedProviderBaseUrl(catalogId, baseUrl);
    const savedApiKeyLabel = catalogId === LOCAL_MODEL_PROVIDER_ID ? "" : apiKeyLabel.trim();
    const savedAutoRouteTarget = catalogId === LOCAL_MODEL_PROVIDER_ID ? false : autoRouteTarget;
    setCustomModelIds(modelIds);

    try {
      if (isAdding) {
        const newId = nextProviderConfigId(configuredProviders);
        const newProv: ConfiguredProvider = {
          id: newId,
          providerId: catalogId,
          providerName: name,
          authMethod: savedAuthMethod,
          baseUrl: savedBaseUrl,
          apiKeyLabel: savedApiKeyLabel,
          customModelIds: modelIds,
          autoRouteTarget: savedAutoRouteTarget,
          createdAtMs: 0,
          updatedAtMs: 0,
        };
        const saved = await invoke<ConfiguredProvider>("save_provider_config", {
          request: {
            ...newProv,
            apiKey: catalogId === LOCAL_MODEL_PROVIDER_ID ? undefined : apiKey.trim() || undefined,
          },
        });
        setConfiguredProviders(curr => mergeSavedProviderConfig(curr, saved));
        clearApiKeyDraft();
        setAutoRouteTarget(Boolean(saved.autoRouteTarget));
        setIsAdding(false);
        setSelectedConfiguredId(saved.id);
        updateAssignedRoutesAfterSave(saved);
      } else if (selectedConfiguredId) {
        const currentProv = configuredProviders.find(p => p.id === selectedConfiguredId);
        if (!currentProv) return;

        const updatedProv = {
          ...currentProv,
          providerId: catalogId,
          providerName: name,
          authMethod: savedAuthMethod,
          baseUrl: savedBaseUrl,
          apiKeyLabel: savedApiKeyLabel,
          customModelIds: modelIds,
          autoRouteTarget: savedAutoRouteTarget,
        };
        const saved = await invoke<ConfiguredProvider>("save_provider_config", {
          request: {
            ...updatedProv,
            apiKey: catalogId === LOCAL_MODEL_PROVIDER_ID ? undefined : apiKey.trim() || undefined,
          },
        });
        setConfiguredProviders(curr => mergeSavedProviderConfig(curr, saved));
        clearApiKeyDraft();
        setAutoRouteTarget(Boolean(saved.autoRouteTarget));
        updateAssignedRoutesAfterSave(saved);
      }
    } catch (e) {
      console.error(safeErrorMessage(e, "Failed to save provider configuration."));
    } finally {
      clearApiKeyDraft();
    }
  }

  function updateAssignedRoutesAfterSave(provider: ConfiguredProvider) {
    const route = persistedRouteForProvider(provider);
    if (!route) return;
    if (primaryRouteId === provider.id) {
      setRoutePreference("primary", route);
    }
    if (fallbackRouteId === provider.id) {
      setRoutePreference("fallback", route);
    }
  }

  async function handleDelete(id: string) {
    const target = configuredProviders.find((provider) => provider.id === id);
    if (!target || deletingProviderId) return;

    setDeletingProviderId(id);
    setProviderDeleteNotice(null);
    try {
      await invoke("delete_provider_config", { id });
      const persistedProviders = await invoke<ConfiguredProvider[]>("list_provider_configs");
      if (persistedProviders.some((provider) => provider.id === id)) {
        throw new Error(t("models.delete_provider_unconfirmed"));
      }
      setConfiguredProviders(persistedProviders);
      if (primaryRouteId === id) setRoutePreference("primary", null);
      if (fallbackRouteId === id) setRoutePreference("fallback", null);
      if (selectedConfiguredId === id) {
        clearApiKeyDraft();
        setSelectedConfiguredId(null);
      }
      setProviderDeleteNotice({
        tone: "success",
        message: t("models.deleted_provider", { name: target.providerName }),
      });
    } catch (e) {
      console.error(safeErrorMessage(e, "Failed to delete provider configuration."));
      setProviderDeleteNotice({
        tone: "error",
        message: t("models.delete_provider_failed"),
      });
    } finally {
      setDeletingProviderId(null);
    }
  }

  useEffect(() => {
    return () => {
      apiKeyDraftRef.current = "";
    };
  }, []);

  const isFormActive = isAdding || selectedConfiguredId !== null;
  const activeCatalog = providerCatalogs.find(c => c.id === catalogId) ?? {
    id: catalogId,
    name: customProviderName.trim() || catalogId,
    type: "custom" as const,
    baseUrl,
    authMethods: [authMethod],
    defaultAuthMethod: authMethod,
    models: [],
  };
  const isLocalModelProvider = activeCatalog.id === LOCAL_MODEL_PROVIDER_ID;
  const selectedProvider = selectedConfiguredId
    ? configuredProviders.find((provider) => provider.id === selectedConfiguredId) ?? null
    : null;
  const selectedProviderNeedsCredential = Boolean(
    selectedProvider && !configuredProviderIsRunnable(selectedProvider) &&
    selectedProvider.providerId !== LOCAL_MODEL_PROVIDER_ID,
  );
  const canonicalRemoteCatalog = canonicalRemoteProvider(activeCatalog.id) ?? null;
  const canonicalBaseUrl = canonicalRemoteCatalog?.baseUrl ?? null;
  const canonicalModelIds = new Set(
    canonicalRemoteCatalog?.models.map((model) => model.modelId) ?? [],
  );
  const selectedModelValue = canonicalRemoteCatalog
    ? parseModelIds(customModelIds)[0] ?? ""
    : customModelIds;
  const legacyConfiguredModelId = canonicalRemoteCatalog
    ? parseModelIds(customModelIds).find((modelId) => !canonicalModelIds.has(modelId)) ?? null
    : null;
  const modelOptions = useMemo(() => {
    const modelIds = new Set<string>();
    const catalogModels =
      isLocalModelProvider && localModelEntries.length > 0
        ? localModelEntries
        : activeCatalog.models;
    const entries = catalogModels.map((model) => {
      modelIds.add(model.id);
      return model;
    });

    if (canonicalRemoteCatalog) {
      if (legacyConfiguredModelId && !modelIds.has(legacyConfiguredModelId)) {
        entries.push({
          id: legacyConfiguredModelId,
          name: legacyConfiguredModelId,
          family: "",
          context: "",
          status: "",
          selectable: false,
          detail: t("models.legacy_model_option"),
        });
      }
      return entries;
    }

    for (const modelId of [...syncedModelIds, ...parseModelIds(customModelIds)]) {
      if (!modelIds.has(modelId)) {
        modelIds.add(modelId);
        entries.push({
          id: modelId,
          name: modelId,
          family: t("models.local_model.synced_manual"),
          context: t("models.local_model.provider_defined"),
          status: t("models.local_model.available"),
        });
      }
    }

    return entries;
  }, [
    activeCatalog.models,
    canonicalRemoteCatalog,
    customModelIds,
    isLocalModelProvider,
    legacyConfiguredModelId,
    localModelEntries,
    syncedModelIds,
    t,
  ]);

  async function handleSyncModels() {
    if (isLocalModelProvider) {
      await loadLocalModels(!customModelIds.trim());
      return;
    }

    setIsSyncing(true);
    setSyncStatus("");

    try {
      if (isAdding || !selectedConfiguredId || apiKey.trim()) {
        setSyncStatus(t("models.save_before_routes"));
        return;
      }
      const fetchedModels = await invoke<string[]>("sync_provider_models", {
        providerConfigId: selectedConfiguredId,
      });

      if (fetchedModels.length === 0) {
        setSyncStatus(t("models.status.provider_no_models"));
        return;
      }

      const offeredModels = canonicalRemoteCatalog
        ? fetchedModels.filter((modelId) => canonicalModelIds.has(modelId))
        : fetchedModels;
      if (offeredModels.length === 0) {
        setSyncStatus(t("models.status.provider_no_models"));
        return;
      }

      setSyncedModelIds(canonicalRemoteCatalog ? [] : offeredModels);
      setSyncStatus(t("models.status.provider_synced", { count: offeredModels.length }));
    } catch (error) {
      setSyncStatus(safeErrorMessage(error, t("models.status.provider_sync_failed")));
    } finally {
      clearApiKeyDraft();
      setIsSyncing(false);
    }
  }

  return (
    <section className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 pb-6">
        <h2 className="text-base font-bold text-[var(--foreground)]">{t("models.title")}</h2>
        <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--foreground-muted)]">
          {t("models.description")}
        </p>
      </div>

      <RecommendedModelSettingsSetup configuredProviders={configuredProviders} onProvidersChange={setConfiguredProviders} />

      <div className="grid min-h-0 flex-1 gap-8 lg:grid-cols-[20rem_minmax(0,1fr)]">
        {/* Left Column: Master List */}
        <aside className="flex min-h-0 flex-col border-r border-[var(--border-soft)] pr-8 custom-scrollbar">
          <div className="flex-1 overflow-y-auto">
            <h3 className={labelClass}>{t("models.configured_providers")}</h3>
            <div className="mt-4 flex flex-col gap-2">
              {configuredProviders.length > 0 ? (
                configuredProviders.map((provider) => (
                  <ConfiguredProviderButton
                    fallbackRouteId={fallbackRouteId}
                    key={provider.id}
                    onSelect={handleSelectProvider}
                    primaryRouteId={primaryRouteId}
                    provider={provider}
                    selected={selectedConfiguredId === provider.id}
                    t={t}
                  />
                ))
              ) : (
                <p className="text-sm text-[var(--foreground-muted)] italic">{t("models.no_providers")}</p>
              )}
            </div>
          </div>
          <div className="mt-6 shrink-0 pt-4">
            <button className={`${btnClass} w-full`} onClick={handleAddClick}>
              {t("models.add_provider")}
            </button>
          </div>
        </aside>

        {/* Right Column: Detail Pane */}
        <main className="min-h-0 overflow-y-auto custom-scrollbar">
          {!isFormActive ? (
            <div className="flex h-full flex-col items-center justify-center text-center text-[var(--foreground-muted)]">
              <p className="text-sm">{t("models.select_provider_empty")}</p>
            </div>
          ) : (
            <div className="max-w-2xl">
              <div className="mb-8 flex items-center justify-between border-b border-[var(--border-soft)] pb-4">
                <h3 className="text-sm font-semibold text-[var(--foreground)]">
                  {isAdding ? t("models.add_new_provider") : t("models.edit_provider")}
                </h3>
                {!isAdding && selectedConfiguredId && (
                  <button
                    className="text-xs font-semibold text-[var(--destructive)] hover:underline disabled:cursor-wait disabled:opacity-60"
                    disabled={deletingProviderId === selectedConfiguredId}
                    onClick={() => void handleDelete(selectedConfiguredId)}
                  >
                    {t("models.delete_provider")}
                  </button>
                )}
              </div>

              <div className="grid gap-6">
                {isAdding && (
                  <div>
                    <label className={labelClass}>{t("models.provider_catalog")}</label>
                    <select
                      className={inputClass}
                      value={catalogId}
                      onChange={(e) => handleCatalogChange(e.target.value)}
                    >
                      {orderedProviderCatalogs.map(cat => (
                        <option key={cat.id} value={cat.id}>{providerCatalogName(cat, t)}</option>
                      ))}
                    </select>
                  </div>
                )}

                <div>
                  <label className={labelClass}>{t("models.provider_name")}</label>
                  <input
                    className={inputClass}
                    value={customProviderName}
                    onChange={(e) => setCustomProviderName(e.target.value)}
                    placeholder={providerCatalogName(activeCatalog, t)}
                  />
                </div>

                {(modelOptions.length > 0 || isLocalModelProvider) && (
                  <div>
                    <div className="flex items-center justify-between mb-2">
                      <label className="block text-xs font-semibold text-[var(--foreground-muted)]">{t("models.model")}</label>
                      <button
                        className="text-xs font-semibold text-[var(--foreground)] hover:underline"
                        disabled={isSyncing || (!isLocalModelProvider && isAdding)}
                        onClick={handleSyncModels}
                      >
                        {isSyncing
                          ? t("models.syncing")
                          : isLocalModelProvider
                            ? t("models.refresh_local_models")
                            : t("models.sync_from_provider")}
                      </button>
                    </div>
                    <select
                      className={inputClass}
                      value={selectedModelValue}
                      onChange={(e) => setCustomModelIds(e.target.value)}
                    >
                      <option value="">{t("models.select_model")}</option>
                      {modelOptions.map(m => (
                        <option key={m.id} value={m.id} disabled={m.selectable === false}>
                          {m.name}
                          {m.selectable === false
                            ? ` · ${m.detail || m.status}`
                            : m.family
                              ? ` · ${m.family}`
                              : ""}
                        </option>
                      ))}
                    </select>
                    {legacyConfiguredModelId && (
                      <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
                        {t("models.legacy_model_help")}
                      </p>
                    )}
                    {syncStatus && (
                      <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
                        {syncStatus}
                      </p>
                    )}
                  </div>
                )}

                <ProviderConnectionFields
                  activeCatalog={activeCatalog}
                  apiKey={apiKey}
                  apiKeyInputRef={apiKeyInputRef}
                  apiKeyLabel={apiKeyLabel}
                  authMethod={authMethod}
                  baseUrl={baseUrl}
                  canonicalBaseUrl={canonicalBaseUrl}
                  customModelIds={customModelIds}
                  isLocalModelProvider={isLocalModelProvider}
                  onApiKeyChange={setApiKey}
                  onApiKeyLabelChange={setApiKeyLabel}
                  onAuthMethodChange={(method) => {
                    clearApiKeyDraft();
                    setAuthMethod(method);
                  }}
                  onBaseUrlChange={setBaseUrl}
                  onCustomModelIdsChange={setCustomModelIds}
                  onToggleAdvanced={() => setShowAdvanced(!showAdvanced)}
                  showAdvanced={showAdvanced}
                  t={t}
                />
                {selectedProviderNeedsCredential && (
                  <p className="text-sm font-semibold text-[var(--destructive)]" role="status">
                    {t("models.api_key_needed_help")}
                  </p>
                )}

                {/* Routing Assignment */}
                <div className="mt-4 border-t border-[var(--border-soft)] pt-6">
                  <h4 className={labelClass}>{t("models.routing_assignment")}</h4>
                  <div className="mt-4 flex gap-4">
                    <label className="flex items-center gap-2 text-sm text-[var(--foreground)]">
                      <input
                        type="checkbox"
                        className="accent-[var(--accent)]"
                        checked={primaryRouteId === (isAdding ? "" : selectedConfiguredId)}
                        onChange={(e) => handleRouteAssignment("primary", e.target.checked)}
                        disabled={isAdding || selectedProviderNeedsCredential}
                      />
                      {t("models.set_primary_route")}
                    </label>
                    <label className="flex items-center gap-2 text-sm text-[var(--foreground)]">
                      <input
                        type="checkbox"
                        className="accent-[var(--accent)]"
                        checked={fallbackRouteId === (isAdding ? "" : selectedConfiguredId)}
                        onChange={(e) => handleRouteAssignment("fallback", e.target.checked)}
                        disabled={isAdding || selectedProviderNeedsCredential}
                      />
                      {t("models.set_fallback_route")}
                    </label>
                    <label className="flex items-center gap-2 text-sm text-[var(--foreground)]">
                      <input
                        type="checkbox"
                        className="accent-[var(--accent)] disabled:opacity-50" id="oomu-provider-auto-route-cloud"
                        checked={!isLocalModelProvider && autoRouteTarget}
                        disabled={isLocalModelProvider || selectedProviderNeedsCredential}
                        onChange={(e) => setAutoRouteTarget(e.target.checked)}
                      />
                      {t("models.set_auto_route_cloud")}
                    </label>
                  </div>
                  {isLocalModelProvider && (
                    <p className="mt-2 text-xs text-[var(--foreground-muted)]">
                      {t("models.local_auto_route_unavailable")}
                    </p>
                  )}
                  {isAdding && <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("models.save_before_routes")}</p>}
                </div>

                <div className="mt-8">
                  <button
                    className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-5 py-2.5 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]" id="oomu-provider-save"
                    onClick={handleSave}
                  >
                    {t("models.save_configuration")}
                  </button>
                </div>

              </div>
            </div>
          )}
        </main>
      </div>

      {providerDeleteNotice && (
        <div
          className={`fixed top-16 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 rounded-full border py-2 px-4 shadow-lg ${
            providerDeleteNotice.tone === "success"
              ? "border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground)]"
              : "border-[var(--destructive)]/30 bg-[var(--destructive-background)] text-[var(--destructive)]"
          }`}
          role="status"
        >
          <span className="text-sm">{providerDeleteNotice.message}</span>
        </div>
      )}
    </section>
  );
}
