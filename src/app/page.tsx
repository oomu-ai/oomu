"use client";
/* eslint-disable @next/next/no-img-element */
import AgentCard from "@/components/AgentCard";
import { useAppShell } from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { defaultAgentPersonalityProfile } from "@/lib/agentPersonality";
import { invoke } from "@/lib/invoke";
import { isDeveloperBuild } from "@/lib/buildFlags";
import {
  canonicalModelId,
  modelsForProvider,
  providerConfigurationId,
  providerTypeId,
} from "@/lib/modelRegistry";
import { ChangeEvent, FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import AgentProfileView from "@/components/AgentProfileView";
import { inferenceService } from "@/lib/InferenceService";
import type { ChatAgent } from "./components/ChatScreen";
import { PersistentChatSurface } from "./components/chat/PersistentChatSurface";
import { DeveloperPanel } from "./components/DeveloperPanel";
import { ModsScreen } from "./components/ModsScreen";
import { ProjectHomeSurface } from "./components/projects/ProjectHomeSurface";
import { TasksWorkspace } from "./components/tasks/TasksWorkspace";
import { ConnectionsWorkspace } from "./components/connections/ConnectionsWorkspace";
import { ArtifactStudio } from "./components/artifacts/ArtifactStudio";
import { DecisionBriefScreen } from "./components/hero/DecisionBriefScreen";
import { SetupLaunchGate } from "./components/integrations/SetupLaunchGate";
import { SovereignLedger } from "./components/SovereignLedger";
import type { ChatSession } from "@/lib/chatSessions";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import type { ChatSessionRouteBinding } from "./components/chat/sessionRouting";
import { AgentTemplateLibraryScreen } from "./components/AgentTemplateLibraryScreen";
import { ImportAgentScreen } from "./components/ImportAgentScreen";
import { LicenseAgreementGate, Panel } from "./components/HomeChrome";
import { PersistenceLoadNotice } from "./components/PersistenceLoadNotice";
import { SettingsPanel, UserConfigPanel } from "./components/settings";
import { resolveHeroDestination, useHomeWorkspaceNavigation } from "./homeNavigation";
import {
  agentToConfigRequest,
  attributeLabel,
  buildTemplatePreview,
  configToAgent,
  createAgentId,
  createAgentTemplateId,
  createAgentTimestamp,
  cropAgentImage,
  defaultLocalAgentEndpoint,
  normalizeAgentCards,
  personalityTemplateOptions,
  sortAgentCards,
  type AgentCardData,
  type AgentConfigRecord,
  type AgentInstructionTemplate,
  type AgentModBadge,
  type AgentModelProvider,
  type AgentPersonalityTemplate,
} from "./homeAgents";
import { useConfiguredProviders } from "./useConfiguredProviders";
import {
  canCreateAgentWithModel,
  resolvedAgentSessionRouteFor,
  useVerifiedStartupModel,
} from "./verifiedStartupModel";
import { useHomeStartupState } from "./useHomeStartupState";
import { useHomeProjectChatContext } from "./hooks/useHomeProjectChatContext";
import { useHomeRecoverableChatSessionDeletion } from "./hooks/useHomeRecoverableChatSessionDeletion";
import { ChatSessionDeleteToast } from "./components/ChatSessionDeleteToast";
import { NewAgentModelSelect } from "./components/NewAgentModelSelect";
import {
  commitImportedAgent,
  importedAgentRefreshAction,
  importedAgentRefreshTarget,
} from "./importedAgentRefresh";
import { useAgentModBadgeRefresh } from "./hooks/useAgentModBadgeRefresh";
import { useNewAgentModelOptions } from "./hooks/useNewAgentModelOptions";
import { openRoutineReview } from "./components/routines/openRoutineReview";
import { useRecommendedModelSettingsRoute } from "./components/integrations/recommendedModelSettingsRoute";

function localizedAgentTemplates(custom: AgentInstructionTemplate[], t: (key: string) => string) {
  return [...personalityTemplateOptions.map((template) => ({
    ...template,
    name: t(`agents.system_templates.${template.id}.name`),
    description: t(`agents.system_templates.${template.id}.description`),
  })), ...custom];
}

function useSettingsInitialTab() {
  const [destination, setDestination] = useState({ tab: "general" as "general" | "models", requestId: 0 });
  const openTab = (tab: "general" | "models") => setDestination((current) => ({
    tab,
    requestId: current.requestId + 1,
  }));
  useRecommendedModelSettingsRoute(() => openTab("models"));
  return [destination.tab, openTab, destination.requestId] as const;
}

export default function Home() {
  const { activeItem, agentsView, connectionsSection, globalChatRequestId, launchOptions, setActiveItem, setAgentsView, setRoutineDraft, tasksSection } = useAppShell();
  const { t } = useI18n();
  const workspaceNavigation = useHomeWorkspaceNavigation(setActiveItem);
  const [agentsTab, setAgentsTab] = useState<"active" | "archived">("active");
  const {
    degradedModeProbeFailed,
    degradedModeStatus,
    privacySettings,
    privacySettingsProbeFailed,
    setDegradedModeStatus,
    setPrivacySettings,
    setSetupState,
    setupProbeFailed,
    setupState,
  } = useHomeStartupState();
  const [isAcceptingLicense, setIsAcceptingLicense] = useState(false);
  const [licenseNoticeError, setLicenseNoticeError] = useState("");
  const [settingsInitialTab, setSettingsInitialTab, settingsTabRequestId] = useSettingsInitialTab();
  const [activeAgentCards, setActiveAgentCards] = useState<AgentCardData[]>([]);
  const [archivedAgentCards, setArchivedAgentCards] = useState<AgentCardData[]>([]);
  const [importRefreshAgent, setImportRefreshAgent] = useState<AgentCardData | null>(null);
  const [agentStateError, setAgentStateError] = useState("");
  const [agentModBadges, setAgentModBadges] = useState<Record<string, AgentModBadge[]>>({});
  const [configuredProviders, setConfiguredProviders] = useConfiguredProviders(
    privacySettings?.licenseAccepted,
  );
  const [chatSessions, setChatSessions] = useState<ChatSession[]>([]);
  const [chatSessionsLoaded, setChatSessionsLoaded] = useState(false);
  const [activeChatSessionId, setActiveChatSessionId] = useState("");
  const { activeChatProjectId, handleSelectChatSession, openProjectChat, startGlobalChat } =
    useHomeProjectChatContext(
      chatSessions,
      activeChatSessionId,
      setActiveChatSessionId,
      () => setActiveItem("chat"),
      globalChatRequestId,
    );
  const [chatSessionStateError, setChatSessionStateError] = useState("");
  const [isRetryingAgentState, setIsRetryingAgentState] = useState(false);
  const [isRetryingChatSessionState, setIsRetryingChatSessionState] = useState(false);
  const agentRetryInFlightRef = useRef(false);
  const chatSessionRetryInFlightRef = useRef(false);
  const [newAgentName, setNewAgentName] = useState("");
  const [newAgentDescription, setNewAgentDescription] = useState("");
  const [newAgentProvider, setNewAgentProvider] = useState<AgentModelProvider>(defaultLocalAgentEndpoint.provider);
  const [newAgentModelId, setNewAgentModelId] = useState("");
  const [newAgentPersonality, setNewAgentPersonality] =
    useState<AgentPersonalityTemplate>("everyday_agent");
  const [newAgentImage, setNewAgentImage] = useState<string | null>(null);
  const [newAgentPromptOverride, setNewAgentPromptOverride] = useState<string | null>(null);
  const [isNewAgentSheetOpen, setIsNewAgentSheetOpen] = useState(false);
  const verifiedStartupModelId = useVerifiedStartupModel(privacySettings?.licenseAccepted, isNewAgentSheetOpen);
  const [showNewAgentOptions, setShowNewAgentOptions] = useState(false);
  const [recentlyDeletedAgent, setRecentlyDeletedAgent] = useState<AgentCardData | null>(null);
  const undoTimerRef = useRef<number | null>(null);
  const [recentlyDeletedTemplate, setRecentlyDeletedTemplate] = useState<AgentInstructionTemplate | null>(null);
  const templateUndoTimerRef = useRef<number | null>(null);
  const newAgentFileInputRef = useRef<HTMLInputElement>(null);
  const [customAgentTemplates, setCustomAgentTemplates] = useState<AgentInstructionTemplate[]>([]);
  const [activeAgentTemplateId, setActiveAgentTemplateId] =
    useState<AgentPersonalityTemplate>("everyday_agent");
  const [isCreatingTemplate, setIsCreatingTemplate] = useState(false);
  const [showRawPrompt, setShowRawPrompt] = useState(false);
  const [customTemplateName, setCustomTemplateName] = useState("");
  const [customTemplateDescription, setCustomTemplateDescription] = useState("");
  const [customTemplateInstructions, setCustomTemplateInstructions] = useState("");
  const [customTemplateAttributes, setCustomTemplateAttributes] = useState<string[]>([
    "friendly",
    "concise",
  ]);
  const [isGeneratingAIInstructions, setIsGeneratingAIInstructions] = useState(false);
  const [aiInstructionsProgress, setAiInstructionsProgress] = useState("");
  const refreshAgentModBadges = useAgentModBadgeRefresh(
    setAgentModBadges,
    setAgentStateError,
  );

  const applyPersistedAgentConfigs = useCallback((configs: AgentConfigRecord[]) => {
    const loadedAgents = configs.map(configToAgent);
    setActiveAgentCards(
      normalizeAgentCards(
        loadedAgents.filter((agent) => agent.type !== "archived"),
        "active",
      ),
    );
    setArchivedAgentCards(
      normalizeAgentCards(
        loadedAgents.filter((agent) => agent.type === "archived"),
        "archived",
      ),
    );
    setAgentStateError("");
    void refreshAgentModBadges(loadedAgents);
  }, [refreshAgentModBadges]);

  const reloadPersistedAgents = useCallback(async () => {
    const configs = await invoke<AgentConfigRecord[]>("list_agent_configs");
    applyPersistedAgentConfigs(configs);
    return configs;
  }, [applyPersistedAgentConfigs]);

  const retryPersistedAgents = useCallback(async () => {
    if (agentRetryInFlightRef.current) return;
    agentRetryInFlightRef.current = true;
    setIsRetryingAgentState(true);
    try {
      await reloadPersistedAgents();
    } catch (error) {
      console.error("Failed to retry native agent configurations:", error);
      setAgentStateError("persistence_errors.agents_unavailable");
    } finally {
      agentRetryInFlightRef.current = false;
      setIsRetryingAgentState(false);
    }
  }, [reloadPersistedAgents]);

  async function persistAgentAndReload(agent: AgentCardData) {
    const request = agentToConfigRequest(agent);
    await invoke<AgentConfigRecord>("save_agent_config", { request });
    const configs = await invoke<AgentConfigRecord[]>("list_agent_configs");
    const persisted = configs.find((entry) => entry.id === request.id);
    if (
      !persisted ||
      persisted.name !== request.name ||
      persisted.system_prompt !== request.system_prompt ||
      persisted.model_id !== request.model_id ||
      persisted.provider_id !== request.provider_id ||
      persisted.description !== request.description ||
      persisted.status !== request.status ||
      persisted.favorited !== request.favorited
    ) {
      throw new Error(t("agents.save_unconfirmed"));
    }
    applyPersistedAgentConfigs(configs);
    return configToAgent(persisted);
  }

  async function deleteAgentAndReload(agentId: string) {
    await invoke<boolean>("delete_agent_config", {
      agent_id: agentId,
      agentId,
    });
    const configs = await invoke<AgentConfigRecord[]>("list_agent_configs");
    if (configs.some((entry) => entry.id === agentId)) {
      throw new Error(t("agents.delete_unconfirmed"));
    }
    applyPersistedAgentConfigs(configs);
  }

  function reportAgentStateError(error: unknown) {
    console.error("Native agent mutation failed:", error);
    setAgentStateError(
      error instanceof Error && error.message
        ? error.message
        : t("agents.native_state_unavailable"),
    );
  }

  function handleAgentModBindingsChange(agentId: string, badges: AgentModBadge[]) {
    setAgentModBadges((current) => ({
      ...current,
      [agentId]: badges,
    }));
  }

  const refreshChatSessions = useCallback(async () => {
    try {
      const sessions = await invoke<ChatSession[]>("list_chat_sessions");
      setChatSessions(sessions);
      setChatSessionsLoaded(true);
      setChatSessionStateError("");
      setActiveChatSessionId((current) => current || sessions[0]?.id || "");
      return sessions;
    } catch (e) {
      console.error("Failed to load chat sessions:", e);
      setChatSessionStateError("persistence_errors.chats_unavailable");
      return null;
    }
  }, []);

  const retryChatSessions = useCallback(async () => {
    if (chatSessionRetryInFlightRef.current) return;
    chatSessionRetryInFlightRef.current = true;
    setIsRetryingChatSessionState(true);
    try {
      await refreshChatSessions();
    } finally {
      chatSessionRetryInFlightRef.current = false;
      setIsRetryingChatSessionState(false);
    }
  }, [refreshChatSessions]);

  useEffect(() => {
    if (!privacySettings?.licenseAccepted) {
      return;
    }
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshChatSessions();
  }, [privacySettings?.licenseAccepted, refreshChatSessions]);

  useEffect(() => {
    if (!privacySettings?.licenseAccepted) {
      return;
    }
    let cancelled = false;

    async function loadAgentConfigs() {
      try {
        const configs = await invoke<AgentConfigRecord[]>("list_agent_configs");
        if (cancelled) {
          return;
        }

        if (configs.length > 0) {
          applyPersistedAgentConfigs(configs);
          return;
        }

        // The native startup path owns the implicit local-model decision.
        // Waiting here keeps setup usable until that exact model is verified.
        if (!verifiedStartupModelId) {
          setAgentStateError("");
          return;
        }

        applyPersistedAgentConfigs([]);
      } catch (error) {
        if (!cancelled) {
          setAgentStateError("persistence_errors.agents_unavailable");
        }
        console.error("Failed to load native agent configurations:", error);
      }
    }

    void loadAgentConfigs();

    return () => {
      cancelled = true;
    };
  }, [applyPersistedAgentConfigs, privacySettings?.licenseAccepted, verifiedStartupModelId]);

  const handleUpdateAgent = async (updatedAgent: AgentCardData) => {
    const touchedAt = createAgentTimestamp();
    const existingProfile = updatedAgent.personalityProfile ??
      defaultAgentPersonalityProfile({
        name: updatedAgent.name,
        description: updatedAgent.description,
        templateId: updatedAgent.personalityTemplate,
        templateName: updatedAgent.personalityTemplate,
        providerId: updatedAgent.endpoint?.provider,
      });
    const touchedAgent = {
      ...updatedAgent,
      systemPrompt: updatedAgent.systemPrompt || updatedAgent.description,
      createdAt: updatedAgent.createdAt ?? touchedAt,
      favorited: updatedAgent.type === "archived" ? false : updatedAgent.favorited,
      lastAccessedAt: touchedAt,
      personalityProfile: {
        ...existingProfile,
        identity: {
          ...existingProfile.identity,
          displayName: updatedAgent.name,
        },
        personality: {
          ...existingProfile.personality,
          summary: updatedAgent.description || existingProfile.personality.summary,
        },
      },
    };
    try {
      const persistedAgent = await persistAgentAndReload(touchedAgent);
      if (selectedAgent?.id === updatedAgent.id) {
        setSelectedAgent(persistedAgent);
      }
    } catch (error) {
      reportAgentStateError(error);
    }
  };

  const handleToggleAgentArchive = async (agent: AgentCardData) => {
    const touchedAt = createAgentTimestamp();

    if (agent.type === "archived") {
      const reactivatedAgent = {
        ...agent,
        type: "active" as const,
        favorited: false,
        createdAt: agent.createdAt ?? touchedAt,
        lastAccessedAt: touchedAt,
      };
      try {
        const persistedAgent = await persistAgentAndReload(reactivatedAgent);
        setAgentsTab("active");
        setSelectedAgent(persistedAgent);
      } catch (error) {
        reportAgentStateError(error);
      }
      return;
    }

    const archivedAgent = {
      ...agent,
      type: "archived" as const,
      favorited: false,
      createdAt: agent.createdAt ?? touchedAt,
      lastAccessedAt: touchedAt,
    };
    try {
      const persistedAgent = await persistAgentAndReload(archivedAgent);
      setAgentsTab("archived");
      setSelectedAgent(persistedAgent);
    } catch (error) {
      reportAgentStateError(error);
    }
  };

  const handleDeleteAgent = async (agent: AgentCardData) => {
    try {
      await deleteAgentAndReload(agent.id);
      setSelectedAgent(null);
      if (undoTimerRef.current) {
        window.clearTimeout(undoTimerRef.current);
      }
      setRecentlyDeletedAgent(agent);
      undoTimerRef.current = window.setTimeout(() => setRecentlyDeletedAgent(null), 10000);
    } catch (error) {
      reportAgentStateError(error);
    }
  };

  const handleUndoDeleteAgent = async () => {
    const agent = recentlyDeletedAgent;
    if (!agent) {
      return;
    }

    if (undoTimerRef.current) {
      window.clearTimeout(undoTimerRef.current);
      undoTimerRef.current = null;
    }
    try {
      await persistAgentAndReload(agent);
      setRecentlyDeletedAgent(null);
    } catch (error) {
      reportAgentStateError(error);
    }
  };

  const resetNewAgentForm = () => {
    setNewAgentName("");
    setNewAgentDescription("");
    setNewAgentProvider(defaultLocalAgentEndpoint.provider);
    setNewAgentModelId("");
    setNewAgentPersonality("everyday_agent");
    setNewAgentImage(null);
    setNewAgentPromptOverride(null);
    setShowNewAgentOptions(false);
    if (newAgentFileInputRef.current) {
      newAgentFileInputRef.current.value = "";
    }
  };

  const closeNewAgentSheet = () => {
    setIsNewAgentSheetOpen(false);
    resetNewAgentForm();
  };

  const agentTemplateOptions = useMemo(() => localizedAgentTemplates(customAgentTemplates, t), [customAgentTemplates, t]);
  const selectedPersonalityTemplate =
    agentTemplateOptions.find((template) => template.id === newAgentPersonality) ??
    agentTemplateOptions[0];
  const activeAgentTemplate =
    agentTemplateOptions.find((template) => template.id === activeAgentTemplateId) ??
    agentTemplateOptions[0];
  const canSaveCustomTemplate =
    customTemplateName.trim().length > 0 && customTemplateInstructions.trim().length > 0;
  const {
    model: selectedConfiguredModel,
    modelId: effectiveNewAgentModelId,
    models: selectedConfiguredModels,
    provider: effectiveNewAgentProvider,
    providerOptions: configuredProviderOptions,
  } = useNewAgentModelOptions(
    configuredProviders,
    newAgentProvider,
    newAgentModelId,
    verifiedStartupModelId,
  );
  // Keep the verified startup model implicit until the user chooses another.
  const canSaveNewAgent = canCreateAgentWithModel(
    newAgentName, newAgentModelId, effectiveNewAgentProvider, verifiedStartupModelId,
  );

  const handleNewAgentProviderChange = (providerId: AgentModelProvider) => {
    const providerModels = modelsForProvider(configuredProviders, providerId);
    setNewAgentProvider(providerId);
    setNewAgentModelId(providerModels[0]?.modelId ?? "");
  };

  const handleNewAgentImageUpload = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) {
      return;
    }

    cropAgentImage(file, setNewAgentImage);
  };

  const handleToggleCustomTemplateAttribute = (attributeId: string) => {
    setCustomTemplateAttributes((current) =>
      current.includes(attributeId)
        ? current.filter((entry) => entry !== attributeId)
        : [...current, attributeId],
    );
  };

  const resetCustomTemplateForm = () => {
    setCustomTemplateName("");
    setCustomTemplateDescription("");
    setCustomTemplateInstructions("");
    setCustomTemplateAttributes(["friendly", "concise"]);
  };

  const handleSaveCustomTemplate = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    const name = customTemplateName.trim();
    const instructions = customTemplateInstructions.trim();

    if (!name || !instructions) {
      return;
    }

    const template: AgentInstructionTemplate = {
      id: createAgentTemplateId(),
      name,
      description:
        customTemplateDescription.trim() ||
        "A custom template built from your own guidance.",
      instructions,
      attributes: customTemplateAttributes,
      origin: "custom",
    };
    const nextTemplates = [...customAgentTemplates, template];

    setCustomAgentTemplates(nextTemplates);
    setActiveAgentTemplateId(template.id);
    setNewAgentPersonality(template.id);
    resetCustomTemplateForm();
    setIsCreatingTemplate(false);
  };

  const handleGenerateInstructionsWithAI = async () => {
    if (!customTemplateName.trim()) return;
    setIsGeneratingAIInstructions(true);
    setAiInstructionsProgress("Connecting to local Gemma...");

    const attributeLabels = customTemplateAttributes
      .map((attr) => attributeLabel(attr))
      .join(", ");

    const prompt = `You are an elite UX architect. Write a concise, professional set of core instructions (under 80 words) for an assistant named "${customTemplateName.trim()}" described as "${customTemplateDescription.trim()}".
Its core traits are: ${attributeLabels}.
Draft the instructions in a highly actionable, clear format. Do not write any preambles or introductions, go straight to the instructions.`;

    try {
      const result = await invoke<{ text: string }>("infer", {
        request: { prompt },
      });
      if (!result.text.trim()) {
        throw new Error("Native inference returned no instructions.");
      }
      setCustomTemplateInstructions(result.text.trim());
      setAiInstructionsProgress("Ready");
      setIsGeneratingAIInstructions(false);
    } catch {
      try {
        setAiInstructionsProgress("Using native streaming inference...");
        let streamedText = "";
        const runResult = await inferenceService.infer(
          prompt,
          (token) => {
            streamedText += token;
            setCustomTemplateInstructions(streamedText);
          },
          (prog) => {
            setAiInstructionsProgress(`${Math.round(prog.progress)}% Loaded`);
          }
        );
        setCustomTemplateInstructions(runResult.text);
        setAiInstructionsProgress("Completed");
        setIsGeneratingAIInstructions(false);
      } catch (err) {
        setIsGeneratingAIInstructions(false);
        setAiInstructionsProgress(
          "Generation failed: " + (err instanceof Error ? err.message : String(err)),
        );
      }
    }
  };

  const handleDeleteCustomTemplate = (templateId: AgentPersonalityTemplate) => {
    const target = customAgentTemplates.find((template) => template.id === templateId);
    const nextTemplates = customAgentTemplates.filter((template) => template.id !== templateId);

    setCustomAgentTemplates(nextTemplates);

    if (activeAgentTemplateId === templateId) {
      setActiveAgentTemplateId("everyday_agent");
    }

    if (newAgentPersonality === templateId) {
      setNewAgentPersonality("everyday_agent");
    }

    if (!target) {
      return;
    }
    if (templateUndoTimerRef.current) {
      window.clearTimeout(templateUndoTimerRef.current);
    }
    setRecentlyDeletedTemplate(target);
    templateUndoTimerRef.current = window.setTimeout(() => setRecentlyDeletedTemplate(null), 10000);
  };

  const handleUndoDeleteTemplate = () => {
    const target = recentlyDeletedTemplate;
    if (!target) {
      return;
    }
    if (templateUndoTimerRef.current) {
      window.clearTimeout(templateUndoTimerRef.current);
      templateUndoTimerRef.current = null;
    }
    setRecentlyDeletedTemplate(null);
    setCustomAgentTemplates((current) =>
      current.some((template) => template.id === target.id) ? current : [...current, target],
    );
  };

  const handleUseTemplateForNewAgent = (templateId: AgentPersonalityTemplate) => {
    setNewAgentPersonality(templateId);
    setNewAgentPromptOverride(null);
    setAgentsView("my_agents");
    setIsNewAgentSheetOpen(true);
  };

  const handleSaveNewAgent = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    const name = newAgentName.trim();
    if (!name) {
      return;
    }

    const purpose = newAgentDescription.trim();
    const description = purpose;
    const baseInstructions = buildTemplatePreview(selectedPersonalityTemplate);
    const systemPrompt = newAgentPromptOverride?.trim()
      ? newAgentPromptOverride.trim()
      : purpose
        ? `${baseInstructions}\n\nTask Focus\n${purpose}`
        : baseInstructions;
    const createdAt = createAgentTimestamp();
    const personalityProfile = defaultAgentPersonalityProfile({
      name,
      description,
      templateName: selectedPersonalityTemplate.name,
      templateId: selectedPersonalityTemplate.id,
      templateOrigin: selectedPersonalityTemplate.origin,
      traits: selectedPersonalityTemplate.attributes.map((attributeId) => attributeLabel(attributeId)),
      providerId: selectedConfiguredModel
        ? effectiveNewAgentProvider
        : defaultLocalAgentEndpoint.provider,
    });
    const newAgent: AgentCardData = {
      id: createAgentId(),
      name,
      description,
      systemPrompt,
      icon: "mark",
      image: newAgentImage,
      type: "active",
      createdAt,
      favorited: false,
      lastAccessedAt: createdAt,
      endpoint: selectedConfiguredModel
        ? {
            provider: effectiveNewAgentProvider,
            modelId: newAgentModelId,
            customName: selectedConfiguredModel.providerName,
          }
        : {
            provider: defaultLocalAgentEndpoint.provider,
            modelId: "",
          },
      personalityTemplate: newAgentPersonality,
      personalityProfile,
    };
    try {
      await persistAgentAndReload(newAgent);
      closeNewAgentSheet();
      setAgentsTab("active");
      setSelectedAgent(null);
      setAgentsView("my_agents");
    } catch (error) {
      reportAgentStateError(error);
    }
  };

  const [selectedAgent, setSelectedAgent] = useState<AgentCardData | null>(null);

  function handleAgentsTabChange(tab: "active" | "archived") {
    setAgentsTab(tab);
    setSelectedAgent(null);
  }

  const allAgents = useMemo(() => {
    const currentAgents = agentsTab === "active" ? activeAgentCards : archivedAgentCards;
    return sortAgentCards(currentAgents.map((a) => ({ ...a, type: agentsTab })));
  }, [activeAgentCards, archivedAgentCards, agentsTab]);

  const chatAgents: ChatAgent[] = useMemo(
    () =>
      activeAgentCards.map((agent) => ({
        id: agent.id,
        name: agent.name,
        description: agent.description,
        systemPrompt: agent.systemPrompt,
        personalityProfile: agent.personalityProfile,
        endpoint: agent.endpoint,
      })),
    [activeAgentCards],
  );

  function routeForChatAgent(agentId: string) {
    return resolvedAgentSessionRouteFor(chatAgents, agentId, verifiedStartupModelId);
  }

  async function createChatSession(
    agentId: string,
    routeOverride?: ChatSessionRouteBinding,
    projectId?: string | null,
  ) {
    const route: ChatSessionRouteBinding = routeOverride ?? routeForChatAgent(agentId);
    const baseline = route.autoRouteBaseline;
    const baselineContextBudget = Number.parseInt(baseline?.context ?? "", 10);
    try {
      const session = await invoke<ChatSession>("create_chat_session", {
        projectId: projectId ?? null,
        autoRouteBaseline: baseline
          ? {
              providerConfigId: providerConfigurationId(baseline.providerId),
              providerType: providerTypeId(baseline.providerType), modelId: canonicalModelId(baseline.modelId),
              reasoningDepth: baseline.reasoning,
              contextBudget: Number.isFinite(baselineContextBudget) && baselineContextBudget > 0
                ? baselineContextBudget
                : 12_288,
            }
          : null,
        request: {
          agent_id: agentId,
          provider_id: route.providerId,
          model_id: route.modelId,
          dynamic_routing_override: route.dynamicRoutingOverride ?? null,
        },
      });
      const persistedSessions = await invoke<ChatSession[]>("list_chat_sessions");
      const persistedSession = persistedSessions.find((entry) => entry.id === session.id);
      if (!persistedSession) {
        throw new Error(t("chat.session_create_unconfirmed"));
      }
      setChatSessions(persistedSessions);
      setChatSessionStateError("");
      setActiveChatSessionId(persistedSession.id);
      return persistedSession;
    } catch (e) {
      console.error("Failed to create chat session:", e);
      setChatSessionStateError("persistence_errors.chat_create_failed");
      return null;
    }
  }

  const {
    excludePendingSession,
    recentlyDeletedSession,
    stageDelete: handleDeleteChatSession,
    undoDelete: handleUndoDeleteChatSession,
  } = useHomeRecoverableChatSessionDeletion({
    sessions: chatSessions,
    activeSessionId: activeChatSessionId,
    setSessions: setChatSessions,
    setActiveSessionId: setActiveChatSessionId,
    setChatSessionStateError,
    t,
  });

  function handleSessionsChange(sessions: ChatSession[]) {
    setChatSessions(excludePendingSession(sessions));
    setChatSessionStateError("");
  }

  function handleAgentClick(agent: AgentCardData) {
    const touchedAt = createAgentTimestamp();
    const touchedAgent = {
      ...agent,
      createdAt: agent.createdAt ?? touchedAt,
      favorited: agent.type === "archived" ? false : agent.favorited,
      lastAccessedAt: touchedAt,
    };

    setSelectedAgent(agent);
    const persistedCandidate = agent.type === "archived"
      ? { ...touchedAgent, type: "archived" as const, favorited: false }
      : { ...touchedAgent, type: "active" as const };
    void persistAgentAndReload(persistedCandidate)
      .then((persistedAgent) => setSelectedAgent(persistedAgent))
      .catch(reportAgentStateError);
  }

  async function acceptLicense() {
    if (isAcceptingLicense) {
      return privacySettings;
    }

    setIsAcceptingLicense(true);
    setLicenseNoticeError("");
    try {
      const response = await invoke<PrivacySettingsState>("accept_license");
      setPrivacySettings(response);
      return response;
    } catch (error) {
      const message = error instanceof Error
        ? error.message
        : "The license acceptance could not be saved.";
      setLicenseNoticeError(message);
      throw error;
    } finally {
      setIsAcceptingLicense(false);
    }
  }

  function handleAcceptLicense() {
    void acceptLicense().catch(() => undefined);
  }

  function handleDeclineLicenseNotice() {
    if (isAcceptingLicense) {
      return;
    }
    setIsAcceptingLicense(true);
    setLicenseNoticeError("");
    void invoke<void>("decline_license")
      .catch((error) => {
        setLicenseNoticeError(
          error instanceof Error ? error.message : t("license.decline_error"),
        );
      })
      .finally(() => setIsAcceptingLicense(false));
  }

  async function handleToggleFavoriteAgent(agent: AgentCardData) {
    if (agent.type === "archived") {
      return;
    }

    const touchedAt = createAgentTimestamp();
    const candidate = {
      ...agent,
      type: "active" as const,
      createdAt: agent.createdAt ?? touchedAt,
      favorited: !agent.favorited,
      lastAccessedAt: touchedAt,
    };
    try {
      const persistedAgent = await persistAgentAndReload(candidate);
      if (selectedAgent?.id === agent.id) {
        setSelectedAgent(persistedAgent);
      }
    } catch (error) {
      reportAgentStateError(error);
    }
  }

  useEffect(() => {
    setSelectedAgent(null);
  }, [activeItem, agentsView]);
  if (degradedModeProbeFailed || privacySettingsProbeFailed || setupProbeFailed) {
    return (
      <section
        aria-live="assertive"
        className="flex h-full min-h-0 w-full items-center justify-center overflow-hidden bg-[var(--background)] p-6 text-[var(--foreground)]"
        role="alert"
      >
        <div className="max-w-lg rounded-[var(--radius-md)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] p-6">
          <h1 className="text-xl font-semibold">{t("degraded.title")}</h1>
          <p className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]">
            {t("degraded.description")}
          </p>
        </div>
      </section>
    );
  }

  if (!privacySettings || !degradedModeStatus) {
    return (
      <section
        aria-busy="true"
        className="flex h-full min-h-0 w-full items-center justify-center overflow-hidden bg-[var(--background)] p-6 text-[var(--foreground)]"
      >
        <p className="text-sm font-semibold">{t("license.loading_state")}</p>
      </section>
    );
  }

  const shouldShowLicenseNotice = !privacySettings.licenseAccepted;

  if (shouldShowLicenseNotice) {
    return (
      <section className="flex h-full min-h-0 w-full overflow-hidden bg-[var(--background)] text-[var(--foreground)]">
        <LicenseAgreementGate
          error={licenseNoticeError}
          isAccepting={isAcceptingLicense}
          onAccept={handleAcceptLicense}
          onDecline={handleDeclineLicenseNotice}
          settings={privacySettings}
        />
      </section>
    );
  }

  if (!setupState) {
    return <section aria-busy="true" className="flex h-full items-center justify-center"><p className="text-sm font-semibold">{t("setup.loading")}</p></section>;
  }
  return (
    <SetupLaunchGate
      activeItem={activeItem}
      degradedModeStatus={degradedModeStatus}
      firstRunSetup={launchOptions?.firstRunSetup}
      onOpenSettings={() => setActiveItem("settings")}
      onProviderConfigured={(provider) => setConfiguredProviders((current) => [
        provider,
        ...current.filter((item) => item.id !== provider.id),
      ])}
      onSetupStateChange={setSetupState}
      onStatusChange={setDegradedModeStatus}
      setupState={setupState}
    >
    <section className="flex h-full min-h-0 w-full overflow-hidden bg-[var(--background)] text-[var(--foreground)]">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className={`min-h-0 flex-1 ${activeItem === "chat" || activeItem === "mods" || activeItem === "projects" || activeItem === "hero" || activeItem === "tasks" || activeItem === "artifacts" || activeItem === "connections" || (activeItem === "agents" && agentsView === "template") ? "overflow-hidden flex flex-col" : "overflow-y-auto"}`}>
          {activeItem === "agents" && agentsView === "my_agents" && !selectedAgent && (
            <div className="flex flex-col">
              {/* No "Agents" title — the sidebar already names the active section (matches Chat and Workflows). */}
              <div className="flex flex-col gap-3 px-8 pt-6 pb-4 sm:flex-row sm:items-center sm:justify-between">
                <div className="inline-flex rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-0.5">
                  {([
                    ["active", t("agents.active")],
                    ["archived", t("agents.archived")],
                  ] as const).map(([tabId, label]) => (
                    <button
                      aria-pressed={agentsTab === tabId}
                      className={`rounded-[var(--radius-sm)] px-4 py-1.5 text-sm font-medium transition-colors ${
                        agentsTab === tabId
                          ? "bg-[var(--background)] text-[var(--foreground)] shadow-[var(--shadow-card)]"
                          : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"
                      }`}
                      key={tabId}
                      onClick={() => handleAgentsTabChange(tabId)}
                      type="button"
                    >
                      {label}
                    </button>
                  ))}
                </div>
                <div className="flex flex-col gap-2 sm:flex-row sm:justify-end shrink-0">
                  <button
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                    onClick={() => setAgentsView("template")}
                    type="button"
                  >
                    {t("agents.templates")}
                  </button>
                  <button
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                    onClick={() => { setImportRefreshAgent(null); setAgentsView("import_agent"); }}
                    type="button"
                  >
                    {t("agents.import")}
                  </button>
                  <button
                    className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
                    onClick={() => setIsNewAgentSheetOpen(true)}
                    type="button"
                  >
                    {t("agents.new_agent")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {activeItem === "agents" && agentStateError ? (
            <PersistenceLoadNotice
              message={agentStateError.startsWith("persistence_errors.") ? t(agentStateError) : agentStateError}
              onRetry={() => void retryPersistedAgents()}
              retryLabel={t("tasks.control_retry")}
              retrying={isRetryingAgentState}
            />
          ) : null}
          {activeItem === "chat" && chatSessionStateError ? (
            <PersistenceLoadNotice
              message={t(chatSessionStateError)}
              onRetry={() => void retryChatSessions()}
              retryLabel={t("tasks.control_retry")}
              retrying={isRetryingChatSessionState}
            />
          ) : null}

          <div
            className={`mx-auto flex w-full flex-col ${
              activeItem === "agents" && agentsView === "template"
                ? "h-full min-h-0 p-6 flex-1 flex flex-col overflow-hidden"
                : activeItem === "chat"
                  ? "h-full min-h-0 p-0 flex-1 overflow-hidden"
                : activeItem === "mods"
                  ? "h-full min-h-0 p-0 flex-1 overflow-hidden"
                : activeItem === "projects" || activeItem === "hero" || activeItem === "tasks" || activeItem === "artifacts" || activeItem === "connections"
                  ? "h-full min-h-0 p-0 flex-1 overflow-hidden"
                : activeItem === "ledger"
                  ? "min-h-full p-0"
                : activeItem === "agents" && agentsView === "my_agents" && !selectedAgent
                  ? "p-4"
                  : "p-8"
            }`}
          >
            <>
              <PersistentChatSurface
                activeSessionId={activeChatSessionId} agents={chatAgents}
                configuredProviders={configuredProviders}
                decisionBriefCompletion={workspaceNavigation.decisionBriefCompletion}
                onCreateSession={createChatSession} onDeleteSession={handleDeleteChatSession}
                onManageAgents={() => setActiveItem("agents")}
                onOpenDocuments={() => setActiveItem("artifacts")}
                onOpenRoutine={(request) => openRoutineReview(request, activeChatProjectId, setRoutineDraft, setActiveItem)}
                onOpenModels={() => { setSettingsInitialTab("models"); setActiveItem("settings"); }}
                onOpenTasks={() => setActiveItem("tasks")} onSelectSession={handleSelectChatSession}
                onSessionsChange={handleSessionsChange} onStartGlobalChat={startGlobalChat}
                onStarterAction={workspaceNavigation.handleChatStarterAction}
                privacySettings={privacySettings} projectId={activeChatProjectId}
                sessions={chatSessions} sessionsLoaded={chatSessionsLoaded}
                verifiedStartupModelId={verifiedStartupModelId}
                visible={activeItem === "chat"}
              />
              {activeItem === "chat" ? null : activeItem === "projects" ? (
              <ProjectHomeSurface onOpenChat={openProjectChat} />
            ) : activeItem === "hero" ? (
              <DecisionBriefScreen onNavigate={(destination) => setActiveItem(resolveHeroDestination(destination))} />
            ) : activeItem === "tasks" ? (
              <TasksWorkspace
                activeSection={tasksSection}
                onRequestedTemplateLoaded={workspaceNavigation.handleRequestedWorkflowTemplateLoaded}
                onSectionChange={workspaceNavigation.handleTasksSectionChange}
                onStartInChat={() => setActiveItem("chat")}
                requestedTemplateId={workspaceNavigation.requestedWorkflowTemplateId}
                requestedTemplateSourceFolder={workspaceNavigation.requestedWorkflowSourceFolder}
              />
            ) : activeItem === "artifacts" ? (
              <ArtifactStudio
                onOpenSettings={() => setActiveItem("settings")}
                onStartInChat={() => setActiveItem("chat")}
              />
            ) : activeItem === "connections" ? (
              <ConnectionsWorkspace
                activeSection={connectionsSection}
                onSectionChange={workspaceNavigation.handleConnectionsSectionChange}
              />
            ) : activeItem === "agents" ? (
              <>
                {agentsView === "my_agents" && (
                  selectedAgent ? (
                    <AgentProfileView
                      agent={selectedAgent}
                      configuredProviders={configuredProviders}
                      key={selectedAgent.id}
                      onBack={() => setSelectedAgent(null)}
                      onDelete={handleDeleteAgent}
                      onModBindingsChange={handleAgentModBindingsChange}
                      onOpenMods={() => setActiveItem("mods")}
                      onRefreshImportedMemory={importedAgentRefreshAction(selectedAgent, (agent) => { setImportRefreshAgent(agent); setAgentsView("import_agent"); })}
                      onToggleArchive={handleToggleAgentArchive}
                      onUpdate={handleUpdateAgent}
                      templateOptions={agentTemplateOptions}
                    />
                  ) : (
                    <section className="flex flex-col gap-12">
                      {allAgents.length === 0 ? (
                        <div className="mx-auto flex w-full max-w-[68rem] flex-col items-center gap-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-12 text-center">
                          <span aria-hidden="true" className="flex h-12 w-12 items-center justify-center rounded-full bg-[var(--accent-background)] text-[var(--foreground-muted)]">
                            <svg className="h-6 w-6" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.6" viewBox="0 0 24 24">
                              <rect x="4" y="8" width="16" height="11" rx="2.5" />
                              <path d="M12 8V4" />
                              <circle cx="12" cy="3" r="1" />
                              <path d="M9 13h.01M15 13h.01" />
                            </svg>
                          </span>
                          <p className="text-sm text-[var(--foreground-muted)]">
                            {agentsTab === "archived"
                              ? t("agents.no_archived")
                              : t("agents.no_agents")}
                          </p>
                          {agentsTab !== "archived" && (
                            <button
                              className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
                              onClick={() => setIsNewAgentSheetOpen(true)}
                              type="button"
                            >
                              {t("agents.new_agent")}
                            </button>
                          )}
                        </div>
                      ) : (
                        <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 xl:grid-cols-4 max-w-[68rem] mx-auto">
                          {allAgents.map((agent) => (
                            <div className="w-full" key={agent.id}>
                              <AgentCard
                                canFavorite={agent.type !== "archived"}
                                description={agent.description}
                                image={agent.image}
                                isFavorite={agent.favorited}
                                modBadges={agentModBadges[agent.id] ?? []}
                                name={agent.name}
                                onOpen={() => handleAgentClick(agent)}
                                onToggleFavorite={() => handleToggleFavoriteAgent(agent)}
                              />
                            </div>
                          ))}
                        </div>
                      )}
                    </section>
                  )
                )}
                {agentsView === "template" && (
                  <AgentTemplateLibraryScreen
                    activeAgentTemplate={activeAgentTemplate}
                    agentTemplateOptions={agentTemplateOptions}
                    aiInstructionsProgress={aiInstructionsProgress}
                    canSaveCustomTemplate={canSaveCustomTemplate}
                    customTemplateAttributes={customTemplateAttributes}
                    customTemplateDescription={customTemplateDescription}
                    customTemplateInstructions={customTemplateInstructions}
                    customTemplateName={customTemplateName}
                    isCreatingTemplate={isCreatingTemplate}
                    isGeneratingAIInstructions={isGeneratingAIInstructions}
                    onActiveTemplateChange={setActiveAgentTemplateId}
                    onBackToAgents={() => setAgentsView("my_agents")}
                    onCustomTemplateAttributeToggle={handleToggleCustomTemplateAttribute}
                    onCustomTemplateDescriptionChange={setCustomTemplateDescription}
                    onCustomTemplateInstructionsChange={setCustomTemplateInstructions}
                    onCustomTemplateNameChange={setCustomTemplateName}
                    onDeleteTemplate={handleDeleteCustomTemplate}
                    onGenerateInstructions={handleGenerateInstructionsWithAI}
                    onResetCustomTemplate={resetCustomTemplateForm}
                    onSaveCustomTemplate={handleSaveCustomTemplate}
                    onSetCreatingTemplate={setIsCreatingTemplate}
                    onShowRawPromptChange={setShowRawPrompt}
                    onUseTemplate={handleUseTemplateForNewAgent}
                    showRawPrompt={showRawPrompt}
                  />
                )}
                {agentsView === "import_agent" && (
                  <ImportAgentScreen
                    configuredProviders={configuredProviders}
                    refreshTarget={importedAgentRefreshTarget(importRefreshAgent, defaultLocalAgentEndpoint.modelId)}
                    templateOptions={agentTemplateOptions}
                    onImportComplete={(newConfig: AgentConfigRecord) => { commitImportedAgent(newConfig, importRefreshAgent, setActiveAgentCards, setArchivedAgentCards, setSelectedAgent); setImportRefreshAgent(null); setAgentsView("my_agents"); }}
                    onCancel={() => { setImportRefreshAgent(null); setAgentsView("my_agents"); }}
                  />
                )}
              </>
            ) : activeItem === "mods" ? (
              <ModsScreen />
            ) : activeItem === "ledger" ? (
              <SovereignLedger />
            ) : activeItem === "developer" && isDeveloperBuild ? (
              <DeveloperPanel />
            ) : activeItem === "settings" ? (
              <SettingsPanel
                configuredProviders={configuredProviders}
                initialTab={settingsInitialTab}
                key={`${settingsInitialTab}:${settingsTabRequestId}`}
                onConfiguredProvidersChange={setConfiguredProviders}
                onPrivacySettingsChange={setPrivacySettings}
              />
            ) : activeItem === "user_config" ? (
              <UserConfigPanel />
            ) : (
              <Panel
                title={t("agents.nothing_to_show")}
              >
                <div className="border border-[var(--border-strong)] p-6">
                  <p className="text-sm leading-7 text-[var(--foreground-muted)]">
                    {t("agents.pick_section")}
                  </p>
                </div>
              </Panel>
              )}
            </>
          </div>
        </div>
      </div>

      {isNewAgentSheetOpen && (
        <div
          aria-modal="true"
          className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-[12vh] backdrop-blur-sm"
          onClick={closeNewAgentSheet}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              closeNewAgentSheet();
            }
          }}
          role="dialog"
        >
          <form
            className="flex max-h-[76vh] w-full max-w-md flex-col gap-5 overflow-y-auto rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--background)] p-6 shadow-2xl"
            onClick={(event) => event.stopPropagation()}
            onSubmit={handleSaveNewAgent}
          >
            <h2 className="text-base font-semibold text-[var(--foreground)]">{t("agents.new_agent_dialog.title")}</h2>

            <div className="flex items-center gap-4">
              <input
                accept="image/*"
                className="hidden"
                onChange={handleNewAgentImageUpload}
                ref={newAgentFileInputRef}
                type="file"
              />
              <button
                aria-label={newAgentImage ? t("agents.new_agent_dialog.change_photo") : t("agents.new_agent_dialog.add_photo")}
                className="flex h-14 w-14 shrink-0 items-center justify-center overflow-hidden rounded-full border border-[var(--border-strong)] bg-[var(--accent-background)] text-[var(--foreground-subtle)] transition-colors hover:bg-[var(--fill-hover)]"
                onClick={() => newAgentFileInputRef.current?.click()}
                type="button"
              >
                {newAgentImage ? (
                  <img alt={t("agents.new_agent_dialog.photo_alt")} className="h-full w-full object-cover" src={newAgentImage} />
                ) : (
                  <svg aria-hidden="true" className="h-6 w-6" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
                    <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
                    <circle cx="12" cy="7" r="4" />
                  </svg>
                )}
              </button>
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <input
                  autoFocus
                  className="w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]"
                  onChange={(event) => setNewAgentName(event.target.value)}
                  placeholder={t("agents.new_agent_dialog.name_placeholder")}
                  required
                  value={newAgentName}
                />
                {newAgentImage && (
                  <button
                    className="self-start text-xs font-medium text-[var(--foreground-muted)] transition-colors hover:text-[var(--foreground)]"
                    onClick={() => {
                      setNewAgentImage(null);
                      if (newAgentFileInputRef.current) {
                        newAgentFileInputRef.current.value = "";
                      }
                    }}
                    type="button"
                  >
                    {t("agents.new_agent_dialog.remove_photo")}
                  </button>
                )}
              </div>
            </div>

            <label className="flex flex-col gap-1.5">
              <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("agents.new_agent_dialog.description_label")}</span>
              <textarea
                className="h-20 resize-none rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm leading-relaxed text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]"
                onChange={(event) => setNewAgentDescription(event.target.value)}
                placeholder={t("agents.new_agent_dialog.description_placeholder")}
                value={newAgentDescription}
              />
            </label>

            <button
              aria-expanded={showNewAgentOptions}
              className="flex items-center gap-1.5 self-start text-xs font-medium text-[var(--foreground-muted)] transition-colors hover:text-[var(--foreground)]"
              onClick={() => setShowNewAgentOptions((value) => !value)}
              type="button"
            >
              <svg aria-hidden="true" className={`h-3 w-3 transition-transform ${showNewAgentOptions ? "rotate-90" : ""}`} fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
                <path d="M9 5l7 7-7 7" />
              </svg>
              {t("agents.new_agent_dialog.options")}
            </button>

            {showNewAgentOptions && (
              <div className="flex flex-col gap-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
                <label className="flex flex-col gap-1.5">
                  <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("agents.new_agent_dialog.personality")}</span>
                  <select
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]"
                    onChange={(event) => {
                      setNewAgentPersonality(event.target.value as AgentPersonalityTemplate);
                      setNewAgentPromptOverride(null);
                    }}
                    value={newAgentPersonality}
                  >
                    {agentTemplateOptions.map((template) => (
                      <option key={template.id} value={template.id}>
                        {template.origin === "custom"
                          ? `${template.name} (${t("agents.new_agent_dialog.custom_badge")})`
                          : template.name}
                      </option>
                    ))}
                  </select>
                  <span className="text-xs leading-relaxed text-[var(--foreground-subtle)]">
                    {selectedPersonalityTemplate.description}
                  </span>
                </label>

                <label className="flex flex-col gap-1.5">
                  <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("agents.new_agent_dialog.provider")}</span>
                  <select
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)] disabled:text-[var(--foreground-muted)]"
                    disabled={configuredProviderOptions.length === 0}
                    onChange={(event) => handleNewAgentProviderChange(event.target.value as AgentModelProvider)}
                    value={effectiveNewAgentProvider}
                  >
                    {configuredProviderOptions.length === 0 ? (
                      <option value={newAgentProvider}>{t("agents.new_agent_dialog.local_model_default")}</option>
                    ) : (
                      configuredProviderOptions.map((provider) => (
                        <option key={provider.id} value={provider.id}>
                          {provider.label}
                        </option>
                      ))
                    )}
                  </select>
                </label>

                <NewAgentModelSelect
                  models={selectedConfiguredModels}
                  onChange={setNewAgentModelId}
                  value={effectiveNewAgentModelId}
                  verifiedStartupModelId={verifiedStartupModelId}
                />

                <label className="flex flex-col gap-1.5">
                  <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("agents.new_agent_dialog.prompt")}</span>
                  <textarea
                    className="h-32 resize-y rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 font-mono text-xs leading-5 text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]"
                    onChange={(event) => setNewAgentPromptOverride(event.target.value)}
                    value={newAgentPromptOverride ?? buildTemplatePreview(selectedPersonalityTemplate)}
                  />
                  <span className="text-xs leading-relaxed text-[var(--foreground-subtle)]">
                    {newAgentPromptOverride === null
                      ? t("agents.new_agent_dialog.prompt_composed")
                      : t("agents.new_agent_dialog.prompt_custom")}
                  </span>
                </label>
              </div>
            )}

            <div className="flex justify-end gap-2 border-t border-[var(--border-soft)] pt-4">
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                onClick={closeNewAgentSheet}
                type="button"
              >
                {t("common.cancel")}
              </button>
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={!canSaveNewAgent}
                type="submit"
              >
                {t("agents.new_agent_dialog.create_short")}
              </button>
            </div>
          </form>
        </div>
      )}

      {recentlyDeletedAgent && (
        <div className="fixed top-16 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 rounded-full border border-[var(--border-soft)] bg-[var(--background)] py-1.5 pl-4 pr-1.5 shadow-lg">
          <span className="text-sm text-[var(--foreground)]">
            {t("agents.deleted", { name: recentlyDeletedAgent.name })}
          </span>
          <button
            className="rounded-full px-3 py-1.5 text-sm font-medium text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)]"
            onClick={handleUndoDeleteAgent}
            type="button"
          >
            {t("common.undo")}
          </button>
        </div>
      )}

      <ChatSessionDeleteToast
        session={recentlyDeletedSession}
        onUndo={() => void handleUndoDeleteChatSession()}
      />

      {recentlyDeletedTemplate && (
        <div className="fixed top-16 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 rounded-full border border-[var(--border-soft)] bg-[var(--background)] py-1.5 pl-4 pr-1.5 shadow-lg">
          <span className="text-sm text-[var(--foreground)]">
            {t("agents.deleted", { name: recentlyDeletedTemplate.name })}
          </span>
          <button
            className="rounded-full px-3 py-1.5 text-sm font-medium text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)]"
            onClick={handleUndoDeleteTemplate}
            type="button"
          >
            {t("common.undo")}
          </button>
        </div>
      )}
    </section>
    </SetupLaunchGate>
  );
}
