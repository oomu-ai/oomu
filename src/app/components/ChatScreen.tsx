"use client";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { ACTIVE_AGENT_CHANGED_EVENT, ACTIVE_AGENT_STORAGE_KEY, PENDING_SIDEBAR_AGENT_STORAGE_KEY, SIDEBAR_AGENT_SELECT_EVENT, type AgentSelectionEventDetail } from "@/lib/agentSelection";
import type { AgentPersonalityProfile } from "@/lib/agentPersonality";
import { createChatTurnContext, createChatTurnIdentity, deriveChatTurnContext, rebindChatTurnAttachments, type ChatTurnContext } from "@/lib/chatTurnContext";
import {
  BROWSER_SPLIT_MOD_ID,
  activateAuthorizedBrowserDirective,
  authorizedBrowserResearchFallback,
  browserFeedbackIndicatesFailedNavigation,
  browserNavigationBlockedNotice,
  browserNavigationIsBlacklisted,
  browserNavigationScope,
  browserDirectiveGrantsForMessage,
  browserSearchFallbackQuery,
  browserSplitRouteFromUserPrompt,
  hasExplicitBrowserNavigationIntent,
  headlessModSearchForMessage,
  latestBrowserSplitRoute,
  localizedModText,
  mergeBrowserDirectiveGrants,
  normalizedBrowserNavigationKey,
  reportBrowserNavigationFailure,
  splitViewDirectiveMessageIds,
  useVerticalTemplateParser,
  verticalTemplateMessageIds,
  type BrowserSplitRoute,
  type InstalledModCommandSource,
  type VerticalTemplateSection,
  type VerticalTemplateRoute,
} from "./chat/browserRouting";
import { createProjectChatDocumentForTurn, ensurePendingAssistantMessage, prepareProjectChatDocumentTurn, preferProjectDocumentRoute, projectDocumentMcpCapabilities, projectDocumentNativeRequestRoute, projectDocumentPendingAssistantId, projectDocumentRequestNeedsProjectScope, projectDocumentRouteDecision, type ProjectChatDocumentRequest } from "./chat/projectChatDocument";
import { resolveTurnProjectId, shouldDelegateToTaskFlow, unlessRecovery, type ChatIntentRouteDecision, type WorkspaceDataResource } from "./chat/chatIntentRouting";
import { surfaceStoppedChatTurn, visibleCancelledTurnMessages } from "./chat/chatTurnCancellation";
import { useModelRoutingPreferences, type PersistedModelRoute } from "@/app/hooks/useModelRoute";
import { verifiedStartupRouteForAgentEndpoint } from "@/app/verifiedStartupModel";
import { fitChatPanels, ResizeHandle, useContainerWidth, useResizablePanel } from "./ChatPanelResize";
import { useOptionalApproval } from "@/context/ApprovalContext";
import { BrowserModPanel, type RecoverableBrowserFailure } from "./chat/BrowserModPanel";
import { usePersistedDismissedSplitRoutes } from "./chat/usePersistedDismissedSplitRoutes";
import { useGatewayAutoTurn } from "./chat/useGatewayAutoTurn";
import { clearStoredActiveExecution, executionIdFromStartResponse, mergeExecutionLogs, persistActiveExecution, planIdFromStartResponse, readStoredActiveExecution, sessionIdFromStartResponse, statusFromExecutionLogs, streamStartAfterLogIdFromResponse, terminalExecutionStatusFromLogs, type ActiveAgentExecution, type AgentExecutionLogBatch, type AgentPlanAuthorityResponse, type AgentExecutionStartResponse } from "./chat/agentExecutionState";
export { terminalExecutionStatusFromLogs } from "./chat/agentExecutionState";
import { ActiveExecutionProgress } from "./chat/ActiveExecutionProgress";
import { parseAgentExecutionRecoveryReceipt, RecoveryReceiptCard, type RecoveryReceiptActions } from "./chat/RecoveryReceiptCard";
import { useMacPermissionExecutionResume } from "./chat/useMacPermissionExecutionResume";
import { agentRecoveryActionKey, localizedAgentPlanSummary, recoveryPlanRouteDecision, startNewAgentRecoveryPlan, type AgentRecoveryPlanSubmissionOptions } from "./chat/agentExecutionRecovery";
import { useAgentExecutionRecoveryHandlers } from "./chat/useAgentExecutionRecoveryHandlers";
import { calendarRecoveryFollowupForTranscript, resolveCalendarRecoveryFollowup } from "./chat/calendarRecoveryFollowup";
import { actionPlanStepPresentation, type ActionPlanStep } from "./chat/actionPlanPresentation";
import { contextualArtifactTurnRouting, plannerConversationFallbackAllowed as canFallbackAfterPlannerRejection, preferContextualArtifactRoute } from "./chat/contextualArtifactContinuation";
import { type RecoveryReceiptAuthority } from "./chat/recoveryReceiptAuthority";
import { useRecoveryReceiptProjection, type RecoveryExecutionStateSnapshot } from "./chat/useRecoveryReceiptProjection";
import { isSystemDiagnosticsPrompt, sectionLineCount, systemDiagnosticsChatSummary } from "./chat/chatPresentationHelpers";
import { WozniakSearchDebug } from "./chat/WozniakSearchDebug";
import { ChatMessageContent, CompactionSummaryDisclosure } from "./chat/ChatMessageContent";
import { SessionContextPanel, useSessionContextController } from "./chat/SessionContextPanel";
export { normalizeLogicalCertificate, parseLogicalCertificate } from "./chat/ChatMessageContent";
import { AutoRouteGlyph, compactExecutionModelLabel, RoutingIndicator } from "./chat/RoutingIndicator";
import { ChatTurnRecoveryCards } from "./chat/ChatTurnRecoveryCards";
import { AutoRouteActivationRecoveryCard } from "./chat/AutoRouteActivationRecoveryCard";
import { useAutoRouteActivation } from "./chat/useAutoRouteActivation";
import { authoritativeSessionConfigRouteIdentity, buildAutoRouteBaseline, legacySessionConfigWriteAllowed, persistLegacySessionConfigIfAllowed, providerClassIdForRoute, routeUsesLocalModel, sessionConfigContextBudget, sessionConfigReasoning, sessionUsesDynamicBinding, supportedReasoningLevelsForRoute, typedProviderClassIdForRoute, type SessionConfigRecord } from "./chat/autoRouteSessionIdentity"; // Typed routing.
import { ChatConsentCards } from "./chat/ChatConsentCards";
import { useProjectName, useProjectScopedChatSessionCreator, useRemoteMcpCancellation, useVerifiedExecutionCopy } from "./chat/useChatScreenRuntimeBindings";
import { resolveChatCloudConsentBoundary } from "./chat/chatCloudConsentFlow";
import { completeOneTimeRoutineHandoff } from "./chat/chatRoutineHandoff";
import { assistantExecutionIsLocal, assistantExecutionModelLabel, isLocalModelProviderId } from "./chat/assistantExecutionMetadata";
import { useAutoRouteRuntimeState } from "./chat/useAutoRouteRuntimeState";
import { runPermissionRecoverableAppleRead } from "./chat/directApplePermissionRead";
import { useChatTurnRecovery, type PersistedTurnReplaySubmitOptions } from "./chat/useChatTurnRecovery";
import { createReplayAwareTurnContext } from "./chat/persistedTurnReplayContext";
import { chatSubmissionIsBlocked, createChatSubmissionSeed, isRecoverySubmission, recoverySessionMismatch, shouldWaitForLocalModelHydration } from "./chat/chatSubmissionGate";
import { usePendingChatSubmissions } from "./chat/usePendingChatSubmissions";
import { useActiveChatTurns } from "./chat/useActiveChatTurns";
import { executionTurnContextFromPlanReceipt } from "./chat/turnExecutionRoute";
import { finalizeTurnWithCompletionAttention, useChatCompletionAttention } from "./chat/useChatCompletionAttention";
import { useChatCloudConsent } from "./chat/useChatCloudConsent";
import { chatErrorGroup } from "./chat/chatErrorGroups";
import { chatErrorFallbackTranslate, chatFailureNotice, localizePersistedAgentExecutionReceipt, type ChatTranslate } from "./chat/chatFailureNotice";
export { chatFailureNotice, localizePersistedAgentExecutionReceipt } from "./chat/chatFailureNotice";
import { inferenceProgressStatus } from "./chat/inferenceProgressStatus";
import { chatStreamResponseMatches, createProjectedChatStreamController } from "./chat/chatStreamController";
import { isAutoRouteAttentionError, stableErrorCode } from "./chat/inferenceErrors";
import { chatSessionStateScope, NEW_CHAT_SESSION_SCOPE, upsertByNumericId, useSessionScopedState, useStableEvent } from "./chat/sessionScopedState";
import type { CompactSessionHistoryResponse, QueuedMessageExecutionRecord, QueuedMessageRecord } from "./chat/chatPersistenceTypes";
import { ChatEmptyState, type ChatStarterHandler } from "./chat/ChatEmptyState";
import { CheckIcon, CopyIcon } from "./chat/ChatScreenIcons";
import { ChatSessionsSidebar, ChatWorkspaceHeader } from "./chat/ChatWorkspaceChrome";
import { ChatThinkingIndicator } from "./chat/ChatThinkingIndicator";
import { slashCommandForMessage } from "./chat/slashCommandRouting";
import { plannerRequestRoute } from "./chat/plannerRoute";
export { planningPreferenceForProvider } from "./chat/plannerRoute";
import { dynamicRoutingDefaultForAgent, routeBindingForDynamicRouting, type ChatSessionRouteBinding, type RouteOverride } from "./chat/sessionRouting";
import type { DecisionBriefCompletionState } from "./chat/firstRunWelcomeState";
import { isInternalUiOnlyCheckpoint, localizedAssistantTerminalContent, localizedAssistantResponse, localizedUiCheckpointContent, markAcceptedTurnTerminalAfterError, permissionRestoredPresentation, normalizeChatMessageMetadata, type ChatMessageMetadata } from "./chat/messageMetadata";
import { bindNativeEffectExpectationToTool, directExecuteCommandText, hasStructuralExecutionIntent, nativeEffectExpectationForRouteDecision, outstandingNativeEffectAfterReceipt, requiresPendingNativePostcondition, shouldBlockOutstandingNativeEffectClaim, shouldBlockUnverifiedActionClaim as evaluateUnverifiedActionClaim, type NativeEffectExpectation } from "./chat/executionIntentPolicy";
import { appleAppNameForExplicitUiIntent, hasAmbiguousPrivateAppReadLanguage, evaluateLikelyLocalAppNativeTaskIntent, isFocusedLocalAppleUiShortcutRequest, isInformationalLocalSystemTopicQuestion, localProductivityAppKindForTool, readOnlyPrivateAppToolForPrompt, retainApprovedLocalAppRequest, targetsLocalContacts, targetsLocalMusic, targetsLocalNotes, targetsLocalPhotos, targetsLocalReminders } from "./chat/localAppIntent";
import { detectDirectLocalMailReadRequest, type DirectLocalMailReadRequest } from "./chat/localMailReadIntent";
import { localMailFailureKey, localMailToolResultText } from "./chat/localMailToolResult";
import { detectDirectLocalAppleAppWriteRequest, isInternalAgentMemoryRequest, nativeAppleAppApprovalPresentation } from "./chat/localAppWriteIntent";
export { detectDirectLocalAppleAppWriteRequest, isInternalAgentMemoryRequest, nativeAppleAppApprovalPresentation } from "./chat/localAppWriteIntent";
export { detectDirectLocalMailReadRequest };
export { localSearchFailureMessage } from "./chat/localSearchErrors";
import { fetchLocalSearchForTurn, incorporateSucceededLocalSearch, localSearchAttachment, localSearchOutcomeStopsInference, releaseSucceededLocalSearchOutcome, type HeadlessSearchDebug, type LocalSearchOutcome, type LocalSearchRequestOptions } from "./chat/localSearchContext";
import { runWeb } from "./chat/searchRecoveryPolicy";
import { localSearchFailureMessage, localSearchTerminalStatus } from "./chat/localSearchErrors";
import { bindInitialSearchOutcome, createSearchContinuationState, type ParsedSearchContinuationRequest, type SearchContinuationState } from "./chat/searchContinuationCoordinator";
import { handleSearchContinuationRequest as runSearchContinuationRequest, searchContinuationTurnContext, type SearchContinuationTurnContext } from "./chat/searchContinuationWorkflow";
import { assistantControlProjection, conversationalMcpCapabilitiesFromServers, conversationalMcpToolIsAvailable, firstString, isPlainRecord, isSovereignMcpSearchCall, maxConversationalMcpToolLoopDepth, mcpContinuationAttachment, mcpContinuationMessage, mcpTerminalOutcomeMessage, mcpTerminalOutcomeText, sanitizeAssistantTranscriptText, sovereignMcpSearchQuery, type ConversationalMcpToolCapability, type ParsedConversationalMcpToolRequest } from "./chat/conversationalMcpProtocol";
export type { ConversationalMcpToolCapability } from "./chat/conversationalMcpProtocol";
export { parseConversationalMcpToolRequest } from "./chat/conversationalMcpProtocol";
import { localToolFailureCode, conversationalMcpToolIsMutation, mcpToolResultText, nativeMcpExecutionReceipt, nativeMcpPermissionFailure, protectedAppleLibraryDesktopKey, protectedAppleLibraryFailureKey, verifiedSovereignMcpSearchResult, type ProtectedAppleLibraryToolName } from "./chat/mcpToolResults";
export { localToolFailureCode, mcpToolResultText } from "./chat/mcpToolResults";
import { localContextToAttachment, messageWithAttachmentReceipt, releaseAttachmentPayloads, shouldAnalyzeVisualChatAttachment, visualAnalysisRequestForAttachment, visualAnalysisTextForAttachment, type ChatAttachment, type VisualArtifactAnalysis } from "./chat/attachments";
import { attachPrivateDataProvenance } from "./chat/privateEgress/provenance";
import { approvedLocalFileAttachment, approvedLocalFilesContextReady, approvedLocalFilePrompt, nativeDirectFileAccess, verifiedDirectFileReadRouteDecision } from "./chat/directLocalFileRead";
import { detectDirectLocalCommand, isHostLocalPath, prepareDirectLocalReadTurn } from "./chat/directLocalCommand";
export { detectDirectLocalCommand } from "./chat/directLocalCommand";
import { ACCEPTED_CHAT_SUBMISSION, REJECTED_CHAT_SUBMISSION, abandonDurableChatTurn, acceptDurableChatTurn, finalizeDurableChatTurn, type ChatSubmissionOutcome } from "./chat/submissionAcceptance";
import { waitForTerminalChatTurnResult } from "./chat/turnReconciliation";
import { agentPlanTurnContextRequest, mcpTurnContextRequest, nativeProjectTurnContextRequest, nativeTurnContextRequest, systemDiagnosticsRequest } from "./chat/chatTurnRequests";
import { useOptionalMcp, type McpToolCallResult, type McpToolApprovalRequest } from "@/hooks/useMcp";
import { DEFAULT_REASONING_LEVELS, defaultReasoningLevelForProvider, modelsForProvider, providerConfigurationId, providerOptionsFromConfigured, resolveConfiguredModelRoute, resolveReasoningFallback, type ConfiguredProvider, type ReasoningLevel } from "@/lib/modelRegistry";
import type { ChatSession, StoredChatMessage } from "@/lib/chatSessions";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import { useI18n } from "@/context/I18nContext";
import { useHumanTrust } from "@/lib/utils/trustUtils";
import { chatMcpShieldApprovalRequest, DENIED_ONCE_APPROVAL } from "@/lib/publicSearchApproval";
import { processAttachmentsBounded } from "@/lib/attachmentProcessing";
import { safeErrorMessage } from "@/lib/redaction";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { attachmentFailureCopy, composerAttachmentIsSupported, type AttachmentFailure } from "./chat/attachmentFailureCopy";
import { ChatComposer, type SlashCommandOption } from "./ChatComposer";
import { contextBudgetBoundsForProvider, type ContextBudgetBounds } from "./chat/contextBudgetBounds";
import { formatContextBudgetLabel, nearestContextBudgetStep, normalizeContextBudget } from "./chat/contextBudgetPresentation";
export { contextBudgetBoundsForProvider } from "./chat/contextBudgetBounds";
import { buildChatTaskFlowDirective, createTaskFlow, executeTaskFlow, subscribeToTaskFlowEvents, taskFlowChatLine, taskFlowExecutionIsVerified, type TaskFlowExecutionResponse } from "./taskFlowClient";
export type { ChatStarterAction, ChatStarterHandler } from "./chat/ChatEmptyState";
export { BrowserModPanel } from "./chat/BrowserModPanel";
export { containsUnverifiedActionClaim, directExecuteCommandText, isRetrospectiveNativeActionQuestion } from "./chat/executionIntentPolicy";
export { isInformationalLocalSystemTopicQuestion } from "./chat/localAppIntent";
export { candidateLocalPathsFromText } from "./chat/localPathIntent";

export { browserFeedbackIndicatesFailedNavigation, browserNavigationBlockPayload, browserNavigationBlockedNotice, browserSearchFallbackQuery, latestBrowserSplitRoute, latestVerticalTemplateRoute, parseBrowserSplitViewPayload, parseVerticalTemplatePayload, useVerticalTemplateParser, type BrowserSplitRoute, type VerticalTemplateParseResult, type VerticalTemplateRoute, type VerticalTemplateSection } from "./chat/browserRouting";
export { isSupportedVisualChatAttachment, mimeTypeForChatFile, releaseAttachmentPayloads, shouldAnalyzeVisualChatAttachment, visualAnalysisRequestForAttachment } from "./chat/attachments";

type RouteProviderId = string;
export type ChatAgent = {
  id: string;
  name: string;
  description: string;
  systemPrompt?: string;
  personalityProfile?: AgentPersonalityProfile;
  endpoint?: {
    provider: RouteProviderId;
    modelId: string;
    customName?: string;
    customBaseUrl?: string;
  };
};

const OPERATION_CONTROL_SPLIT_MOD_ID = "ai.eldris.mods.operation_control";
function splitPanelRouteIdentity(sessionId: string, providerId: string, messageId: number) {
  const routeIdentity = [chatSessionStateScope(sessionId), providerId, messageId];
  return JSON.stringify(routeIdentity);
}

function browserSplitPanelRouteIdentity(route: BrowserSplitRoute, fallbackSessionId: string) {
  return splitPanelRouteIdentity(route.sessionId?.trim() || fallbackSessionId, BROWSER_SPLIT_MOD_ID, route.messageId);
}

type ContextBudgetTone = "accent" | "emerald" | "amber" | "rose";

type SystemHardwareProfile = {
  physicalMemoryGb: number;
  processorTier: string;
  cpuArch?: string;
  cpuCores?: number;
  osName?: string;
  metalSupported?: boolean;
  maxLocalContextBudget?: number;
};

type ChatTranslateFn = (key: string, variables?: Record<string, string | number>) => string;

const contextBudgetToneClass: Record<ContextBudgetTone, string> = {
  accent: "text-[var(--accent)]",
  emerald: "text-emerald-500",
  amber: "text-amber-500",
  rose: "text-rose-500",
};

const contextBudgetToneColor: Record<ContextBudgetTone, string> = {
  accent: "var(--accent)",
  emerald: "#10b981",
  amber: "#f59e0b",
  rose: "#f43f5e",
};

const fieldClass = "border border-[var(--border-strong)] bg-[var(--background)] text-sm font-semibold text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)] disabled:cursor-not-allowed disabled:opacity-50";

type SplitPanelProvider = {
  id: string;
  routeIdentity: string;
  label: string;
  resizeLabel: string;
  render: (width: number) => ReactNode;
};

type ChatTranscriptMessage = {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  providerId?: string | null;
  modelId?: string | null;
  metadata?: ChatMessageMetadata | null;
  isCompacted?: boolean;
  compactionType?: string | null;
  isPending?: boolean;
};

const ChatMessageBubble = memo(
  function ChatMessageBubble({
    assistantName,
    completedRecoveryActionKeys,
    recoveryReceiptAuthority,
    recoveryExecutionStateSnapshot,
    onRefreshRecoveryExecutionStates,
    message,
    recoveryActions,
    onStartNewRecoveryPlan,
  }: {
    assistantName: string;
    completedRecoveryActionKeys?: ReadonlySet<string>;
    recoveryReceiptAuthority?: RecoveryReceiptAuthority;
    recoveryExecutionStateSnapshot: RecoveryExecutionStateSnapshot;
    onRefreshRecoveryExecutionStates: () => void;
    message: ChatTranscriptMessage;
    recoveryActions: RecoveryReceiptActions;
    onStartNewRecoveryPlan?: (executionId: string) => Promise<void>;
  }) {
    const { t } = useI18n();
    const recoveryReceipt = message.role === "assistant" ? parseAgentExecutionRecoveryReceipt(message.content) : null;
    if (recoveryReceipt) {
      return <RecoveryReceiptCard {...recoveryActions} content={message.content} completedActionKeys={completedRecoveryActionKeys} executionState={recoveryExecutionStateSnapshot.byExecutionId.get(recoveryReceipt.executionId)} executionStateStatus={recoveryExecutionStateSnapshot.status} onRefreshExecutionState={onRefreshRecoveryExecutionStates} onStartNewPlan={onStartNewRecoveryPlan} recoveryReceiptAuthority={recoveryReceiptAuthority} />;
    }
    const isCompactionAnchor = message.compactionType === "summary_anchor";
    const permissionRestored = permissionRestoredPresentation(message.metadata);
    const isSuccessfulToolResultNotice = message.role === "system" && message.content.trim() === t("chat.status.tool_result_ready").trim();
    const bubbleClassName =
      message.role === "user"
        ? "self-end rounded-[var(--radius-lg)] bg-[var(--accent-background)] text-[var(--foreground)]"
        : isCompactionAnchor
          ? "self-start rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--accent-background)] text-[var(--foreground)]"
          : permissionRestored
            ? permissionRestored.bubbleClassName
            : message.role === "system"
              ? isSuccessfulToolResultNotice
                ? "self-start rounded-[var(--radius-lg)] bg-[var(--success-background)] text-[var(--success)]"
                : "self-start rounded-[var(--radius-lg)] bg-[var(--destructive-background)] text-[var(--destructive)]"
              : "self-start border-b border-[var(--border-soft)] bg-transparent text-[var(--foreground)]";
    const authorLabel = message.role === "user" ? t("chat.author.you") : isCompactionAnchor ? t("chat.author.memory_checkpoint") : message.role === "system" ? t("chat.author.system") : assistantName;
    const executionModelLabel = message.role === "assistant" ? assistantExecutionModelLabel(message) : null;
    const executionIsLocal = message.role === "assistant" ? assistantExecutionIsLocal(message) : false;

    return (
      <div {...(permissionRestored?.attributes ?? {})} className={`max-w-3xl px-5 py-4 ${bubbleClassName}`}>
        <p className="text-xs font-semibold text-[var(--foreground-subtle)]">{authorLabel}</p>
        {message.role === "assistant" && message.isPending && !message.content.trim() ? (
          <div aria-live="polite" className="mt-3 inline-flex items-center gap-2 text-sm font-medium text-[var(--foreground-muted)]" role="status">
            <span className="h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-[var(--accent)] border-t-transparent" />
            <span>{t("chat.thinking_named", { name: assistantName })}</span>
          </div>
        ) : isCompactionAnchor ? (
          <CompactionSummaryDisclosure content={message.content} />
        ) : (
          <ChatMessageContent accessibilityId={message.role === "assistant" ? `oomu-assistant-response-${message.id}` : undefined} content={message.content} metadata={message.metadata} role={message.role} sources={message.metadata?.publicGroundingProvenance} />
        )}
        {message.role === "user" && ((message.metadata?.turnState === "accepted" && message.isPending) || message.metadata?.turnState === "interrupted") && (
          <p className="mt-2 text-right text-[11px] font-medium text-[var(--foreground-subtle)]" role="status">
            {t(message.metadata.turnState === "accepted" ? "chat.status.thinking" : "chat.status.generation_stopped")}
          </p>
        )}
        {message.role === "assistant" && (message.metadata?.secureMemoryStatus === "unavailable" || message.metadata?.secureMemoryStatus === "claim_rejected") && (
          <div className="mt-3 flex items-start gap-2 text-xs leading-relaxed text-[var(--foreground-muted)]" role="status">
            <svg aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
              <rect height="10" rx="2" width="14" x="5" y="11" />
              <path d="M8 11V8a4 4 0 0 1 8 0v3" />
            </svg>
            <span>
              <span className="font-medium text-[var(--foreground)]">{t(message.metadata.secureMemoryStatus === "claim_rejected" ? "chat.secure_memory.claim_rejected_title" : "chat.secure_memory.unavailable_title")}</span> {t(message.metadata.secureMemoryStatus === "claim_rejected" ? "chat.secure_memory.claim_rejected_body" : "chat.secure_memory.unavailable_body")}
            </span>
          </div>
        )}
        {executionModelLabel && (
          <div className="mt-2 flex items-center justify-end">
            <span
              aria-label={t("chat.processed_via_full", {
                model: executionModelLabel,
              })}
              className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium ${executionIsLocal ? "border-[var(--route-local-border)] bg-[var(--route-local-background)] text-[var(--route-local)]" : "border-[var(--route-cloud-border)] bg-[var(--route-cloud-background)] text-[var(--route-cloud)]"}`}
              title={t("chat.processed_via_full", {
                model: executionModelLabel,
              })}
            >
              <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
              {executionModelLabel}
            </span>
          </div>
        )}
      </div>
    );
  },
  (previous, next) =>
    previous.assistantName === next.assistantName &&
    previous.completedRecoveryActionKeys === next.completedRecoveryActionKeys &&
    previous.recoveryReceiptAuthority === next.recoveryReceiptAuthority &&
    previous.recoveryExecutionStateSnapshot === next.recoveryExecutionStateSnapshot &&
    previous.onRefreshRecoveryExecutionStates === next.onRefreshRecoveryExecutionStates &&
    previous.recoveryActions === next.recoveryActions &&
    previous.onStartNewRecoveryPlan === next.onStartNewRecoveryPlan &&
    previous.message.id === next.message.id &&
    previous.message.role === next.message.role &&
    previous.message.content === next.message.content &&
    previous.message.providerId === next.message.providerId &&
    previous.message.modelId === next.message.modelId &&
    previous.message.metadata === next.message.metadata &&
    previous.message.isCompacted === next.message.isCompacted &&
    previous.message.compactionType === next.message.compactionType &&
    previous.message.isPending === next.message.isPending,
);

function OperationControlPanel({ route }: { route: VerticalTemplateRoute }) {
  const { t } = useI18n();
  const [copiedSection, setCopiedSection] = useState<string | null>(null);
  const completionPercent = Math.round(route.parsed.completionRatio * 100);

  async function copySection(section: VerticalTemplateSection) {
    try {
      await navigator.clipboard?.writeText(section.content || section.label);
      setCopiedSection(section.key);
      window.setTimeout(() => setCopiedSection(null), 1200);
    } catch {
      setCopiedSection(null);
    }
  }

  return (
    <>
      <header className="shrink-0 border-b border-[var(--border-soft)] bg-[var(--background)] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase text-[var(--foreground-subtle)]">{t("chat.operation.eyebrow")}</p>
            <h2 className="mt-1 truncate text-sm font-semibold text-[var(--foreground)]">{t("chat.operation.title")}</h2>
          </div>
          <span className="shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[11px] font-semibold text-[var(--foreground-muted)]">{completionPercent}%</span>
        </div>
        <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-[var(--border-soft)]" role="meter" aria-valuemax={100} aria-valuemin={0} aria-valuenow={completionPercent}>
          <div className="h-full bg-[var(--accent)] transition-all duration-200" style={{ width: `${completionPercent}%` }} />
        </div>
      </header>

      <div className="custom-scrollbar grid min-h-0 flex-1 gap-3 overflow-y-auto p-3">
        {route.parsed.sections.map((section) => {
          const copied = copiedSection === section.key;
          const lineCount = sectionLineCount(section.content);

          return (
            <section className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3 text-[var(--foreground)]" key={section.key}>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h3 className="text-xs font-semibold text-[var(--foreground)]">{section.label}</h3>
                  <p className="mt-1 text-[11px] font-medium text-[var(--foreground-subtle)]">{section.present ? (lineCount === 1 ? t("chat.operation.line_one") : t("chat.operation.line_many", { count: lineCount })) : t("chat.operation.pending")}</p>
                </div>
                <button
                  aria-label={t("chat.operation.copy_section", {
                    label: section.label,
                  })}
                  className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] disabled:cursor-not-allowed disabled:opacity-40"
                  disabled={!section.content}
                  onClick={() => void copySection(section)}
                  title={t("chat.operation.copy_section", {
                    label: section.label,
                  })}
                  type="button"
                >
                  {copied ? <CheckIcon /> : <CopyIcon />}
                </button>
              </div>

              {section.content ? (
                <div className="chat-message-content mt-3 text-xs leading-5 text-[var(--foreground-muted)]">
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>{section.content}</ReactMarkdown>
                </div>
              ) : (
                <p className="mt-3 text-xs leading-5 text-[var(--foreground-subtle)]">{t("chat.operation.streaming")}</p>
              )}
            </section>
          );
        })}

        {route.parsed.fallbackContent && (
          <section className="rounded-[var(--radius-sm)] border border-[var(--warning)] bg-[var(--warning-background)] p-3">
            <h3 className="text-xs font-semibold text-[var(--foreground)]">{t("trust.unstructured_segment")}</h3>
            <div className="chat-message-content mt-3 text-xs leading-5 text-[var(--foreground-muted)]">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{route.parsed.fallbackContent}</ReactMarkdown>
            </div>
          </section>
        )}
      </div>
    </>
  );
}

type LocalContextResponse = {
  name: string;
  mime_type: string;
  byte_count: number;
  text: string;
  truncated: boolean;
};

type PickerGrantResult = {
  name: string;
  ok: boolean;
  grantId: string | null;
  mimeType: string;
  decodedByteCount: number;
  encodedByteCount: number;
  expiresAtMs: number | null;
  errorCode: string | null;
};

type ChooseLocalContextResponse = {
  results: PickerGrantResult[];
  countLimit: number;
  decodedByteLimit: number;
  encodedByteLimit: number;
};

type ChatTurnResponse = {
  text: string;
  session_id: string;
  turn_id: string;
  generation_token: string;
  metadata?: unknown;
  route_escalation?: ChatIntentRouteDecision | null;
};

type AcceptedTurnCheckpointReceipt = {
  messageId: number;
  sessionId: string;
  turnId: string;
  generationToken: string;
  kind: "web_grounding_unavailable";
  localizationKey: "chat.search_errors.ambient_unavailable";
  recordedAtMs: number;
  created: boolean;
};

type BrowserResearchContinuationResponse = {
  code: "search_continuation_completed";
  sessionId: string;
  originatingTurnId: string;
  continuationTurnId: string;
  engine: string;
};

type ExecuteCommandResponse = {
  operation: string;
  status: string;
  message: string;
  verified: boolean;
  claims?: string[];
};

type DirectLocalCalendarReadRequest = {
  calendarName: string;
  startDate: string;
  endDate: string;
  label: string;
};

type DirectLocalAppleAppReadRequest = {
  toolName: string;
  argumentsValue: Record<string, unknown>;
  appLabel: string;
  source: string;
  scope: string;
  instruction: string;
  attachmentName: string;
};

type ChatSubmitOptions = Partial<AgentRecoveryPlanSubmissionOptions> & PersistedTurnReplaySubmitOptions;

type PendingSteerTurn = {
  turnContext: ChatTurnContext;
  sessionId: string;
  agentId: string;
  userMessageId: number | null;
  message: string;
  attachments: ChatAttachment[];
  providerId: string;
  modelId: string;
  reasoning?: string;
  context?: string;
  contextBudget?: number;
  primaryRouteId?: string | null;
  fallbackRouteId?: string | null;
  automatedWebGroundingEnabled: boolean;
  assistantMessageId: number | null;
  mcpToolCapabilities: ConversationalMcpToolCapability[];
  toolLoopDepth: number;
  executableActionExpected: boolean;
  outstandingNativeEffect?: NativeEffectExpectation | null;
  verifiedNativeExecutionReceipt: boolean;
  nativeExecutionReceiptId?: string | null;
  searchContinuationState?: SearchContinuationState;
  terminalAfterResponse?: boolean;
};

type ConversationalMcpTurnContext = {
  turnContext: ChatTurnContext;
  sessionId: string;
  agentId: string;
  providerId: string;
  modelId: string;
  reasoning?: string;
  context?: string;
  contextBudget?: number;
  primaryRouteId?: string | null;
  fallbackRouteId?: string | null;
  automatedWebGroundingEnabled: boolean;
  capabilities: ConversationalMcpToolCapability[];
  toolLoopDepth: number;
  outstandingNativeEffect: NativeEffectExpectation | null;
};

type SystemDiagnosticsReport = {
  status: string;
  summary: string;
  durationMs: number;
  markdownReportPath?: string | null;
  markdownExported: boolean;
  system?: {
    environment?: {
      performance?: {
        warnings?: unknown[];
      };
    };
  };
  databaseFragmentation: { status: string }[];
  configurationHealth: { status: string }[];
  logs: { status: string }[];
};

type OomuBypassEvent = {
  sessionId: string;
  turnId: string;
  generationToken: string;
  kind: string;
  reason: string;
  message?: string;
  estimatedTokens: number;
  localContextMaxTokens: number;
  providerId: string;
  modelId: string;
  securityDisclaimer?: string;
  occurredAtMs: number;
};

type OomuBypassNotice = {
  title: string;
  body: string;
  detail: string;
};

function formatBypassTokenCount(tokens: number) {
  if (!Number.isFinite(tokens) || tokens <= 0) {
    return "unknown";
  }
  if (tokens >= 1_000_000) {
    return `${Number((tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1))}M`;
  }
  if (tokens >= 1_000) {
    return `${Number((tokens / 1_000).toFixed(tokens % 1_000 === 0 ? 0 : 1))}K`;
  }
  return String(Math.round(tokens));
}

export function oomuBypassNotice(event: Pick<OomuBypassEvent, "kind" | "reason" | "estimatedTokens" | "localContextMaxTokens" | "providerId" | "modelId" | "securityDisclaimer" | "occurredAtMs">): OomuBypassNotice {
  const model = event.modelId || "the selected remote model";
  const tokenCount = formatBypassTokenCount(event.estimatedTokens);
  const threshold = formatBypassTokenCount(event.localContextMaxTokens);
  const isTimeout = event.kind === "timeout" || event.reason.includes("timeout");
  const body = isTimeout ? `Preflight timeout: Degraded execution after the local check exceeded its time limit. Routed directly to ${model}.` : `Bypassed local check due to payload size (${tokenCount} tokens, over the ${threshold} local threshold). Routed directly to ${model}.`;

  return {
    title: "Security preflight",
    body,
    detail: event.securityDisclaimer || "Local security preflight did not complete before remote execution continued.",
  };
}

type ActionPlan = {
  id: string;
  objective: string;
  steps: ActionPlanStep[];
  exit_condition: string;
  trusted_automatic_execution: boolean;
  model_route: {
    reason: string;
    requires_principal_authorization: boolean;
  };
};

type PendingActionPlan = {
  plan: ActionPlan;
  turnContext: ChatTurnContext;
};

export function shouldUseConversationalMcpBridge(message: string, routeDecision: ChatIntentRouteDecision, capabilities: ConversationalMcpToolCapability[]) {
  const normalized = message.trim();
  if (!normalized || routeDecision.route !== "agentic_planner" || !routeDecision.requires_local_access || capabilities.length === 0) {
    return false;
  }
  if (["native_artifact_creation_filter", "deterministic_decision_pack_filter"].includes(routeDecision.decision_source)) {
    return false;
  }

  return !hasStructuralExecutionIntent(normalized);
}

export function workspaceDataResourceForAttachment(attachment: Pick<ChatAttachment, "name" | "text">): WorkspaceDataResource | null {
  const name = attachment.name.trim().toLowerCase();
  const text = attachment.text?.trim().toLowerCase() ?? "";

  if (name === "local_mail.json" || name === "local_unread_mail.json" || name === "local_unread_or_today_mail.json" || text.includes("source: macos_applescript/read_system_emails") || text.includes("local mail")) {
    return "mail";
  }
  if (name === "local_calendar.json" || text.includes("source: macos_applescript/read_system_calendar") || text.includes("local calendar context")) {
    return "calendar";
  }
  if (name === "local_reminders.json" || text.includes("source: macos_applescript/read_system_reminders") || text.includes("local reminders")) {
    return "reminders";
  }
  if (name === "local_notes.json" || text.includes("source: macos_applescript/read_system_notes") || text.includes("local notes")) {
    return "notes";
  }
  if (name === "local_contacts.json" || text.includes("source: macos_applescript/read_system_contacts") || text.includes("source: native_contacts/read_system_contacts") || text.includes("local contacts")) {
    return "contacts";
  }
  if (name === "local_photos.json" || text.includes("source: native_photos/read_system_photos") || text.includes("local photos context")) {
    return "photos";
  }
  if (name === "local_music.json" || text.includes("source: native_music/read_system_music") || text.includes("local music context")) {
    return "music";
  }
  if ((name.startsWith("local_") && name.endsWith("_ui.json")) || text.includes("source: macos_applescript/read_apple_app_ui")) {
    return "apple_app_ui";
  }

  return null;
}

function workspaceDataResourcesForAttachments(attachments: Array<Pick<ChatAttachment, "name" | "text">>) {
  return new Set(attachments.map(workspaceDataResourceForAttachment).filter((resource): resource is WorkspaceDataResource => Boolean(resource)));
}

function workspaceResourceForAppleReadTool(toolName: string): WorkspaceDataResource | null {
  switch (toolName.trim().toLowerCase()) {
    case "read_system_emails":
      return "mail";
    case "read_system_calendar":
      return "calendar";
    case "read_system_reminders":
      return "reminders";
    case "read_system_notes":
      return "notes";
    case "read_system_contacts":
      return "contacts";
    case "read_system_photos":
      return "photos";
    case "read_system_music":
      return "music";
    case "read_apple_app_ui":
      return "apple_app_ui";
    default:
      return null;
  }
}

function workspaceDataAttachmentBlocksCapability(resources: Set<WorkspaceDataResource>, capability: ConversationalMcpToolCapability) {
  if (capability.serverName.trim().toLowerCase() !== "macos_applescript") {
    return false;
  }
  const resource = workspaceResourceForAppleReadTool(capability.toolName);
  return Boolean(resource && resources.has(resource));
}

export function mcpCapabilitiesForContextualTurn(capabilities: ConversationalMcpToolCapability[], _routeDecision: ChatIntentRouteDecision, attachments: Array<Pick<ChatAttachment, "name" | "text">>) {
  const resources = workspaceDataResourcesForAttachments(attachments);
  if (resources.size === 0) {
    return capabilities;
  }
  return capabilities.filter((capability) => !workspaceDataAttachmentBlocksCapability(resources, capability));
}

export function detectDirectLocalCalendarReadRequest(message: string, now = new Date()): DirectLocalCalendarReadRequest | null {
  const normalized = message.trim();
  if (!normalized) {
    return null;
  }
  if (isInformationalLocalSystemTopicQuestion(normalized)) {
    return null;
  }

  const lower = normalized.toLowerCase();
  const mentionsCalendar = /\b(calendar|calendars|agenda|schedule|events?|meetings?|appointments?)\b/i.test(lower);
  const asksToRead = /\b(check|find|look\s+for|look\s+up|lookup|read|review|scan|search|summari[sz]e|summary|report|show|list|see|what|when)\b/i.test(lower);
  if (!mentionsCalendar || readOnlyPrivateAppToolForPrompt(lower) !== "read_system_calendar" || !asksToRead) {
    return null;
  }

  const asksForInviteDraft = /\b(?:draft|prepare|write)\b[\s\S]{0,120}\b(?:calendar\s+)?invite\b/i.test(lower) || /\b(?:calendar\s+)?invite\b[\s\S]{0,120}\b(?:draft|brief)\b/i.test(lower);
  if (/\b(add|book|cancel|create|delete|edit|move|remove|reschedule|set\s+up|update)\b/i.test(lower) || (/\binvite\b/i.test(lower) && !asksForInviteDraft) || /\bschedule\s+(?:a|an|the)?\s*(?:appointment|call|event|meeting)\b/i.test(lower)) {
    return null;
  }

  const range = calendarReadRangeFromMessage(lower, now);
  return {
    calendarName: "",
    ...range,
  };
}

function calendarReadRangeFromMessage(message: string, now: Date) {
  const explicitDateRange = calendarExplicitDateRangeFromMessage(message, now);
  if (explicitDateRange) {
    return explicitDateRange;
  }

  if (/\btomorrow\b|\bnext day\b/i.test(message)) {
    const target = addLocalDays(now, 1);
    return calendarDayRange(target, "tomorrow");
  }

  if (/\btoday\b|\bthis morning\b|\bthis afternoon\b|\btonight\b/i.test(message)) {
    return calendarDayRange(now, "today");
  }

  if (/\bthis week\b|\bnext 7 days\b|\bnext seven days\b/i.test(message)) {
    return {
      startDate: formatLocalIsoDateTime(now),
      endDate: formatLocalIsoDateTime(addLocalDays(now, 7)),
      label: "the next 7 days",
    };
  }

  return {
    startDate: formatLocalIsoDateTime(now),
    endDate: formatLocalIsoDateTime(addLocalHours(now, 24)),
    label: "the next 24 hours",
  };
}

function calendarExplicitDateRangeFromMessage(message: string, now: Date) {
  const matches = [...message.matchAll(monthDayPattern)];
  if (matches.length === 0) {
    return null;
  }

  const fallbackYear = matches.map((match) => Number.parseInt(match[3] ?? "", 10)).find((year) => Number.isInteger(year)) ?? now.getFullYear();
  const dates = matches
    .map((match) => {
      const month = monthNumberFromName(match[1]);
      const day = Number.parseInt(match[2], 10);
      const year = Number.parseInt(match[3] ?? "", 10) || fallbackYear;
      if (!month || !Number.isInteger(day) || day < 1 || day > 31) {
        return null;
      }
      return new Date(year, month - 1, day, 12, 0, 0, 0);
    })
    .filter((date): date is Date => Boolean(date));
  if (dates.length === 0) {
    return null;
  }

  dates.sort((left, right) => left.getTime() - right.getTime());
  const first = dates[0];
  const last = dates[dates.length - 1];
  const useBusinessHours = /\bbusiness hours\b/i.test(message) || /\b9\s*(?:am|a\.m\.)\b[\s\S]{0,40}\b5\s*(?:pm|p\.m\.)\b/i.test(message);

  return {
    startDate: formatLocalIsoDateTime(useBusinessHours ? atLocalTime(first, 9, 0) : startOfLocalDay(first)),
    endDate: formatLocalIsoDateTime(useBusinessHours ? atLocalTime(last, 17, 0) : endOfLocalDay(last)),
    label: calendarDateRangeLabel(first, last),
  };
}

const monthDayPattern = /\b(january|february|march|april|may|june|july|august|september|october|november|december)\s+(\d{1,2})(?:st|nd|rd|th)?(?:,\s*(\d{4}))?\b/gi;

function monthNumberFromName(name: string) {
  return ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"].indexOf(name.toLowerCase()) + 1;
}

function atLocalTime(value: Date, hours: number, minutes: number) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate(), hours, minutes, 0, 0);
}

function calendarDateRangeLabel(first: Date, last: Date) {
  const month = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][first.getMonth()];
  if (first.getFullYear() === last.getFullYear() && first.getMonth() === last.getMonth() && first.getDate() === last.getDate()) {
    return `${month} ${first.getDate()}, ${first.getFullYear()}`;
  }
  if (first.getFullYear() === last.getFullYear() && first.getMonth() === last.getMonth()) {
    return `${month} ${first.getDate()}-${last.getDate()}, ${first.getFullYear()}`;
  }

  return `${month} ${first.getDate()}, ${first.getFullYear()} through ${["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][last.getMonth()]} ${last.getDate()}, ${last.getFullYear()}`;
}

function calendarDayRange(day: Date, label: string) {
  return {
    startDate: formatLocalIsoDateTime(startOfLocalDay(day)),
    endDate: formatLocalIsoDateTime(endOfLocalDay(day)),
    label,
  };
}

function startOfLocalDay(value: Date) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate(), 0, 0, 0, 0);
}

function endOfLocalDay(value: Date) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate(), 23, 59, 59, 0);
}

function addLocalDays(value: Date, days: number) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate() + days, value.getHours(), value.getMinutes(), value.getSeconds(), 0);
}

function addLocalHours(value: Date, hours: number) {
  const next = new Date(value);
  next.setHours(next.getHours() + hours, next.getMinutes(), next.getSeconds(), 0);
  return next;
}

function formatLocalIsoDateTime(value: Date) {
  const pad = (part: number) => String(part).padStart(2, "0");
  return [value.getFullYear(), "-", pad(value.getMonth() + 1), "-", pad(value.getDate()), "T", pad(value.getHours()), ":", pad(value.getMinutes()), ":", pad(value.getSeconds())].join("");
}

function directAppleAppUiReadRequest(appName: string): DirectLocalAppleAppReadRequest {
  return {
    toolName: "read_apple_app_ui",
    argumentsValue: {
      app_name: appName,
      max_items: 80,
      activate: true,
    },
    appLabel: appName,
    source: "macos_applescript/read_apple_app_ui",
    scope: "read-only visible UI text from an allowlisted Apple system app. This may require macOS Accessibility permission.",
    instruction: "Summarize the visible app content for the user's original request. Use only this UI snapshot. If the snapshot is empty or blocked, state that macOS did not return readable UI text.",
    attachmentName: `local_${appName.toLowerCase().replace(/\s+/g, "_")}_ui.json`,
  };
}

export function detectDirectLocalAppleAppReadRequest(message: string): DirectLocalAppleAppReadRequest | null {
  const normalized = message.trim();
  if (!normalized) {
    return null;
  }
  if (isInternalAgentMemoryRequest(normalized)) {
    return null;
  }
  if (isInformationalLocalSystemTopicQuestion(normalized)) {
    return null;
  }
  const lower = normalized.toLowerCase();
  const asksToRead = /\b(check|find|look\s+at|look\s+for|look\s+up|lookup|read|review|scan|search|summari[sz]e|summary|report|show|list|see|what|which|when)\b/i.test(lower) || /\b(open|launch|activate)\b/i.test(lower) || /\b(?:do|did)\s+i\s+have\b/i.test(lower) || /\bare\s+there\b/i.test(lower) || /\b(outstanding|open|pending|remaining|due|overdue|upcoming)\b/i.test(lower);
  if (!asksToRead || hasAppleAppMutatingIntent(lower)) {
    return null;
  }

  const explicitlyTargetedAppleApp = appleAppNameFromPrompt(lower);
  if (explicitlyTargetedAppleApp === "Messages") {
    if (!isFocusedLocalAppleUiShortcutRequest(normalized)) {
      return null;
    }
    return directAppleAppUiReadRequest(explicitlyTargetedAppleApp);
  }

  if (targetsLocalReminders(lower)) {
    return {
      toolName: "read_system_reminders",
      argumentsValue: {
        completed_only: /\b(completed|done|finished)\b/i.test(lower),
      },
      appLabel: "Reminders",
      source: "macos_applescript/read_system_reminders",
      scope: "read-only local Reminders task metadata.",
      instruction: "Summarize these reminders for the user's original request. Do not invent reminders or claim a reminder was created, updated, completed, or deleted.",
      attachmentName: "local_reminders.json",
    };
  }

  if (targetsLocalNotes(lower)) {
    return {
      toolName: "read_system_notes",
      argumentsValue: {
        max_notes: 20,
        include_body: true,
      },
      appLabel: "Notes",
      source: "macos_applescript/read_system_notes",
      scope: "read-only local Notes metadata and bounded body excerpts.",
      instruction: "Summarize these notes for the user's original request. Use only this local Notes result. Do not invent notes or claim a note was created, updated, or deleted.",
      attachmentName: "local_notes.json",
    };
  }

  if (targetsLocalPhotos(lower)) {
    return {
      toolName: "read_system_photos",
      argumentsValue: {
        max_photos: 1,
      },
      appLabel: "Photos",
      source: "native_photos/read_system_photos",
      scope: "read-only metadata for the newest image in the local Photos library.",
      instruction: "Answer the user's question using only this bounded Photos result. Identify the newest image by its available filename, creation time, dimensions, and favorite status. Do not claim to recognize its visual contents unless visual analysis is explicitly present.",
      attachmentName: "local_photos.json",
    };
  }

  const asksForPersonalMusicLibrary = targetsLocalMusic(lower) && (/\b(?:check|find|list|look\s+at|look\s+for|read|review|scan|search|show|summari[sz]e|what|which|when)\b/i.test(lower) || /\b(?:newest|latest|most\s+recent|recently\s+added|last\s+added)\b/i.test(lower));
  if (asksForPersonalMusicLibrary) {
    const asksForSeveralSongs = /\b(?:list|songs|tracks|several|recently\s+added)\b/i.test(lower);
    return {
      toolName: "read_system_music",
      argumentsValue: {
        max_songs: asksForSeveralSongs ? 10 : 1,
      },
      appLabel: "Music",
      source: "native_music/read_system_music",
      scope: "read-only metadata for the newest-added songs in the local Apple Music library.",
      instruction: "Answer the user's question using only this bounded Music result. Use the available title, artist, album, and date-added metadata. Never claim playback started or that the library changed.",
      attachmentName: "local_music.json",
    };
  }

  if (targetsLocalContacts(lower)) {
    const searchText = contactSearchTextFromPrompt(normalized);
    return {
      toolName: "read_system_contacts",
      argumentsValue: {
        max_contacts: 20,
        ...(searchText ? { search_text: searchText } : {}),
      },
      appLabel: "Contacts",
      source: "native_contacts/read_system_contacts",
      scope: "read-only local Contacts names and bounded contact fields.",
      instruction: "Summarize these contacts for the user's original request. Do not invent contacts or claim a contact was created, updated, or deleted.",
      attachmentName: "local_contacts.json",
    };
  }

  const appName = explicitlyTargetedAppleApp;
  if (!appName || !isFocusedLocalAppleUiShortcutRequest(normalized)) {
    return null;
  }
  return directAppleAppUiReadRequest(appName);
}

function contactSearchTextFromPrompt(message: string) {
  const patterns = [
    /\b(?:search|scan|check|look\s+through)\s+(?:my\s+)?(?:contacts?|address book)\s+(?:and|to)\s+(?:see|check)?\s*(?:if|whether)?\s*(?:you\s+)?(?:can\s+)?(?:find|locate|look\s+for|look\s+up)\s+([\s\S]+?)(?:[?.!,]|$)/i,
    /\b(?:contacts?|address book)\s+(?:for|named|called|about)\s+([\s\S]+?)(?:[?.!,]|$)/i,
    /\b(?:for|named|called|about)\s+([\s\S]+?)\s+(?:in|from|on|within)\s+(?:my\s+)?(?:contacts?|address book)\b/i,
    /\b(?:find|look\s+for|look\s+up|lookup|search\s+for|check|show|read)\s+([\s\S]+?)\s+(?:in|from|on|within)\s+(?:my\s+)?(?:contacts?|address book)\b/i,
    /\b(?:do|did)\s+i\s+have\s+([\s\S]+?)\s+(?:in|on|within)\s+(?:my\s+)?(?:contacts?|address book)\b/i,
    /\bis\s+([\s\S]+?)\s+(?:in|on|within)\s+(?:my\s+)?(?:contacts?|address book)\b/i,
    /\b(?:what(?:'s| is)|show|get|find|check)\s+([\s\S]+?)'s\s+(?:contact|contacts?|phone|number|email|email address|address)\b/i,
  ];

  for (const pattern of patterns) {
    const match = pattern.exec(message);
    const cleaned = cleanAppleAppSearchText(match?.[1] ?? "");
    if (cleaned) {
      return cleaned;
    }
  }
  return "";
}

function cleanAppleAppSearchText(value: string) {
  const text = value
    .replace(/\b(?:please|thanks|thank you)\b/gi, " ")
    .replace(/\b(?:contact|contacts|address book|phone number|phone|number|email address|email|address)\b$/i, "")
    .replace(/^(?:my|the|a|an)\s+/i, "")
    .replace(/^["'`]+|["'`.?!,;:]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();

  if (!text || text.length > 128 || text.split(/\s+/).length > 8) {
    return "";
  }
  if (/^(?:all|any|it|me|my|them|contacts?|address book)$/i.test(text)) {
    return "";
  }
  return text;
}

function hasAppleAppMutatingIntent(message: string) {
  const prospectiveMessage = message.replace(/\b(?:did\s+i\s+add|have\s+i\s+added)\b/gi, " ");
  return /\b(add|archive|book|cancel|compose|create|delete|draft|edit|forward|invite|mark\s+(?:as\s+)?read|mark\s+(?:as\s+)?unread|move|remove|rename|reply|reschedule|save|send|set\s+up|update|write)\b/i.test(prospectiveMessage);
}

function appleAppNameFromPrompt(message: string) {
  return appleAppNameForExplicitUiIntent(message);
}

export function hasLikelyLocalNativeTaskIntent(message: string) {
  return evaluateLikelyLocalAppNativeTaskIntent(message, isInternalAgentMemoryRequest(message));
}

type BrowserResearchFallbackDependencies = {
  activeSessionId: string;
  messages: ChatTranscriptMessage[];
  seenFallbacks: Set<string>;
  registerFailedNavigation: (url: string, sessionId: string) => boolean;
  runLocalSearch: (query: string, owner: Pick<ChatTurnContext, "sessionId" | "turnId" | "generationToken">, options: LocalSearchRequestOptions) => Promise<LocalSearchOutcome>;
  addSystemMessage: (sessionId: string, content: string) => void;
  setStatus: (sessionId: string, status: string) => void;
  refreshMessages: (sessionId: string) => Promise<unknown>;
  translate: ChatTranslate;
};

async function continueBrowserResearchHeadlessly(route: BrowserSplitRoute, failureCode: RecoverableBrowserFailure, dependencies: BrowserResearchFallbackDependencies) {
  const routeSessionId = route.sessionId ?? dependencies.activeSessionId;
  if (!routeSessionId || routeSessionId !== dependencies.activeSessionId) {
    return false;
  }
  const fallback = authorizedBrowserResearchFallback(dependencies.messages, route);
  if (!fallback) {
    return false;
  }
  const origin = dependencies.messages.find((message) => message.id === fallback.originatingUserMessageId && message.role === "user");
  const originatingTurnId = origin?.metadata?.turnId;
  const originGenerationToken = origin?.metadata?.generationToken;
  if (!originatingTurnId || !originGenerationToken) {
    return false;
  }
  const fallbackIdentity = [routeSessionId, fallback.originatingUserMessageId, originatingTurnId, originGenerationToken].join(":");
  if (dependencies.seenFallbacks.has(fallbackIdentity)) {
    return true;
  }
  dependencies.seenFallbacks.add(fallbackIdentity);
  dependencies.registerFailedNavigation(route.url, routeSessionId);

  const owner = {
    sessionId: routeSessionId,
    turnId: originatingTurnId,
    generationToken: originGenerationToken,
  };
  let outcome: LocalSearchOutcome;
  try {
    outcome = await dependencies.runLocalSearch(fallback.originatingUtterance, owner, {
      activePageAvailable: false,
      searchQuery: fallback.query,
      targetSessionId: routeSessionId,
      sources: [{ kind: "user_text" }],
    });
  } catch {
    const errorCode = localSearchFailureMessage("search_unavailable", dependencies.translate);
    dependencies.addSystemMessage(routeSessionId, errorCode);
    dependencies.setStatus(routeSessionId, errorCode);
    return true;
  }

  if (outcome.kind !== "succeeded" || !outcome.verifiedContextJson) {
    const errorCode = "errorCode" in outcome ? outcome.errorCode : "search_unavailable";
    const failure = localSearchFailureMessage(errorCode, dependencies.translate);
    dependencies.addSystemMessage(routeSessionId, failure);
    dependencies.setStatus(routeSessionId, failure);
    if (outcome.kind === "succeeded") {
      releaseAttachmentPayloads(outcome.attachments);
    }
    return true;
  }

  try {
    const continuation = await invoke<BrowserResearchContinuationResponse>("continue_browser_research_headlessly", {
      request: {
        sessionId: routeSessionId,
        originatingMessageId: fallback.originatingUserMessageId,
        originatingTurnId,
        originGenerationToken,
        query: fallback.query,
        contextJson: outcome.verifiedContextJson,
        route: "interactive_browser_research",
        failureCode,
      },
    });
    if (continuation.code !== "search_continuation_completed" || continuation.sessionId !== routeSessionId || continuation.originatingTurnId !== originatingTurnId) {
      throw new Error("search_continuation_mismatch");
    }
    await dependencies.refreshMessages(routeSessionId).catch(() => false);
    return true;
  } catch {
    await dependencies.refreshMessages(routeSessionId).catch(() => false);
    dependencies.setStatus(routeSessionId, dependencies.translate("chat.search_fallback.failed"));
    return true;
  } finally {
    releaseAttachmentPayloads(outcome.attachments);
  }
}

async function resolveDirectTurnRequests({ message, recoveryTurn, attachedWorkspaceResources }: { message: string; recoveryTurn: boolean; attachedWorkspaceResources: ReturnType<typeof workspaceDataResourcesForAttachments> }) {
  let ambiguousLocalAppTriageFailure = hasAmbiguousPrivateAppReadLanguage(message);
  const markAmbiguousLocalAppTriageFailure = () => {
    ambiguousLocalAppTriageFailure = true;
  };
  const directLocalIntent = recoveryTurn ? null : detectDirectLocalCommand(message);
  const directLocalReadRequest = directLocalIntent?.kind === "read" || directLocalIntent?.kind === "read_many" ? directLocalIntent : null;
  const directLocalCommand = directLocalIntent?.kind === "read" || directLocalIntent?.kind === "read_many" ? null : directLocalIntent;
  const directMailReadCandidate = !recoveryTurn && !directLocalCommand && !directLocalReadRequest && !attachedWorkspaceResources.has("mail") ? detectDirectLocalMailReadRequest(message) : null;
  const directMailReadRequest = await retainApprovedLocalAppRequest(directMailReadCandidate, message, "mail", markAmbiguousLocalAppTriageFailure);
  const directCalendarReadCandidate = !recoveryTurn && !directLocalCommand && !directLocalReadRequest && !directMailReadRequest && !attachedWorkspaceResources.has("calendar") ? detectDirectLocalCalendarReadRequest(message) : null;
  const directCalendarReadRequest = await retainApprovedLocalAppRequest(directCalendarReadCandidate, message, "calendar", markAmbiguousLocalAppTriageFailure);
  const directAppleAppReadCandidate = recoveryTurn || directLocalCommand || directLocalReadRequest || directMailReadRequest || directCalendarReadRequest ? null : detectDirectLocalAppleAppReadRequest(message);
  const directAppleAppReadResource = directAppleAppReadCandidate ? workspaceResourceForAppleReadTool(directAppleAppReadCandidate.toolName) : null;
  const unattachedAppleAppReadRequest = directAppleAppReadResource && attachedWorkspaceResources.has(directAppleAppReadResource) ? null : directAppleAppReadCandidate;
  const directAppleAppReadRequest = await retainApprovedLocalAppRequest(unattachedAppleAppReadRequest, message, unattachedAppleAppReadRequest ? localProductivityAppKindForTool(unattachedAppleAppReadRequest.toolName) : null, markAmbiguousLocalAppTriageFailure);
  const directAppleAppWriteCandidate = recoveryTurn || directLocalCommand || directLocalReadRequest || directMailReadRequest || directCalendarReadRequest || directAppleAppReadRequest ? null : detectDirectLocalAppleAppWriteRequest(message);
  const directAppleAppWriteRequest = await retainApprovedLocalAppRequest(directAppleAppWriteCandidate, message, directAppleAppWriteCandidate ? localProductivityAppKindForTool(directAppleAppWriteCandidate.toolName) : null, markAmbiguousLocalAppTriageFailure);

  return {
    ambiguousLocalAppTriageFailure,
    directLocalCommand,
    directLocalReadRequest,
    directMailReadRequest,
    directCalendarReadRequest,
    directAppleAppReadRequest,
    directAppleAppWriteRequest,
    hasPrivateAppCandidate: Boolean(directMailReadCandidate || directCalendarReadCandidate || directAppleAppReadCandidate || directAppleAppWriteCandidate),
  };
}

export function canSubmitLocalToolWorkflowWhileHydrating(message: string) {
  return Boolean(detectDirectLocalCommand(message)) || Boolean(detectDirectLocalMailReadRequest(message)) || Boolean(detectDirectLocalCalendarReadRequest(message)) || Boolean(detectDirectLocalAppleAppReadRequest(message)) || Boolean(detectDirectLocalAppleAppWriteRequest(message)) || hasLikelyLocalNativeTaskIntent(message) || isSystemDiagnosticsPrompt(message);
}

function visualAnalysisErrorTextForAttachment(attachment: ChatAttachment, error: unknown) {
  return [`Visual analysis for ${attachment.name}`, `MIME type: ${attachment.mime_type}`, "", "Analysis blocked:", toolErrorMessage(error)].join("\n");
}

function mcpTargetPath(argumentsValue: unknown) {
  if (!isPlainRecord(argumentsValue)) {
    return null;
  }
  const target = firstString(argumentsValue.path, argumentsValue.targetPath, argumentsValue.target_path);
  return target && target.trim().length > 0 ? target : null;
}

export function calendarToolFailureMessage(error: unknown, t: ChatTranslate = chatErrorFallbackTranslate) {
  switch (localToolFailureCode(error)) {
    case "timeout":
      return t("chat.errors.calendar_timeout");
    case "permission":
      return t("chat.errors.calendar_permission");
    default:
      return t("chat.errors.calendar_unavailable");
  }
}

export function musicToolFailureMessage(error: unknown, t: ChatTranslate = chatErrorFallbackTranslate) {
  return t(protectedAppleLibraryFailureKey("read_system_music", error));
}

type LocalMailMessage = {
  sender: string;
  subject: string;
  dateReceived: string;
  read: boolean | null;
  content: string;
};

type ParsedLocalMailReadResult = {
  emails: LocalMailMessage[];
  error: string | null;
  warnings: string[];
};

function compactDisplayText(text: string) {
  return text.replace(/\s+/g, " ").trim();
}

function truncateDisplayText(text: string, maxChars: number) {
  const compacted = compactDisplayText(text);
  if (compacted.length <= maxChars) {
    return compacted;
  }
  return `${compacted.slice(0, Math.max(0, maxChars - 1)).trimEnd()}...`;
}

function localMailScopeLabel(request: Pick<DirectLocalMailReadRequest, "scope">) {
  switch (request.scope) {
    case "unread":
      return "unread Mail messages";
    case "unread_or_today":
      return "unread Mail messages and Mail from today";
    default:
      return "recent Mail messages";
  }
}

function localMailAttachmentName(request: Pick<DirectLocalMailReadRequest, "scope">) {
  switch (request.scope) {
    case "unread":
      return "local_unread_mail.json";
    case "unread_or_today":
      return "local_unread_or_today_mail.json";
    default:
      return "local_mail.json";
  }
}

function localMailAttachmentHeading(request: Pick<DirectLocalMailReadRequest, "scope">) {
  switch (request.scope) {
    case "unread":
      return "Local Mail unread-message context";
    case "unread_or_today":
      return "Local Mail unread-or-today message context";
    default:
      return "Local Mail recent-message context";
  }
}

function booleanFromMailReadValue(value: unknown) {
  if (typeof value === "boolean") {
    return value;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "true") {
      return true;
    }
    if (normalized === "false") {
      return false;
    }
  }
  return null;
}

function localMailMessageFromRecord(value: unknown): LocalMailMessage | null {
  if (!isPlainRecord(value)) {
    return null;
  }
  const sender = firstString(value.sender, value.from, value.author) ?? "";
  const subject = firstString(value.subject, value.title) ?? "";
  const dateReceived = firstString(value.dateReceived, value.date_received, value.received, value.date) ?? "";
  const content = firstString(value.content, value.body, value.excerpt, value.snippet) ?? "";
  const read = booleanFromMailReadValue(value.read ?? value.readStatus ?? value.read_status);
  if (!sender && !subject && !dateReceived && !content) {
    return null;
  }
  return {
    sender,
    subject,
    dateReceived,
    read,
    content,
  };
}

function parsedLocalMailReadPayload(payload: unknown): ParsedLocalMailReadResult | null {
  if (Array.isArray(payload)) {
    return {
      emails: payload.map(localMailMessageFromRecord).filter((email): email is LocalMailMessage => Boolean(email)),
      error: null,
      warnings: [],
    };
  }
  if (!isPlainRecord(payload)) {
    return null;
  }
  const warning = firstString(payload.warning);
  const emailsPayload = Array.isArray(payload.emails) ? payload.emails : [];
  return {
    emails: emailsPayload.map(localMailMessageFromRecord).filter((email): email is LocalMailMessage => Boolean(email)),
    error: firstString(payload.error, payload.message),
    warnings: warning ? [warning] : [],
  };
}

function parseLocalMailReadResult(resultText: string, structuredContent?: unknown): ParsedLocalMailReadResult {
  const structured = parsedLocalMailReadPayload(structuredContent);
  if (structured) {
    return structured;
  }
  try {
    const parsed = JSON.parse(resultText.trim());
    const payload = parsedLocalMailReadPayload(parsed);
    if (payload) {
      return payload;
    }
  } catch {
    void 0;
  }
  return {
    emails: [],
    error: resultText.trim() ? `Mail returned an unreadable result: ${truncateDisplayText(resultText, 240)}` : "Mail returned no readable result.",
    warnings: [],
  };
}

function parseLocalMailDate(value: string) {
  const normalized = value
    .trim()
    .replace(/\s+at\s+/i, " ")
    .replace(/\s+/g, " ");
  if (!normalized) {
    return null;
  }
  const timestamp = Date.parse(normalized);
  return Number.isNaN(timestamp) ? null : new Date(timestamp);
}

function isSameLocalCalendarDay(left: Date, right: Date) {
  return left.getFullYear() === right.getFullYear() && left.getMonth() === right.getMonth() && left.getDate() === right.getDate();
}

function localMailMessageMatchesRequestScope(request: Pick<DirectLocalMailReadRequest, "scope">, email: LocalMailMessage, now: Date) {
  if (request.scope === "unread") {
    return email.read !== true;
  }
  if (request.scope !== "unread_or_today") {
    return true;
  }
  if (email.read === false) {
    return true;
  }
  const receivedAt = parseLocalMailDate(email.dateReceived);
  return receivedAt ? isSameLocalCalendarDay(receivedAt, now) : false;
}

function scopedLocalMailReadResult(request: Pick<DirectLocalMailReadRequest, "scope">, result: ParsedLocalMailReadResult, now: Date): ParsedLocalMailReadResult {
  if (result.error && result.emails.length === 0) {
    return result;
  }
  return {
    ...result,
    emails: result.emails.filter((email) => localMailMessageMatchesRequestScope(request, email, now)),
  };
}

function localMailReadResultText(result: ParsedLocalMailReadResult) {
  if (result.error || result.warnings.length > 0) {
    return safeJsonStringify({
      ...(result.error ? { error: result.error } : {}),
      ...(result.warnings.length > 0 ? { warnings: result.warnings } : {}),
      emails: result.emails,
    });
  }
  return safeJsonStringify(result.emails);
}

function replyAssessmentForLocalMail(email: LocalMailMessage) {
  const combined = `${email.sender} ${email.subject} ${email.content}`.toLowerCase();
  if (/\b(no[-\s]?reply|donotreply|do[-\s]?not[-\s]?reply|newsletter|notification|receipt|statement|digest|alert|automated)\b/i.test(combined)) {
    return {
      label: "Probably not",
      reason: "It looks automated or informational.",
    };
  }
  if (/\?/.test(email.subject) || /\?/.test(email.content) || /\b(?:please\s+(?:reply|respond|confirm|review|approve|send|share)|let\s+me\s+know|can\s+you|could\s+you|would\s+you|do\s+you|are\s+you|need\s+your|action\s+required|approval|feedback|thoughts|available|availability|follow\s+up|respond\s+by|deadline|due)\b/i.test(combined)) {
    return {
      label: "Likely",
      reason: "The subject or excerpt asks for a response or action.",
    };
  }
  return {
    label: "Unclear",
    reason: "The bounded excerpt does not clearly request a response.",
  };
}

function localMailMessageSummaryLine(email: LocalMailMessage) {
  const subject = email.subject || "(no subject)";
  const sender = email.sender || "unknown sender";
  const received = email.dateReceived ? `, ${email.dateReceived}` : "";
  const readState = email.read === false ? "unread" : email.read === true ? "read" : "read status unknown";
  const assessment = replyAssessmentForLocalMail(email);
  const excerpt = email.content ? ` Excerpt: ${truncateDisplayText(email.content, 180)}` : "";
  return `- ${truncateDisplayText(subject, 100)} from ${truncateDisplayText(sender, 100)} (${readState}${received}): ${assessment.label}. ${assessment.reason}${excerpt}`;
}

export function buildDirectLocalMailReadAssistantText(request: Pick<DirectLocalMailReadRequest, "replyDraft" | "scope" | "unreadOnly">, resultText: string, structuredContent?: unknown, now = new Date()) {
  const parsed = scopedLocalMailReadResult(request, parseLocalMailReadResult(resultText, structuredContent), now);
  const scopeLabel = localMailScopeLabel(request);

  if (parsed.error && parsed.emails.length === 0) {
    return [`I tried to check ${scopeLabel}, but Mail did not return usable results: ${parsed.error}`, "", "I did not find any messages to summarize because the read failed, not because your inbox is clear."].join("\n");
  }

  if (parsed.emails.length === 0) {
    return [`I checked ${scopeLabel} and found no matching messages in the returned Mail results.`, "", "Nothing in that result appears to require a reply."].join("\n");
  }

  const visibleEmails = parsed.emails.slice(0, 10);
  const likelyReplyCount = parsed.emails.filter((email) => replyAssessmentForLocalMail(email).label === "Likely").length;
  const lines = [`I checked ${scopeLabel} and found ${parsed.emails.length} matching message${parsed.emails.length === 1 ? "" : "s"}.`, `Likely needs reply: ${likelyReplyCount}.`];
  if (parsed.error) {
    lines.push(`Mail also reported an error while reading: ${parsed.error}`);
  }
  if (parsed.warnings.length > 0) {
    lines.push(`Mail warning: ${parsed.warnings.join(", ")}`);
  }
  lines.push("", ...visibleEmails.map(localMailMessageSummaryLine));
  if (parsed.emails.length > visibleEmails.length) {
    lines.push(`- ${parsed.emails.length - visibleEmails.length} more matching message(s) omitted from this brief.`);
  }
  if (request.replyDraft) {
    lines.push("");
    lines.push("I did not send, archive, delete, or mark anything read. I can draft reply text from the bounded excerpt once you pick the message to answer.");
  }
  return lines.join("\n");
}

export function isUiSnapshotBlocked(payload: string) {
  try {
    const parsed = JSON.parse(payload);
    if (!Array.isArray(parsed) || parsed.length === 0) {
      return false;
    }
    return parsed.every((item) => typeof item === "string" && item.trim().toLowerCase() === "missing value");
  } catch {
    return false;
  }
}

function accessibilityBlockedNotice(t: ChatTranslateFn, appLabel: string): OomuBypassNotice {
  return {
    title: t("chat.errors.accessibility_blocked.title"),
    body: t("chat.errors.accessibility_blocked.body", { app: appLabel }),
    detail: t("chat.errors.accessibility_blocked.details"),
  };
}

function localMailToolAttachment(request: DirectLocalMailReadRequest, resultText: string): ChatAttachment {
  const text = [
    localMailAttachmentHeading(request),
    "Source: macos_applescript/read_system_emails",
    "Scope: read-only local mail metadata and bounded message excerpts.",
    "",
    request.replyDraft
      ? "Instruction: Review these messages for the user's original request and draft the reply text in this chat only. Do not open a Mail draft, send email, archive, delete, or mark messages read. If the requested target email is ambiguous, say which message you used or ask for clarification."
      : "Instruction: Summarize these messages for the user's original request. Decide whether a reply is likely needed from the sender, subject, and excerpt. Do not invent messages or claim an email was sent, archived, deleted, or marked read.",
    "",
    resultText,
  ].join("\n");
  return {
    name: localMailAttachmentName(request),
    mime_type: "application/json",
    byte_count: text.length,
    text,
  };
}

export function localCalendarToolAttachment(request: DirectLocalCalendarReadRequest, resultText: string, structuredContent?: unknown, t: ChatTranslate = chatErrorFallbackTranslate): ChatAttachment {
  const source = calendarResultSource(structuredContent, t);
  const text = [
    "Local Calendar context",
    `Source: ${source}`,
    `Scope: read-only local Calendar event metadata for ${request.label}.`,
    `Window: ${request.startDate} to ${request.endDate}.`,
    "",
    "Instruction: Summarize these events for the user's original request. Use only this local Calendar result and preserve each returned time zone or UTC offset. If no events were returned, say no events were found in the requested window. If the result says it was truncated, clearly say the calendar view is partial. Do not invent events or claim an event was created, updated, deleted, or accepted.",
    "",
    resultText,
  ].join("\n");
  return {
    name: "local_calendar.json",
    mime_type: "application/json",
    byte_count: text.length,
    text,
  };
}

function calendarResultSource(structuredContent: unknown, t: ChatTranslate = chatErrorFallbackTranslate) {
  const backend = isPlainRecord(structuredContent) ? firstString(structuredContent.backend) : null;
  return backend === "eventkit" ? t("chat.calendar_source_eventkit") : backend === "applescript" || backend === "eventkit+applescript" ? t("chat.calendar_source_applescript") : t("chat.calendar_source_native");
}

function localAppleAppToolAttachment(request: DirectLocalAppleAppReadRequest, resultText: string): ChatAttachment {
  const text = [`Local ${request.appLabel} context`, `Source: ${request.source}`, `Scope: ${request.scope}`, "", `Instruction: ${request.instruction}`, "", resultText].join("\n");
  return {
    name: request.attachmentName,
    mime_type: "application/json",
    byte_count: text.length,
    text,
  };
}

function safeJsonStringify(value: unknown) {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return "";
  }
}

export function toolErrorMessage(error: unknown) {
  if (isMcpAuthorizationError(error)) {
    return "Local tools need your explicit approval before they can cross the workspace boundary. Approve the Shield Gate request, then try again.";
  }

  const detail = safeErrorMessage(error, "");
  if (!detail || detail === "null" || looksLikeRawMcpEnvelope(detail)) {
    return "Local tool was unavailable.";
  }
  return detail;
}

export function localCommandFailureText(error: unknown, t: ChatTranslate = chatErrorFallbackTranslate) {
  const group = chatErrorGroup(stableErrorCode(error));
  if (group !== "default") {
    return chatFailureNotice(error, t).content;
  }
  return `${t("trust.local_command_failed")} ${toolErrorMessage(error)}`;
}

function invokeErrorHasCode(error: unknown, expectedCode: string) {
  return stableErrorCode(error) === expectedCode;
}

function looksLikeRawMcpEnvelope(value: string) {
  const normalized = value.trim();
  if (/["'](?:structuredContent|structured_content|isError|is_error)["']\s*:/.test(normalized) && /["']content["']\s*:/.test(normalized)) {
    return true;
  }
  try {
    const parsed = JSON.parse(normalized) as Record<string, unknown>;
    return Boolean(parsed && typeof parsed === "object" && ("isError" in parsed || "structuredContent" in parsed || "structured_content" in parsed) && "content" in parsed);
  } catch {
    return false;
  }
}

function isMcpAuthorizationError(error: unknown) {
  const code = error && typeof error === "object" && "code" in error && typeof error.code === "string" ? error.code : "";
  if (code === "mcp_permission_required" || code === "shield_approval_denied" || code === "shield_approval_not_found") {
    return true;
  }

  const message = typeof error === "string" ? error : error && typeof error === "object" && "message" in error && typeof error.message === "string" ? error.message : "";
  return /MCP workspace boundary|MCP Permission Gateway|MCP stdio server|Shield Gate approval|MCP approval token|approval token|mcp_(?:connect_server|execute_tool).*not allowed|Command "?mcp_(?:connect_server|execute_tool)"?/i.test(message);
}

function modelLabel(configuredProviders: ConfiguredProvider[], providerId: RouteProviderId, modelId: string) {
  return modelsForProvider(configuredProviders, providerId).find((model) => model.modelId === modelId)?.label ?? modelId;
}
function metadataRouteUsesLocalModel(configuredProviders: ConfiguredProvider[], metadata: ChatMessageMetadata | null | undefined, fallbackProviderId: RouteProviderId) {
  const providerId = metadata?.executingProviderId ?? metadata?.targetProviderId ?? fallbackProviderId;
  return routeUsesLocalModel(configuredProviders, providerId) || isLocalModelProviderId(providerId);
}

function routeIdFromPersistedRoute(route: PersistedModelRoute | null) {
  if (!route) {
    return null;
  }
  return `${route.providerConfigId}:${route.modelId}`;
}

function defaultReasoningForProviderRoute(configuredProviders: ConfiguredProvider[], providerId: RouteProviderId) {
  return defaultReasoningLevelForProvider(providerClassIdForRoute(configuredProviders, providerId));
}

function defaultContextBudgetTextForProvider(configuredProviders: ConfiguredProvider[], providerId: RouteProviderId, systemHardwareProfile?: SystemHardwareProfile | null) {
  return String(contextBudgetBoundsForProvider(configuredProviders, providerId, systemHardwareProfile).defaultValue);
}

function contextBudgetForRoute(route: Pick<RouteOverride, "context" | "providerId">, configuredProviders: ConfiguredProvider[], systemHardwareProfile?: SystemHardwareProfile | null) {
  const bounds = contextBudgetBoundsForProvider(configuredProviders, route.providerId, systemHardwareProfile);
  return Number(normalizeContextBudget(route.context, bounds));
}

function comparableSessionRoute(route: RouteOverride, configuredProviders: ConfiguredProvider[], systemHardwareProfile?: SystemHardwareProfile | null) {
  const supportedLevels = supportedReasoningLevelsForRoute(configuredProviders, route.providerId, route.modelId);
  return {
    providerId: route.providerId,
    providerType: route.providerType,
    modelId: route.modelId,
    reasoning: resolveReasoningFallback(route.reasoning || defaultReasoningForProviderRoute(configuredProviders, route.providerId), supportedLevels),
    contextBudget: contextBudgetForRoute(route, configuredProviders, systemHardwareProfile),
  };
}

function sessionRoutesMatch(left: RouteOverride, right: RouteOverride, configuredProviders: ConfiguredProvider[], systemHardwareProfile?: SystemHardwareProfile | null) {
  const leftRoute = comparableSessionRoute(left, configuredProviders, systemHardwareProfile);
  const rightRoute = comparableSessionRoute(right, configuredProviders, systemHardwareProfile);
  return leftRoute.providerId === rightRoute.providerId && leftRoute.providerType === rightRoute.providerType && leftRoute.modelId === rightRoute.modelId && leftRoute.reasoning === rightRoute.reasoning && leftRoute.contextBudget === rightRoute.contextBudget;
}

function normalizeRouteReasoning(route: RouteOverride, configuredProviders: ConfiguredProvider[]): RouteOverride {
  const supportedLevels = supportedReasoningLevelsForRoute(configuredProviders, route.providerId, route.modelId);
  return {
    ...route,
    reasoning: resolveReasoningFallback(route.reasoning || defaultReasoningForProviderRoute(configuredProviders, route.providerId), supportedLevels),
  };
}

function routeFromPersistedPreference(configuredProviders: ConfiguredProvider[], persistedRoute: PersistedModelRoute | null): RouteOverride | null {
  if (!persistedRoute) {
    return null;
  }
  const providerModels = modelsForProvider(configuredProviders, persistedRoute.providerId);
  const model = providerModels.find((entry) => entry.modelId === persistedRoute.modelId);
  if (!model) {
    return null;
  }
  const providerConfigId = providerConfigurationId(persistedRoute.providerId);
  return {
    providerId: providerConfigId,
    providerType: typedProviderClassIdForRoute(configuredProviders, providerConfigId),
    modelId: persistedRoute.modelId,
    reasoning: resolveReasoningFallback("medium", model.supportedReasoningLevels),
    context: defaultContextBudgetTextForProvider(configuredProviders, persistedRoute.providerId),
  };
}

function routeFromAgentEndpoint(agent: ChatAgent | undefined, configuredProviders: ConfiguredProvider[]): RouteOverride | null {
  const providerId = agent?.endpoint?.provider?.trim() ?? "";
  const modelId = agent?.endpoint?.modelId?.trim() ?? "";
  if (!providerId || !modelId) {
    return null;
  }

  const selectedModel = resolveConfiguredModelRoute(configuredProviders, providerId, modelId);
  if (!selectedModel) return null;

  const providerConfigId = selectedModel.providerConfigId;
  return {
    providerId: providerConfigId,
    providerType: selectedModel.providerType,
    modelId: selectedModel.modelId,
    reasoning: resolveReasoningFallback("medium", selectedModel.supportedReasoningLevels),
    context: defaultContextBudgetTextForProvider(configuredProviders, providerConfigId),
  };
}

function defaultRouteForAgent(agent: ChatAgent | undefined, configuredProviders: ConfiguredProvider[], primaryRoute: PersistedModelRoute | null = null, verifiedStartupModelId: string | null = null): RouteOverride {
  const agentRoute = routeFromAgentEndpoint(agent, configuredProviders);
  if (agentRoute) {
    return agentRoute;
  }

  const verifiedStartupRoute = verifiedStartupRouteForAgentEndpoint(agent?.endpoint?.provider, agent?.endpoint?.modelId, verifiedStartupModelId);
  if (verifiedStartupRoute) {
    const configuredStartupRoute = resolveConfiguredModelRoute(configuredProviders, verifiedStartupRoute.providerId, verifiedStartupRoute.modelId);
    if (configuredStartupRoute) {
      return {
        providerId: configuredStartupRoute.providerConfigId,
        providerType: configuredStartupRoute.providerType,
        modelId: configuredStartupRoute.modelId,
        reasoning: resolveReasoningFallback(defaultReasoningForProviderRoute(configuredProviders, configuredStartupRoute.providerConfigId), configuredStartupRoute.supportedReasoningLevels),
        context: defaultContextBudgetTextForProvider(configuredProviders, configuredStartupRoute.providerConfigId),
      };
    }
  }

  const persistedRoute = routeFromPersistedPreference(configuredProviders, primaryRoute);
  if (persistedRoute) {
    return persistedRoute;
  }

  const fallbackProvider = providerOptionsFromConfigured(configuredProviders)[0];
  const providerId = providerConfigurationId(fallbackProvider?.id || "local_model");
  const providerModels = modelsForProvider(configuredProviders, providerId);
  const modelId = providerModels[0]?.modelId || "";
  const selectedModel = providerModels.find((model) => model.modelId === modelId) ?? providerModels[0];

  return {
    providerId,
    providerType: typedProviderClassIdForRoute(configuredProviders, providerId),
    modelId,
    reasoning: resolveReasoningFallback("medium", selectedModel?.supportedReasoningLevels ?? DEFAULT_REASONING_LEVELS),
    context: defaultContextBudgetTextForProvider(configuredProviders, providerId),
  };
}

type ContextBudgetSliderProps = {
  bounds: ContextBudgetBounds;
  currentValue: number;
  disabled?: boolean;
  helpText: string;
  label: string;
  onChange: (value: number) => void;
};

export function contextBudgetToneForValue(bounds: ContextBudgetBounds, value: number): ContextBudgetTone {
  if (bounds.target === "cloud") {
    return "accent";
  }

  if (bounds.max >= 32_768) {
    if (value <= 16_384) return "emerald";
    if (value <= 24_576) return "amber";
    return "rose";
  }

  if (bounds.max >= 16_384) {
    if (value <= 8192) return "emerald";
    if (value <= 12_288) return "amber";
    return "rose";
  }

  return value <= 4096 ? "emerald" : "amber";
}

function contextBudgetTrackBackground(bounds: ContextBudgetBounds) {
  if (bounds.target === "cloud") {
    return "var(--border-soft)";
  }

  const green = "rgba(16, 185, 129, 0.75)";
  const amber = "rgba(245, 158, 11, 0.75)";
  const rose = "rgba(244, 63, 94, 0.75)";

  if (bounds.max >= 32_768) {
    return `linear-gradient(90deg, ${green} 0%, ${green} 50%, ${amber} 50%, ${amber} 75%, ${rose} 75%, ${rose} 100%)`;
  }

  if (bounds.max >= 16_384) {
    return `linear-gradient(90deg, ${green} 0%, ${green} 33%, ${amber} 33%, ${amber} 67%, ${rose} 67%, ${rose} 100%)`;
  }

  return `linear-gradient(90deg, ${green} 0%, ${green} 50%, ${amber} 50%, ${amber} 100%)`;
}

function contextBudgetWarningText(bounds: ContextBudgetBounds, currentValue: number, t: ChatTranslateFn) {
  if (bounds.target === "cloud") {
    return t("chat.drawer.context_budget_cloud_warning");
  }

  if (bounds.max >= 32_768) {
    if (currentValue <= 16_384) {
      return t("chat.drawer.context_budget_local_ultra_performant", {
        tier: bounds.processorTier?.trim() || t("chat.drawer.context_budget_local_hardware"),
      });
    }
    if (currentValue <= 24_576) {
      return t("chat.drawer.context_budget_local_ultra_heavy");
    }
    return t("chat.drawer.context_budget_local_ultra_warning");
  }

  if (bounds.max >= 16_384) {
    if (currentValue <= 8192) {
      return t("chat.drawer.context_budget_local_premium_performant");
    }
    if (currentValue <= 12_288) {
      return t("chat.drawer.context_budget_local_premium_moderate");
    }
    return t("chat.drawer.context_budget_local_premium_warning");
  }

  return currentValue <= 4096 ? t("chat.drawer.context_budget_local_standard_performant") : t("chat.drawer.context_budget_local_standard_warning");
}

function ContextBudgetSlider({ bounds, currentValue, disabled = false, helpText, label, onChange }: ContextBudgetSliderProps) {
  const valueIndex = Math.max(0, bounds.steps.indexOf(nearestContextBudgetStep(currentValue, bounds)));
  const tone = contextBudgetToneForValue(bounds, currentValue);

  return (
    <div>
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-medium text-[var(--foreground-subtle)]">{label}</span>
        <span className={`shrink-0 rounded-[var(--radius-sm)] bg-[var(--fill-active)] px-2 py-0.5 text-[11px] font-bold ${contextBudgetToneClass[tone]}`}>{formatContextBudgetLabel(currentValue)}</span>
      </div>
      <input
        aria-label={label}
        className="mt-3 h-2 w-full cursor-pointer appearance-none rounded-full border border-[var(--border-soft)] bg-[var(--border-soft)] accent-[var(--inverse-background)] disabled:cursor-not-allowed disabled:opacity-50"
        disabled={disabled}
        max={Math.max(bounds.steps.length - 1, 0)}
        min={0}
        onChange={(event) => onChange(bounds.steps[Number(event.target.value)] ?? bounds.defaultValue)}
        step={1}
        style={{
          accentColor: contextBudgetToneColor[tone],
          background: contextBudgetTrackBackground(bounds),
        }}
        type="range"
        value={valueIndex}
      />
      <div
        className="mt-1 grid text-[11px] font-medium text-[var(--foreground-subtle)]"
        style={{
          gridTemplateColumns: `repeat(${bounds.steps.length}, minmax(0, 1fr))`,
        }}
      >
        {bounds.steps.map((step, index) => (
          <span className={index === 0 ? "text-left" : index === bounds.steps.length - 1 ? "text-right" : "text-center"} key={step}>
            {formatContextBudgetLabel(step).replace(" tokens", "")}
          </span>
        ))}
      </div>
      <span className="mt-1 block text-[11px] leading-4 text-[var(--foreground-subtle)]">{helpText}</span>
    </div>
  );
}

export function ChatScreen({
  agents,
  configuredProviders,
  decisionBriefCompletion = "incomplete",
  sessions,
  sessionsLoaded = false,
  isVisible = true,
  activeSessionId,
  onCreateSession,
  onSelectSession,
  onDeleteSession,
  onSessionsChange,
  onStarterAction,
  onManageAgents,
  onOpenDocuments,
  onOpenModels,
  onStartGlobalChat,
  onOpenTasks,
  onOpenRoutine,
  privacySettings,
  initialBypassNotice = null,
  projectId = null,
  verifiedStartupModelId = null,
}: {
  agents: ChatAgent[];
  configuredProviders: ConfiguredProvider[];
  decisionBriefCompletion?: DecisionBriefCompletionState;
  sessions: ChatSession[];
  sessionsLoaded?: boolean;
  isVisible?: boolean;
  activeSessionId: string;
  onCreateSession: (agentId: string, route: ChatSessionRouteBinding, projectId?: string | null) => Promise<ChatSession | null>;
  onSelectSession: (sessionId: string) => void;
  onDeleteSession: (sessionId: string) => Promise<boolean>;
  onSessionsChange: (sessions: ChatSession[]) => void;
  onStarterAction?: ChatStarterHandler;
  onManageAgents?: () => void;
  onOpenDocuments?: () => void;
  onOpenModels?: () => void;
  onStartGlobalChat?: () => void;
  onOpenTasks?: () => void;
  onOpenRoutine?: NonNullable<Parameters<typeof completeOneTimeRoutineHandoff>[0]["onOpenRoutine"]>;
  privacySettings: PrivacySettingsState | null;
  initialBypassNotice?: OomuBypassNotice | null;
  projectId?: string | null;
  verifiedStartupModelId?: string | null;
}) {
  const { t, language } = useI18n();
  const projectName = useProjectName(projectId);
  const { getRiskLevelLabel, getToolKindLabel } = useHumanTrust();
  const createSessionInContext = useProjectScopedChatSessionCreator(onCreateSession, projectId);
  const verifiedExecutionCopy = useVerifiedExecutionCopy(t);
  const tRef = useRef(t);
  tRef.current = t;
  const approvals = useOptionalApproval();
  const mcp = useOptionalMcp();
  useRemoteMcpCancellation(activeSessionId, mcp?.cancelRemoteOperations);
  const [systemHardwareProfile, setSystemHardwareProfile] = useState<SystemHardwareProfile | null>(null);
  const activeSession = useMemo(() => sessions.find((session) => session.id === activeSessionId), [activeSessionId, sessions]);
  const [selectedAgentId, setSelectedAgentId] = useState(activeSession?.agentId ?? agents[0]?.id ?? "");
  const [routeOverrides, setRouteOverrides] = useState<Record<string, RouteOverride>>({});
  const [messages, setMessages, setMessagesForSession, clearSessionMessages, transcriptHydrated] = useSessionScopedState<ChatTranscriptMessage[]>(activeSessionId, []);
  const sessionContextController = useSessionContextController({
    onCompacted: async (response) => {
      await refreshSessionMessages(activeSessionId).catch(() => undefined);
      void invoke<ChatSession[]>("list_chat_sessions")
        .then(onSessionsChange)
        .catch(() => undefined);
      setChatStatus(response.compactedMessageCount > 0 ? t("chat.status.compacted") : t("chat.status.nothing_to_compact"));
    },
    refreshSignal: messages.length,
    sessionId: activeSessionId,
  });
  const verticalTemplateRoute = useVerticalTemplateParser(messages);
  const verticalTemplateIds = useMemo(() => verticalTemplateMessageIds(messages), [messages]);
  const [installedMods, setInstalledMods] = useState<InstalledModCommandSource[]>([]);
  const [liveBrowserRoute, , setLiveBrowserRouteForSession, clearSessionLiveBrowserRoute] = useSessionScopedState<BrowserSplitRoute | null>(activeSessionId, null);
  const failedBrowserNavigationUrlsRef = useRef(new Map<string, Set<string>>());
  const blockedBrowserNavigationKeysRef = useRef(new Set<string>());
  const browserResearchFallbacksRef = useRef(new Set<string>());
  const [browserNavigationBlacklistRevision, setBrowserNavigationBlacklistRevision] = useState(0);
  const storedBrowserRoute = useMemo(() => latestBrowserSplitRoute(messages, (message) => browserDirectiveGrantsForMessage(installedMods, message)), [installedMods, messages]);
  const activeBrowserRoute = useMemo(() => {
    void browserNavigationBlacklistRevision;
    const scopedLiveBrowserRoute = liveBrowserRoute && (!liveBrowserRoute.sessionId || liveBrowserRoute.sessionId === activeSessionId) ? liveBrowserRoute : null;
    const availableLiveBrowserRoute = scopedLiveBrowserRoute && !browserNavigationIsBlacklisted(scopedLiveBrowserRoute.url, scopedLiveBrowserRoute.sessionId ?? activeSessionId, failedBrowserNavigationUrlsRef.current) ? scopedLiveBrowserRoute : null;
    const availableStoredBrowserRoute = storedBrowserRoute && !browserNavigationIsBlacklisted(storedBrowserRoute.url, storedBrowserRoute.sessionId ?? activeSessionId, failedBrowserNavigationUrlsRef.current) ? storedBrowserRoute : null;
    if (!availableLiveBrowserRoute) {
      return availableStoredBrowserRoute;
    }
    if (!availableStoredBrowserRoute || availableLiveBrowserRoute.messageId >= availableStoredBrowserRoute.messageId) {
      return availableLiveBrowserRoute;
    }
    return availableStoredBrowserRoute;
  }, [activeSessionId, liveBrowserRoute, storedBrowserRoute, browserNavigationBlacklistRevision]);
  const splitViewDirectiveIds = useMemo(() => splitViewDirectiveMessageIds(messages), [messages]);
  const [isSending, setIsSending, setIsSendingForSession, clearSessionSending] = useSessionScopedState(activeSessionId, false);
  const [isProcessing, , setIsProcessingForSession, clearSessionProcessing, , isProcessingForSession] = useSessionScopedState(activeSessionId, false);
  const [activeStreamId, , setActiveStreamIdForSession, clearSessionStream] = useSessionScopedState<string | null>(activeSessionId, null);
  const [isSendMenuOpen, setIsSendMenuOpen] = useState(false);
  const [composerResetSignal, , setComposerResetSignalForSession, clearSessionComposerReset] = useSessionScopedState(activeSessionId, 0);
  const [composerDraft, setComposerDraft, setComposerDraftForSession, clearSessionDraft] = useSessionScopedState(activeSessionId, "");
  const [chatStatus, setChatStatus, setChatStatusForSession, clearSessionStatus] = useSessionScopedState(activeSessionId, t("chat.status.ready"));
  const [headlessSearchDebug, , setHeadlessSearchDebugForSession] = useSessionScopedState<HeadlessSearchDebug | null>(activeSessionId, null);
  const { autoRouteAttention, requestAutoRouteTurnChoice, resolveAutoRouteTurnChoice, cancelAutoRouteTurnChoiceForSession, directApplePermissionAttention, directApplePermissionActions, requestDirectApplePermissionRecovery } = useChatTurnRecovery({
    activeSessionId,
    messages,
    onOpenModels,
    restoreEnabled: transcriptHydrated,
    setProcessingForSession: setIsProcessingForSession,
    setSendingForSession: setIsSendingForSession,
    setStatusForSession: setChatStatusForSession,
    submit: handleSubmit,
    translate: t,
  });
  const cloudConsent = useChatCloudConsent({
    activeSessionId,
    t,
    setSendingForSession: setIsSendingForSession,
    setProcessingForSession: setIsProcessingForSession,
    setStatusForSession: setChatStatusForSession,
  });
  const [bypassNotice, setBypassNotice, setBypassNoticeForSession, clearSessionBypassNotice] = useSessionScopedState<OomuBypassNotice | null>(activeSessionId, null);
  const [attachments, setAttachments, setAttachmentsForSession, clearSessionAttachments] = useSessionScopedState<ChatAttachment[]>(activeSessionId, []);
  const attachmentPayloadsBySessionRef = useRef(new Map<string, ChatAttachment[]>());
  useLayoutEffect(() => {
    attachmentPayloadsBySessionRef.current.set(chatSessionStateScope(activeSessionId), attachments);
  }, [activeSessionId, attachments]);
  const [isReadingAttachments, , setIsReadingAttachmentsForSession, clearSessionAttachmentRead] = useSessionScopedState(activeSessionId, false);
  const attachmentReadAbortRef = useRef<AbortController | null>(null);
  const activeTurnsRef = useRef(new Map<string, ChatTurnContext>());
  const { activeStreamIdsRef, activeAssistantMessageIdsRef, activeTurnForSession, registerActiveTurn, turnIsCurrent, clearActiveTurn, updateTurnMessages, updateTurnStatus } = useActiveChatTurns<ChatTranscriptMessage>({
    activeTurnsRef,
    setActiveStreamId: setActiveStreamIdForSession,
    setSending: setIsSendingForSession,
    setProcessing: setIsProcessingForSession,
    setMessages: setMessagesForSession,
    setStatus: setChatStatusForSession,
  });
  useEffect(() => {
    const payloadsBySession = attachmentPayloadsBySessionRef.current;
    const activeTurns = activeTurnsRef.current;
    return () => {
      attachmentReadAbortRef.current?.abort();
      attachmentReadAbortRef.current = null;
      for (const scopedAttachments of payloadsBySession.values()) {
        releaseAttachmentPayloads(scopedAttachments);
      }
      payloadsBySession.clear();
      if (isTauriRuntime) {
        for (const searchOwner of activeTurns.values()) {
          void invoke("cancel_sovereign_search", {
            request: {
              sessionId: searchOwner.sessionId,
              originTurnId: searchOwner.turnId,
              originGenerationToken: searchOwner.generationToken,
            },
          }).catch(() => undefined);
        }
      }
    };
  }, []);
  useEffect(() => {
    const sessionScope = chatSessionStateScope(activeSessionId);
    const activeTurns = activeTurnsRef.current;
    return () => {
      if (!isTauriRuntime || activeTurns.has(sessionScope)) {
        return;
      }
      void invoke("revoke_local_context_grants", {
        request: { sessionId: sessionScope },
      }).catch(() => undefined);
    };
  }, [activeSessionId]);
  const [isDrawerOpen, setIsDrawerOpen] = useState(false);
  const [dismissedSplitRoutes, dismissSplitRoute, restoreSplitRoute] = usePersistedDismissedSplitRoutes();
  const chatRootRef = useRef<HTMLElement>(null);
  const sessionsLiveRef = useRef(0);
  const splitLiveRef = useRef(0);
  const tuningLiveRef = useRef(0);
  const chatRootWidth = useContainerWidth(chatRootRef);
  const sessionsPanel = useResizablePanel({
    storageKey: "oomu.chat.sessionsWidth",
    defaultWidth: 256,
    min: 170,
    max: 420,
    side: "right",
    liveWidthRef: sessionsLiveRef,
  });
  const splitPanel = useResizablePanel({
    storageKey: "oomu.chat.splitWidth",
    defaultWidth: 360,
    min: 280,
    max: 520,
    side: "left",
    liveWidthRef: splitLiveRef,
  });
  const tuningPanel = useResizablePanel({
    storageKey: "oomu.chat.tuningWidth",
    defaultWidth: 320,
    min: 230,
    max: 460,
    side: "left",
    liveWidthRef: tuningLiveRef,
  });
  function registerFailedBrowserNavigation(url: string, sessionId?: string | null) {
    const key = normalizedBrowserNavigationKey(url);
    if (!key) {
      return false;
    }
    const scope = browserNavigationScope(sessionId ?? activeSessionId);
    const failedUrls = failedBrowserNavigationUrlsRef.current.get(scope) ?? new Set<string>();
    if (failedUrls.has(key)) {
      return false;
    }
    failedUrls.add(key);
    failedBrowserNavigationUrlsRef.current.set(scope, failedUrls);
    setBrowserNavigationBlacklistRevision((value) => value + 1);
    return true;
  }
  function recordBlockedBrowserNavigation(route: BrowserSplitRoute) {
    const routeSessionId = route.sessionId ?? activeSessionIdRef.current;
    if (!routeSessionId) {
      return;
    }
    const key = `${browserNavigationScope(routeSessionId)}:${route.url}`;
    if (blockedBrowserNavigationKeysRef.current.has(key)) {
      return;
    }
    blockedBrowserNavigationKeysRef.current.add(key);
    const notice = browserNavigationBlockedNotice(t);
    setMessagesForSession(routeSessionId, (current) => [
      ...current,
      {
        id: nextMessageIdRef.current++,
        role: "system",
        content: notice.message,
      },
    ]);
    setChatStatusForSession(routeSessionId, notice.status);
  }
  function activateBrowserSplitRoute(route: BrowserSplitRoute) {
    const routeSessionId = route.sessionId ?? activeSessionIdRef.current;
    if (!routeSessionId) {
      return;
    }
    if (browserNavigationIsBlacklisted(route.url, routeSessionId, failedBrowserNavigationUrlsRef.current)) {
      recordBlockedBrowserNavigation(route);
      return;
    }
    setLiveBrowserRouteForSession(routeSessionId, (current) => {
      if (current?.sessionId === route.sessionId && current?.messageId === route.messageId && current?.url === route.url && current?.reason === route.reason) {
        return current;
      }
      return route;
    });
  }
  const continueBrowserResearchAfterNavigationFailure = (route: BrowserSplitRoute, failureCode: RecoverableBrowserFailure) =>
    continueBrowserResearchHeadlessly(route, failureCode, {
      activeSessionId: activeSessionIdRef.current,
      messages,
      seenFallbacks: browserResearchFallbacksRef.current,
      registerFailedNavigation: registerFailedBrowserNavigation,
      runLocalSearch: buildLocalSearchOutcome,
      addSystemMessage: (sessionId, content) => {
        setMessagesForSession(sessionId, (current) => [...current, { id: nextMessageIdRef.current++, role: "system", content }]);
      },
      setStatus: setChatStatusForSession,
      refreshMessages: refreshSessionMessages,
      translate: t,
    });
  const activeSplitModProvider = ((): SplitPanelProvider | null => {
    if (activeBrowserRoute && (!verticalTemplateRoute || activeBrowserRoute.messageId >= verticalTemplateRoute.messageId)) {
      return {
        id: BROWSER_SPLIT_MOD_ID,
        routeIdentity: browserSplitPanelRouteIdentity(activeBrowserRoute, activeSessionId),
        label: t("chat.browser.eyebrow"),
        resizeLabel: t("chat.browser.resize"),
        render: () => <BrowserModPanel key={`${activeBrowserRoute.messageId}:${activeBrowserRoute.url}`} onResearchRouteUnavailable={continueBrowserResearchAfterNavigationFailure} route={activeBrowserRoute} />,
      };
    }
    if (verticalTemplateRoute) {
      return {
        id: OPERATION_CONTROL_SPLIT_MOD_ID,
        routeIdentity: splitPanelRouteIdentity(activeSessionId, OPERATION_CONTROL_SPLIT_MOD_ID, verticalTemplateRoute.messageId),
        label: t("chat.operation.title"),
        resizeLabel: t("chat.operation.resize"),
        render: () => <OperationControlPanel route={verticalTemplateRoute} />,
      };
    }
    return null;
  })();
  const hasSplitPanelContent = Boolean(activeSplitModProvider);
  const activeSplitRouteIdentity = activeSplitModProvider?.routeIdentity ?? null;
  const isSplitPanelOpen = activeSplitRouteIdentity ? !dismissedSplitRoutes.has(activeSplitRouteIdentity) : true;
  const splitInlineOpen = isSplitPanelOpen && hasSplitPanelContent;
  const TUNING_INLINE_MIN_ROOT = 742;
  const tuningInlineMinRoot = TUNING_INLINE_MIN_ROOT + (splitInlineOpen ? splitPanel.min + 1 : 0);
  const tuningIsOverlay = isDrawerOpen && chatRootWidth > 0 && chatRootWidth < tuningInlineMinRoot;
  const tuningInlineOpen = isDrawerOpen && !tuningIsOverlay;
  const fittedPanels = fitChatPanels(chatRootWidth, sessionsPanel.width, tuningPanel.width, tuningInlineOpen, {
    mainMin: 320,
    sessionsMin: 170,
    splitMin: 280,
    splitOpen: splitInlineOpen,
    splitStored: splitPanel.width,
    tuningMin: 230,
  });
  const overlayTuningWidth = Math.min(Math.max(tuningPanel.width, 280), Math.max(260, chatRootWidth - fittedPanels.sessions - 56));
  sessionsLiveRef.current = fittedPanels.sessions;
  splitLiveRef.current = fittedPanels.split;
  tuningLiveRef.current = fittedPanels.tuning;
  const [pendingPlan, , setPendingPlanForSession, clearSessionPendingPlan] = useSessionScopedState<PendingActionPlan | null>(activeSessionId, null);
  const [completedRecoveryActionKeys, , setCompletedRecoveryActionKeysForSession] = useSessionScopedState<ReadonlySet<string>>(activeSessionId, new Set());
  const [isExecutingPlan, , setIsExecutingPlanForSession, clearSessionPlanExecution] = useSessionScopedState(activeSessionId, false);
  const [activeExecution, setActiveExecution, setActiveExecutionForSession, clearSessionActiveExecution] = useSessionScopedState<ActiveAgentExecution | null>(activeSessionId, null);
  const [queuedMessages, setQueuedMessages, setQueuedMessagesForSession, clearSessionQueue] = useSessionScopedState<QueuedMessageRecord[]>(activeSessionId, []);
  const [isQueueExecuting, , setIsQueueExecutingForSession, clearSessionQueueExecution] = useSessionScopedState(activeSessionId, false);
  const [isSavingWebGroundingOverride, setIsSavingWebGroundingOverride] = useState(false);
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [editingSessionTitle, setEditingSessionTitle] = useState("");
  const [isRenamingSession, setIsRenamingSession] = useState(false);
  const transcriptScrollRef = useRef<HTMLDivElement>(null);
  const nextMessageIdRef = useRef(1);
  const pendingSubmissions = usePendingChatSubmissions(activeSessionId, selectedAgentId);
  const skipRenameCommitRef = useRef(false);
  const pendingSteersRef = useRef(new Map<string, PendingSteerTurn>());
  const turnReconciliationControllersRef = useRef(new Map<string, AbortController>());
  useEffect(
    () => () => {
      for (const pendingSteer of pendingSteersRef.current.values()) {
        releaseAttachmentPayloads(pendingSteer.attachments);
      }
      pendingSteersRef.current.clear();
      for (const controller of turnReconciliationControllersRef.current.values()) {
        controller.abort();
      }
      turnReconciliationControllersRef.current.clear();
    },
    [],
  );
  const cancelledGenerationTokensRef = useRef(new Set<string>());
  const pendingSidebarAgentRef = useRef<string | null>(null);
  const executingQueueSessionsRef = useRef(new Set<string>());
  const executionCleanupTimeoutsRef = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const executionSubscriptionRef = useRef<{
    executionId: string;
    cancelled: boolean;
  } | null>(null);
  const executionStartRequestsRef = useRef(new Set<string>());
  const onSessionsChangeRef = useRef(onSessionsChange);
  const refreshSessionMessagesRef = useRef(refreshSessionMessages);
  const sessionHydrationLocksRef = useRef(new Map<string, number>());
  const sessionHydrationVersionsRef = useRef(new Map<string, number>());
  const nextHydrationLockTokenRef = useRef(1);
  const sessionConfigHydrationTokenRef = useRef(0);
  const activeSessionRef = useRef(activeSession);
  const activeSessionIdRef = useRef(activeSessionId);
  activeSessionIdRef.current = activeSessionId;
  const pendingConfigPersistCountsRef = useRef(new Map<string, number>());
  const latestSessionConfigRouteRef = useRef(new Map<string, RouteOverride>());
  const sessionConfigPersistPromisesRef = useRef(new Map<string, Promise<boolean>>());
  const publishBackgroundCompletionAttention = useChatCompletionAttention({
    activeSessionId,
    isNativeRuntime: isTauriRuntime,
    isVisible,
    onSessionsChange,
    unreadCompletion: Boolean(activeSession?.unreadCompletion),
  });
  const providerOptions = useMemo(() => providerOptionsFromConfigured(configuredProviders), [configuredProviders]);
  const { primaryRoute, fallbackRoute } = useModelRoutingPreferences();
  useEffect(() => {
    if (!isTauriRuntime) {
      return;
    }
    let cancelled = false;
    invoke<SystemHardwareProfile>("get_system_hardware_profile")
      .then((profile) => {
        if (!cancelled) {
          setSystemHardwareProfile(profile);
        }
      })
      .catch((error) => {
        console.warn("Unable to query system hardware profile.", error);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const effectiveSelectedAgentId = activeSession?.agentId ?? selectedAgentId;
  const selectedAgent = useMemo(() => agents.find((agent) => agent.id === effectiveSelectedAgentId) ?? agents[0], [agents, effectiveSelectedAgentId]);
  const activeAgentId = selectedAgent?.id ?? "";
  const initialBypassNoticeSeededRef = useRef(false);
  useEffect(() => {
    if (initialBypassNoticeSeededRef.current || !initialBypassNotice || !activeSessionId) {
      return;
    }
    initialBypassNoticeSeededRef.current = true;
    setBypassNoticeForSession(activeSessionId, initialBypassNotice);
  }, [activeSessionId, initialBypassNotice, setBypassNoticeForSession]);
  useEffect(() => {
    if (!isTauriRuntime) {
      setInstalledMods([]);
      return;
    }
    let cancelled = false;
    invoke<InstalledModCommandSource[]>("list_installed_mods")
      .then((mods) => {
        if (!cancelled) {
          setInstalledMods(Array.isArray(mods) ? mods : []);
        }
      })
      .catch((error) => {
        console.warn("Unable to load OOMU mod slash commands.", error);
        if (!cancelled) {
          setInstalledMods([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [activeAgentId]);
  const availableSlashCommands = useMemo<SlashCommandOption[]>(() => {
    return installedMods
      .filter((mod) => mod.isActive && Array.isArray(mod.commands))
      .flatMap((mod) =>
        (mod.commands ?? [])
          .map((command) => ({
            trigger: command.trigger.trim(),
            description: localizedModText(command.description, language),
            modId: mod.id,
            modName: mod.name,
          }))
          .filter((command) => command.trigger.startsWith("/")),
      );
  }, [installedMods, language]);
  const selectedAgentDefaultRoute = useMemo(() => defaultRouteForAgent(selectedAgent, configuredProviders, primaryRoute, verifiedStartupModelId), [configuredProviders, primaryRoute, selectedAgent, verifiedStartupModelId]);
  const route = useMemo(() => (activeAgentId ? (activeSessionId ? (routeOverrides[activeSessionId] ?? selectedAgentDefaultRoute) : (routeOverrides[`agent:${activeAgentId}`] ?? selectedAgentDefaultRoute)) : defaultRouteForAgent(undefined, configuredProviders, primaryRoute, verifiedStartupModelId)), [activeAgentId, activeSessionId, configuredProviders, primaryRoute, routeOverrides, selectedAgentDefaultRoute, verifiedStartupModelId]);
  const routeRef = useRef(route);
  const activeRouteUsesLocalModel = useMemo(() => routeUsesLocalModel(configuredProviders, route.providerId), [configuredProviders, route.providerId]);
  const latestAssistantRouteMetadata = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (message.role === "assistant" && (message.metadata || message.providerId || message.modelId)) {
        return {
          providerId: message.providerId ?? null,
          modelId: message.modelId ?? null,
          metadata: message.metadata ?? null,
        };
      }
    }
    return null;
  }, [messages]);
  const routingIndicatorState = useMemo(() => {
    const metadata = latestAssistantRouteMetadata?.metadata ?? null;
    const providerId = metadata?.executingProviderId ?? metadata?.targetProviderId ?? latestAssistantRouteMetadata?.providerId ?? route.providerId;
    const modelId = metadata?.executingModelId ?? metadata?.targetModelId ?? latestAssistantRouteMetadata?.modelId ?? route.modelId;
    return {
      hasHistory: Boolean(latestAssistantRouteMetadata),
      isLocal: latestAssistantRouteMetadata ? metadataRouteUsesLocalModel(configuredProviders, metadata, providerId) : activeRouteUsesLocalModel,
      modelId: modelId || route.modelId || "selected model",
    };
  }, [activeRouteUsesLocalModel, configuredProviders, latestAssistantRouteMetadata, route.modelId, route.providerId]);
  const routeModelOptions = useMemo(() => modelsForProvider(configuredProviders, route.providerId), [configuredProviders, route.providerId]);
  const modelOptions = useMemo(
    () =>
      routeModelOptions.some((model) => model.modelId === route.modelId) || !route.modelId
        ? routeModelOptions
        : [
            {
              providerId: route.providerId,
              providerName: route.providerId,
              modelId: route.modelId,
              label: t("common.saved_not_configured", { name: route.modelId }),
              context: "provider-defined",
              supportedReasoningLevels: supportedReasoningLevelsForRoute(configuredProviders, route.providerId, route.modelId),
            },
            ...routeModelOptions,
          ],
    [configuredProviders, route.modelId, route.providerId, routeModelOptions, t],
  );
  const activeModelOption = useMemo(() => modelOptions.find((model) => model.modelId === route.modelId), [modelOptions, route.modelId]);
  const activeReasoningLevels = useMemo(() => activeModelOption?.supportedReasoningLevels ?? supportedReasoningLevelsForRoute(configuredProviders, route.providerId, route.modelId), [activeModelOption?.supportedReasoningLevels, configuredProviders, route.modelId, route.providerId]);
  const activeReasoningLevel = resolveReasoningFallback(route.reasoning, activeReasoningLevels);
  const activeContextBudgetBounds = useMemo(() => contextBudgetBoundsForProvider(configuredProviders, route.providerId, systemHardwareProfile), [configuredProviders, route.providerId, systemHardwareProfile]);
  const activeContextBudget = useMemo(() => contextBudgetForRoute(route, configuredProviders, systemHardwareProfile), [configuredProviders, route, systemHardwareProfile]);
  const activeContextBudgetText = String(activeContextBudget);
  const activePrimaryRouteId = useMemo(() => routeIdFromPersistedRoute(primaryRoute), [primaryRoute]);
  const activeFallbackRouteId = useMemo(() => routeIdFromPersistedRoute(fallbackRoute), [fallbackRoute]);
  const sessionWebGroundingOverride = activeSession?.webGroundingOverride ?? null;
  const automatedWebGroundingEnabled = sessionWebGroundingOverride ?? privacySettings?.automatedWebGroundingEnabled ?? false;
  const sessionDynamicRoutingOverride = activeSession?.dynamicRoutingOverride ?? null;
  const selectedAgentDynamicRoutingDefault = dynamicRoutingDefaultForAgent(selectedAgent);
  const dynamicRoutingEnabled = sessionDynamicRoutingOverride ?? selectedAgentDynamicRoutingDefault;
  const {
    failure: autoRouteActivationFailure,
    isSaving: isSavingDynamicRoutingOverride,
    keepCommittedRoute: keepAutoRoute,
    refreshNonce: routeRefreshNonce,
    toggle: handleDynamicRoutingToggle,
  } = useAutoRouteActivation({
    activeSessionId,
    canActivate: Boolean(selectedAgent),
    buildBaseline: (selectedRoute) => buildAutoRouteBaseline(selectedRoute, supportedReasoningLevelsForRoute(configuredProviders, selectedRoute.providerId, selectedRoute.modelId), contextBudgetForRoute(selectedRoute, configuredProviders, systemHardwareProfile)),
    dynamicRoutingEnabled,
    ensureSession: (id) => ensureActiveSessionForMutation(id, false),
    getRoute: () => routeRef.current,
    onSessionsChange,
    sessions,
    setStatus: setChatStatus,
    statusBlocked: t("chat.status.routing_setting_blocked"),
    statusDisabled: t("chat.status.model_locked"),
    statusEnabled: t("chat.status.dynamic_routing_on"),
    unlockSession: unlockSessionHydration,
  });
  const { autoRouteCloudModelId, autoRouteSessionReadiness, localModelStatus } = useAutoRouteRuntimeState({
    attention: autoRouteAttention,
    configuredProviders,
    dynamicRoutingEnabled,
    localModelId: route.modelId,
    resolveChoice: resolveAutoRouteTurnChoice,
    sessionId: activeSessionId,
  });
  const localModelIsHydrating = !dynamicRoutingEnabled && activeRouteUsesLocalModel && localModelStatus === "loading";
  const tuningControlsDisabled = !activeSessionId;
  const agentById = useMemo(() => new Map(agents.map((agent) => [agent.id, agent])), [agents]);
  const conversationalMcpCapabilities = useMemo(() => conversationalMcpCapabilitiesFromServers(mcp?.servers), [mcp?.servers]);
  async function handleNewChat() {
    if (!selectedAgent) return;
    const nextRoute = selectedAgentDefaultRoute;
    const session = await createSessionInContext(selectedAgent.id, routeBindingForDynamicRouting(dynamicRoutingDefaultForAgent(selectedAgent), nextRoute));
    if (session) {
      setRouteOverrides((current) => ({
        ...current,
        [session.id]: nextRoute,
      }));
      if (!sessionUsesDynamicBinding(session)) {
        void persistSessionConfig(session.id, nextRoute);
      }
      onSelectSession(session.id);
    }
  }
  function beginRenameSession(session: ChatSession) {
    skipRenameCommitRef.current = false;
    setEditingSessionId(session.id);
    setEditingSessionTitle(session.title);
  }
  function cancelRenameSession() {
    setEditingSessionId(null);
    setEditingSessionTitle("");
  }
  async function handleDeleteChatSession(sessionId: string) {
    const streamId = activeStreamIdsRef.current.get(sessionId);
    if (streamId) {
      handleStopGeneration(streamId, sessionId, true);
    }
    const deleted = await onDeleteSession(sessionId);
    if (!deleted) {
      setChatStatusForSession(sessionId, t("chat.status.delete_failed"));
      return;
    }
    pendingSubmissions.removeSession(sessionId);
    discardPendingSteer(sessionId);
    await approvals?.cancelApprovalsForSession(sessionId);
    activeTurnsRef.current.delete(sessionId);
    activeStreamIdsRef.current.delete(sessionId);
    activeAssistantMessageIdsRef.current.delete(sessionId);
    executingQueueSessionsRef.current.delete(sessionId);
    latestSessionConfigRouteRef.current.delete(sessionId);
    sessionHydrationLocksRef.current.delete(sessionId);
    sessionHydrationVersionsRef.current.set(sessionId, nextHydrationLockTokenRef.current++);
    pendingConfigPersistCountsRef.current.delete(sessionId);
    sessionConfigPersistPromisesRef.current.delete(sessionId);
    void cancelAutoRouteTurnChoiceForSession(sessionId).catch(() => undefined);
    cloudConsent.cancelChatCloudConsentForSession(sessionId);
    setRouteOverrides((current) => {
      if (!(sessionId in current)) return current;
      const next = { ...current };
      delete next[sessionId];
      return next;
    });
    clearSessionMessages(sessionId);
    clearSessionSending(sessionId);
    clearSessionProcessing(sessionId);
    clearSessionStream(sessionId);
    clearSessionComposerReset(sessionId);
    clearSessionDraft(sessionId);
    clearSessionStatus(sessionId);
    clearSessionBypassNotice(sessionId);
    clearSessionLiveBrowserRoute(sessionId);
    clearSessionAttachments(sessionId);
    clearSessionAttachmentRead(sessionId);
    clearSessionPendingPlan(sessionId);
    clearSessionPlanExecution(sessionId);
    clearSessionQueue(sessionId);
    clearSessionQueueExecution(sessionId);
    clearStoredActiveExecution(sessionId);
    const executionCleanupTimeout = executionCleanupTimeoutsRef.current.get(sessionId);
    if (executionCleanupTimeout) {
      clearTimeout(executionCleanupTimeout);
      executionCleanupTimeoutsRef.current.delete(sessionId);
    }
    clearSessionActiveExecution(sessionId);
  }
  function abortRenameSession() {
    skipRenameCommitRef.current = true;
    cancelRenameSession();
  }
  async function commitRenameSession(session: ChatSession) {
    const title = editingSessionTitle.trim();
    if (!title || title === session.title) {
      cancelRenameSession();
      return;
    }
    setIsRenamingSession(true);
    try {
      const updatedSession = await invoke<ChatSession>("rename_chat_session", {
        request: {
          sessionId: session.id,
          title,
        },
      });
      onSessionsChange(sessions.map((entry) => (entry.id === updatedSession.id ? updatedSession : entry)).sort((a, b) => b.updatedAtMs - a.updatedAtMs));
      cancelRenameSession();
      setChatStatus(t("chat.status.renamed"));
    } catch {
      setChatStatus(t("chat.status.rename_blocked"));
    } finally {
      setIsRenamingSession(false);
    }
  }
  const sanitizePersistedApprovedFileMarkers = useCallback(
    (content: string) => {
      return content.replace(/\[approved file:\s*([^\]\r\n]+)\]/gi, (_marker, name: string) => name.trim()).replace(/\[approved file\]/gi, t("permissions.selected_file"));
    },
    [t],
  );
  const storedMessagesToTranscript = useCallback(
    (stored: StoredChatMessage[]) => {
      return stored.flatMap((entry, index) => {
        const metadata = normalizeChatMessageMetadata(entry.metadataJson, entry.providerId, entry.modelId);
        if (isInternalUiOnlyCheckpoint(metadata)) return [];
        const content = localizedUiCheckpointContent(metadata, t) ?? (entry.role === "assistant" ? localizedAssistantTerminalContent(localizePersistedAgentExecutionReceipt(sanitizeAssistantTranscriptText(entry.content, t), t), metadata, t) : sanitizePersistedApprovedFileMarkers(entry.content));
        const precedingUserPrompt =
          stored
            .slice(0, index)
            .reverse()
            .find((message) => message.role === "user")?.content ?? "";
        const safePrecedingUserPrompt = sanitizePersistedApprovedFileMarkers(precedingUserPrompt);
        const unverifiedActionClaim = entry.role === "assistant" && evaluateUnverifiedActionClaim(content, safePrecedingUserPrompt, hasLikelyLocalNativeTaskIntent(safePrecedingUserPrompt) || hasExplicitBrowserNavigationIntent(safePrecedingUserPrompt), metadata?.verifiedNativeExecutionReceipt === true);
        return [
          {
            id: entry.id || index + 1,
            role: unverifiedActionClaim ? ("system" as const) : entry.role,
            providerId: entry.providerId ?? null,
            modelId: entry.modelId ?? null,
            metadata,
            isPending: false,
            isCompacted: entry.isCompacted,
            compactionType: entry.compactionType ?? null,
            content: unverifiedActionClaim ? t("trust.unverified_action_claim") : content,
          },
        ];
      });
    },
    [sanitizePersistedApprovedFileMarkers, t],
  );
  const replaceTranscript = useCallback(
    (transcript: ChatTranscriptMessage[], targetSessionId = activeSessionId) => {
      if (!targetSessionId) {
        return;
      }
      const transcriptNextId = transcript.reduce((max, entry) => Math.max(max, entry.id), 0) + 1;
      nextMessageIdRef.current = Math.max(nextMessageIdRef.current, transcriptNextId);
      setMessagesForSession(targetSessionId, transcript);
    },
    [activeSessionId, setMessagesForSession],
  );
  async function refreshSessionMessages(sessionId: string, options?: { hydrationLockToken?: number | null }) {
    const targetSessionId = sessionId.trim();
    if (!targetSessionId) return false;
    const expectedVersion = sessionHydrationVersionsRef.current.get(targetSessionId) ?? 0;
    const ownerToken = options?.hydrationLockToken ?? null;
    const refreshIsCurrent = () => {
      const activeLock = sessionHydrationLocksRef.current.get(targetSessionId);
      const lockMatches = ownerToken === null ? activeLock === undefined : activeLock === ownerToken;
      return lockMatches && (sessionHydrationVersionsRef.current.get(targetSessionId) ?? 0) === expectedVersion;
    };
    if (!refreshIsCurrent()) {
      return false;
    }
    const stored = await invoke<StoredChatMessage[]>("list_chat_messages", {
      sessionId: targetSessionId,
      session_id: targetSessionId,
    });
    if (!refreshIsCurrent()) {
      return false;
    }
    replaceTranscript(storedMessagesToTranscript(stored), targetSessionId);
    return true;
  }
  async function reconcileTerminalChatTurn(turnContext: ChatTurnContext, hydrationLockToken: number | null, shouldContinue: () => boolean) {
    turnReconciliationControllersRef.current.get(turnContext.sessionId)?.abort();
    const controller = new AbortController();
    turnReconciliationControllersRef.current.set(turnContext.sessionId, controller);
    const result = await waitForTerminalChatTurnResult(
      () =>
        invoke<StoredChatMessage[]>("list_chat_messages", {
          sessionId: turnContext.sessionId,
          session_id: turnContext.sessionId,
        }),
      turnContext.turnId,
      { signal: controller.signal, shouldContinue },
    ).finally(() => {
      if (turnReconciliationControllersRef.current.get(turnContext.sessionId) === controller) {
        turnReconciliationControllersRef.current.delete(turnContext.sessionId);
      }
    });
    if (result.status !== "terminal") return result.status;
    if (hydrationLockToken === null || sessionHydrationLocksRef.current.get(turnContext.sessionId) !== hydrationLockToken) {
      return "cancelled" as const;
    }
    replaceTranscript(storedMessagesToTranscript(result.messages), turnContext.sessionId);
    return "terminal" as const;
  }
  async function abandonAcceptedTurnAfterOwnershipLoss(context: ChatTurnContext, hydrationLockToken: number | null = null) {
    await abandonDurableChatTurn(context, t("chat.errors.turn_persistence.content")).catch(() => null);
    await refreshSessionMessages(context.sessionId, {
      hydrationLockToken,
    }).catch(() => undefined);
  }
  async function surfaceStoppedTurn(errorCode: string, context: ChatTurnContext, hydrationLockToken: number | null, assistantMessageId: number | null) {
    return surfaceStoppedChatTurn<ChatTurnContext, ChatTranscriptMessage>({
      errorCode,
      context,
      assistantMessageId,
      content: t("tasks.error_cancelled"),
      status: t("chat.status.generation_stopped"),
      finalize: (turn) =>
        finalizeDurableChatTurn(turn, {
          role: "system",
          content: t("tasks.error_cancelled"),
          status: "cancelled",
        }),
      refresh: (sessionId) => refreshSessionMessages(sessionId, { hydrationLockToken }),
      updateMessages: updateTurnMessages,
      updateStatus: updateTurnStatus,
      createId: () => nextMessageIdRef.current++,
    });
  }
  async function refreshQueuedMessages(sessionId = activeSessionId, options?: { hydrationLockToken?: number | null }) {
    const targetSessionId = sessionId.trim();
    if (!targetSessionId) {
      return [];
    }
    const expectedVersion = sessionHydrationVersionsRef.current.get(targetSessionId) ?? 0;
    const ownerToken = options?.hydrationLockToken ?? null;
    const refreshIsCurrent = () => {
      const activeLock = sessionHydrationLocksRef.current.get(targetSessionId);
      const lockMatches = ownerToken === null ? activeLock === undefined : activeLock === ownerToken;
      return lockMatches && (sessionHydrationVersionsRef.current.get(targetSessionId) ?? 0) === expectedVersion;
    };
    if (!refreshIsCurrent()) {
      return [];
    }
    const queued = await invoke<QueuedMessageRecord[]>("get_queued_messages", {
      sessionId: targetSessionId,
      session_id: targetSessionId,
    });
    if (!refreshIsCurrent()) {
      return [];
    }
    setQueuedMessagesForSession(targetSessionId, queued);
    return queued;
  }
  async function requestMcpShieldApproval(request: McpToolApprovalRequest, turnContext: ChatTurnContext, exactArguments?: unknown) {
    if (!turnIsCurrent(turnContext) || !approvals) return DENIED_ONCE_APPROVAL;
    setChatStatusForSession(turnContext.sessionId, t("chat.status.waiting_approval"));
    const nativeApproval = nativeAppleAppApprovalPresentation(request.toolName, exactArguments);
    const result = await approvals.requestApproval(chatMcpShieldApprovalRequest(request, turnContext, nativeApproval, mcpTargetPath(request.arguments)));
    const current = turnIsCurrent(turnContext);
    if (current) {
      setChatStatusForSession(turnContext.sessionId, result.decision === "approve" ? t("chat.status.approved") : t("chat.status.denied"));
    }
    return current ? result : DENIED_ONCE_APPROVAL;
  }
  async function executeSystemAppleAppTool(toolName: string, argumentsValue: unknown, turnContext: ChatTurnContext) {
    const approvalRequest = await invoke<McpToolApprovalRequest | null>("prepare_system_apple_app_tool_approval", {
      arguments: argumentsValue,
      toolName,
    });
    let approval: { approvalToken: string } | undefined;
    if (approvalRequest) {
      const approvalResult = await requestMcpShieldApproval(approvalRequest, turnContext, argumentsValue);
      if (approvalResult.decision !== "approve") {
        await invoke<void>("mcp_reject_tool_approval", {
          approvalToken: approvalRequest.approvalToken,
        }).catch(() => undefined);
        throw new Error(`Apple app tool "${toolName}" was not approved.`);
      }
      approval = { approvalToken: approvalRequest.approvalToken };
    }
    if (!turnIsCurrent(turnContext)) {
      throw new Error("The originating chat turn is no longer active.");
    }
    const executeArgs: Record<string, unknown> = {
      arguments: argumentsValue,
      toolName,
      turnContext: mcpTurnContextRequest(turnContext),
    };
    if (approval) {
      executeArgs.approval = approval;
    }
    const result = await invoke<McpToolCallResult>("execute_system_apple_app_tool", executeArgs);
    if (!turnIsCurrent(turnContext)) {
      throw new Error("The originating chat turn is no longer active.");
    }
    return result;
  }
  async function handleSearchContinuationRequest(request: ParsedSearchContinuationRequest, context: SearchContinuationTurnContext<ConversationalMcpToolCapability>) {
    await runSearchContinuationRequest(request, context, {
      isCurrent: turnIsCurrent,
      runSearch: buildLocalSearchOutcome,
      setStatus: updateTurnStatus,
      setFailure: (turn, messageId, content) => updateTurnMessages(turn, (current) => current.map((entry) => (entry.id === messageId ? { ...entry, role: "assistant", content, isPending: false } : entry))),
      replacePending: (sessionId, pending) => {
        discardPendingSteer(sessionId);
        pendingSteersRef.current.set(sessionId, pending);
      },
      releaseAttachments: releaseAttachmentPayloads,
      searchingStatus: t("chat.status.searching_web"),
      readyStatus: t("chat.status.local_search_ready"),
      incompleteMessage: t("chat.search_errors.search_incomplete"),
      failureMessage: (code) => localSearchFailureMessage(code, t),
    });
  }
  async function handleConversationalMcpToolRequest(request: ParsedConversationalMcpToolRequest, context: ConversationalMcpTurnContext): Promise<void> {
    const appendTurnSystemMessage = (content: string) => {
      if (!turnIsCurrent(context.turnContext)) {
        return;
      }
      setMessagesForSession(context.sessionId, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content,
        },
      ]);
    };
    const { call } = request;
    const isCalendarRead = call.serverName.trim().toLowerCase() === "macos_applescript" && call.toolName.trim().toLowerCase() === "read_system_calendar";
    const queueToolContinuation = async ({
      resultText,
      attachments,
      message,
      capabilities,
      nativeExecutionReceiptId,
      outstandingNativeEffect,
      verifiedNativeExecutionReceipt,
      announceResult,
      terminalAfterResponse = false,
    }: {
      resultText: string;
      attachments?: ChatAttachment[];
      message: string;
      capabilities: ConversationalMcpToolCapability[];
      nativeExecutionReceiptId: string | null;
      outstandingNativeEffect: NativeEffectExpectation | null;
      verifiedNativeExecutionReceipt: boolean;
      announceResult: boolean;
      terminalAfterResponse?: boolean;
    }) => {
      if (!turnIsCurrent(context.turnContext)) return;
      const continuationContext = deriveChatTurnContext(context.turnContext, "steer", {
        turnId: createChatTurnIdentity("turn"),
        generationToken: createChatTurnIdentity("generation"),
        attachmentGrants: [],
      });
      const continuationAttachments = await attachPrivateDataProvenance(attachments ?? [mcpContinuationAttachment(call, resultText)], continuationContext.turnId);
      const verifiedContinuationContext = rebindChatTurnAttachments(continuationContext, continuationAttachments);
      if (announceResult) appendTurnSystemMessage(t("chat.status.tool_result_ready"));
      discardPendingSteer(context.sessionId);
      pendingSteersRef.current.set(context.sessionId, {
        turnContext: verifiedContinuationContext,
        sessionId: context.sessionId,
        agentId: context.agentId,
        userMessageId: null,
        message,
        attachments: continuationAttachments,
        providerId: context.providerId,
        modelId: context.modelId,
        reasoning: context.reasoning,
        context: context.context,
        contextBudget: context.contextBudget,
        primaryRouteId: context.primaryRouteId,
        fallbackRouteId: context.fallbackRouteId,
        automatedWebGroundingEnabled: context.automatedWebGroundingEnabled,
        assistantMessageId: null,
        mcpToolCapabilities: capabilities,
        toolLoopDepth: context.toolLoopDepth + 1,
        executableActionExpected: false,
        outstandingNativeEffect,
        verifiedNativeExecutionReceipt,
        nativeExecutionReceiptId,
        terminalAfterResponse,
      });
      setChatStatusForSession(context.sessionId, t("chat.status.tool_result_ready"));
    };
    if (!turnIsCurrent(context.turnContext)) {
      return;
    }
    if (!conversationalMcpToolIsAvailable(call, context.capabilities)) {
      appendTurnSystemMessage(`Blocked local tool request: ${call.serverName}/${call.toolName} was not available to this turn.`);
      setChatStatusForSession(context.sessionId, t("chat.status.tool_unavailable"));
      return;
    }
    if (!mcp) {
      appendTurnSystemMessage("Blocked local tool request: MCP runtime is unavailable.");
      setChatStatusForSession(context.sessionId, t("chat.status.tool_unavailable"));
      return;
    }
    if (context.toolLoopDepth >= maxConversationalMcpToolLoopDepth) {
      appendTurnSystemMessage("Blocked local tool request: tool loop depth limit reached.");
      setChatStatusForSession(context.sessionId, t("chat.status.tool_loop_stopped"));
      return;
    }
    try {
      const toolKey = `${call.serverName.trim().toLowerCase()}/${call.toolName.trim().toLowerCase()}`;
      const mutationExecutionExpected = conversationalMcpToolIsMutation(call.serverName, call.toolName);
      const callEffectExpectation = bindNativeEffectExpectationToTool(context.outstandingNativeEffect, toolKey, mutationExecutionExpected);
      setChatStatusForSession(context.sessionId, t("chat.status.running_tool", { tool: call.toolName }));
      const result =
        call.serverName.trim().toLowerCase() === "macos_applescript"
          ? await executeSystemAppleAppTool(call.toolName, call.argumentsValue, context.turnContext)
          : await mcp.executeTool(call.serverName, call.toolName, call.argumentsValue, {
              requestApproval: (approval) => requestMcpShieldApproval(approval, context.turnContext),
              isExecutionContextCurrent: () => turnIsCurrent(context.turnContext),
              turnContext: mcpTurnContextRequest(context.turnContext),
            });
      const nativeReceipt = nativeMcpExecutionReceipt(result);
      if (isSovereignMcpSearchCall(call)) {
        const search = verifiedSovereignMcpSearchResult(result);
        const requestedQuery = sovereignMcpSearchQuery(call);
        const attachment = search && requestedQuery === search.query ? localSearchAttachment(search) : null;
        if (!attachment) {
          const failure = localSearchFailureMessage("search_unavailable", t);
          appendTurnSystemMessage(failure);
          setChatStatusForSession(context.sessionId, failure);
          return;
        }
        await queueToolContinuation({
          resultText: "",
          attachments: [attachment],
          message: "Use the newly verified public evidence to finish the user's original request. Answer directly and cite only exact URLs supplied by the verified evidence.",
          capabilities: [],
          nativeExecutionReceiptId: null,
          outstandingNativeEffect: context.outstandingNativeEffect,
          verifiedNativeExecutionReceipt: false,
          announceResult: true,
          terminalAfterResponse: true,
        });
        return;
      }
      const permissionFailure = nativeMcpPermissionFailure(nativeReceipt);
      if (permissionFailure) {
        const pendingRecovery = requestDirectApplePermissionRecovery(context.turnContext, permissionFailure.capabilityId, { code: permissionFailure.code });
        if (pendingRecovery) {
          const choice = await pendingRecovery;
          if (choice === "retry") {
            await handleConversationalMcpToolRequest(request, context);
          } else if (turnIsCurrent(context.turnContext)) {
            setChatStatusForSession(context.sessionId, t("tasks.error_cancelled"));
          }
          return;
        }
      }
      if (nativeReceipt && nativeReceipt.outcome !== "succeeded") {
        await queueToolContinuation({
          resultText: mcpTerminalOutcomeText(call, nativeReceipt.outcome, nativeReceipt.nativeResultCode ?? "The native broker could not verify completion."),
          message: mcpTerminalOutcomeMessage(call),
          capabilities: [],
          nativeExecutionReceiptId: nativeReceipt.receiptId,
          outstandingNativeEffect: null,
          verifiedNativeExecutionReceipt: false,
          announceResult: false,
        });
        return;
      }
      const resultText = mcpToolResultText(result, verifiedExecutionCopy);
      if (!turnIsCurrent(context.turnContext)) {
        return;
      }
      if (call.serverName.trim().toLowerCase() === "macos_applescript" && call.toolName.trim().toLowerCase() === "read_apple_app_ui" && isUiSnapshotBlocked(resultText)) {
        const appLabel = isPlainRecord(call.argumentsValue) ? (firstString(call.argumentsValue.app_name, call.argumentsValue.appName) ?? "the requested app") : "the requested app";
        setBypassNoticeForSession(context.sessionId, accessibilityBlockedNotice(t, appLabel));
        setChatStatusForSession(context.sessionId, t("chat.status.app_blocked", { app: appLabel }));
        await queueToolContinuation({
          resultText: mcpTerminalOutcomeText(call, "unavailable", resultText),
          message: mcpTerminalOutcomeMessage(call),
          capabilities: [],
          nativeExecutionReceiptId: nativeReceipt?.receiptId ?? null,
          outstandingNativeEffect: null,
          verifiedNativeExecutionReceipt: false,
          announceResult: false,
        });
        return;
      }
      const outstandingNativeEffect = outstandingNativeEffectAfterReceipt(callEffectExpectation, {
        kind: "native_tool",
        effect: mutationExecutionExpected ? "mutation" : "read",
        toolKey,
        verified: nativeReceipt?.verified === true,
      });
      await queueToolContinuation({
        resultText,
        message: mcpContinuationMessage(call),
        capabilities: context.capabilities,
        nativeExecutionReceiptId: nativeReceipt?.receiptId ?? null,
        outstandingNativeEffect,
        verifiedNativeExecutionReceipt: nativeReceipt?.verified === true,
        announceResult: true,
      });
    } catch (error) {
      if (isSovereignMcpSearchCall(call)) {
        const failure = localSearchFailureMessage("search_unavailable", t);
        appendTurnSystemMessage(failure);
        if (turnIsCurrent(context.turnContext)) {
          setChatStatusForSession(context.sessionId, failure);
        }
        return;
      }
      const detail = isCalendarRead ? calendarToolFailureMessage(error, t) : `Local tool request blocked. ${toolErrorMessage(error)}`;
      try {
        await queueToolContinuation({
          resultText: mcpTerminalOutcomeText(call, localToolFailureCode(error), detail),
          message: mcpTerminalOutcomeMessage(call),
          capabilities: [],
          nativeExecutionReceiptId: null,
          outstandingNativeEffect: null,
          verifiedNativeExecutionReceipt: false,
          announceResult: false,
        });
      } catch {
        appendTurnSystemMessage(detail);
        if (turnIsCurrent(context.turnContext)) {
          setChatStatusForSession(context.sessionId, t("chat.status.tool_blocked"));
        }
      }
    }
  }
  useEffect(() => {
    onSessionsChangeRef.current = onSessionsChange;
    refreshSessionMessagesRef.current = refreshSessionMessages;
  });
  useEffect(() => {
    const cleanupTimeouts = executionCleanupTimeoutsRef.current;
    return () => {
      for (const timeout of cleanupTimeouts.values()) {
        clearTimeout(timeout);
      }
      cleanupTimeouts.clear();
    };
  }, []);
  useEffect(() => {
    activeSessionRef.current = activeSession;
  }, [activeSession]);
  useGatewayAutoTurn({
    translate: t,
    setExecuting: setIsExecutingPlanForSession,
    setProcessing: setIsProcessingForSession,
    setSending: setIsSendingForSession,
    setStatus: setChatStatusForSession,
    refreshSessionMessages,
    onSessionsChange,
  });
  useEffect(() => {
    routeRef.current = route;
  }, [route]);
  useEffect(() => {
    if (!activeSessionId) {
      setActiveExecution(null);
      return;
    }
    setActiveExecution((current) => {
      if (current?.sessionId === activeSessionId) {
        return current;
      }
      return readStoredActiveExecution(activeSessionId);
    });
  }, [activeSessionId, setActiveExecution]);
  const activeExecutionId = activeExecution?.executionId ?? "";
  const activeExecutionSessionId = activeExecution?.sessionId ?? "";
  const activeExecutionStatus = activeExecution?.status ?? null;
  const activeExecutionStreamStartAfterLogId = activeExecution?.streamStartAfterLogId ?? 0;
  const {
    effectiveActionKeys: effectiveRecoveryActionKeys,
    receiptAuthorities: recoveryReceiptAuthorities,
    refresh: refreshRecoveryExecutionStates,
    refreshForTerminalBatch: refreshRecoveryExecutionStatesForTerminalBatch,
    snapshot: recoveryExecutionStateSnapshot,
  } = useRecoveryReceiptProjection({
    activeExecution,
    activeSessionId,
    completedRecoveryActionKeys,
    messages,
  });
  useMacPermissionExecutionResume({
    activeExecution,
    activeSessionId,
    messages,
    onResumed: (executionId) => {
      setActiveExecutionForSession(activeSessionId, (current) => (current?.executionId === executionId ? { ...current, status: "running" } : current));
      setIsExecutingPlanForSession(activeSessionId, true);
      setIsProcessingForSession(activeSessionId, true);
      setChatStatusForSession(activeSessionId, tRef.current("chat.status.executing_plan"));
    },
  });
  useEffect(() => {
    if (!activeExecutionId || activeExecutionStatus !== "running" || activeExecutionSessionId !== activeSessionId || !isTauriRuntime) {
      return;
    }
    const runningExecution = {
      executionId: activeExecutionId,
      sessionId: activeExecutionSessionId,
      lastSeenId: activeExecutionStreamStartAfterLogId,
    };
    const subscription = {
      executionId: runningExecution.executionId,
      cancelled: false,
    };
    executionSubscriptionRef.current = subscription;
    async function subscribeToExecutionSteps() {
      try {
        const { Channel } = await import("@tauri-apps/api/core");
        if (subscription.cancelled) {
          return;
        }
        const channel = new Channel<AgentExecutionLogBatch>((batch) => {
          if (subscription.cancelled || batch.executionId !== subscription.executionId) {
            return;
          }
          const terminalStatus = batch.terminal ? terminalExecutionStatusFromLogs(batch.logs) : "running";
          setActiveExecution((current) => {
            if (!current || current.executionId !== batch.executionId) {
              return current;
            }
            const logs = mergeExecutionLogs(current.logs, batch.logs);
            const lastSeenId = logs.reduce((max, log) => Math.max(max, log.id), current.lastSeenId);
            const status = batch.terminal ? statusFromExecutionLogs(logs, terminalStatus) : current.status;
            const next = {
              ...current,
              logs,
              lastSeenId,
              status,
            };
            if (status === "running") {
              persistActiveExecution(next);
            } else {
              clearStoredActiveExecution(next.sessionId);
            }
            return next;
          });
          if (batch.terminal) {
            const sessionId = runningExecution.sessionId;
            const executionId = batch.executionId;
            refreshRecoveryExecutionStatesForTerminalBatch(sessionId, executionId, terminalStatus);
            setIsExecutingPlanForSession(sessionId, false);
            setIsProcessingForSession(sessionId, false);
            setChatStatusForSession(sessionId, tRef.current(terminalStatus === "completed" ? "chat.execution.status.complete" : terminalStatus === "halted" ? "chat.execution.status.halted" : "chat.execution.status.failed"));
            void refreshSessionMessagesRef.current(sessionId).catch(() => undefined);
            void invoke<ChatSession[]>("list_chat_sessions")
              .then((nextSessions) => onSessionsChangeRef.current(nextSessions))
              .catch(() => undefined);
            const existingCleanupTimeout = executionCleanupTimeoutsRef.current.get(sessionId);
            if (existingCleanupTimeout) {
              clearTimeout(existingCleanupTimeout);
            }
            const cleanupTimeout = setTimeout(() => {
              setActiveExecution((current) => {
                if (current?.executionId === executionId && current.sessionId === sessionId && current.status !== "running") {
                  return null;
                }
                return current;
              });
              executionCleanupTimeoutsRef.current.delete(sessionId);
            }, 1200);
            executionCleanupTimeoutsRef.current.set(sessionId, cleanupTimeout);
          }
        });
        await invoke<void>("stream_execution_steps", {
          executionId: runningExecution.executionId,
          execution_id: runningExecution.executionId,
          lastSeenId: runningExecution.lastSeenId,
          last_seen_id: runningExecution.lastSeenId,
          channel,
        });
      } catch {
        if (!subscription.cancelled) {
          setChatStatus(tRef.current("chat.status.execution_stream_unavailable"));
        }
      }
    }
    void subscribeToExecutionSteps();
    return () => {
      subscription.cancelled = true;
      if (executionSubscriptionRef.current === subscription) {
        executionSubscriptionRef.current = null;
      }
    };
  }, [activeExecutionId, activeExecutionSessionId, activeExecutionStatus, activeExecutionStreamStartAfterLogId, activeSessionId, refreshRecoveryExecutionStatesForTerminalBatch, setActiveExecution, setChatStatus, setChatStatusForSession, setIsExecutingPlanForSession, setIsProcessingForSession]);
  function compactSessionHistory(sessionId: string, agentId: string) {
    const compactSessionId = sessionId.trim();
    const compactAgentId = agentId.trim();
    if (!compactSessionId || !compactAgentId) {
      return;
    }
    void invoke<CompactSessionHistoryResponse>("compact_session_history", {
      request: {
        session_id: compactSessionId,
        agent_id: compactAgentId,
        max_turns: 32,
      },
    }).catch(() => undefined);
  }
  function lockSessionHydration(sessionId: string) {
    const token = nextHydrationLockTokenRef.current++;
    sessionHydrationLocksRef.current.set(sessionId, token);
    sessionHydrationVersionsRef.current.set(sessionId, token);
    return token;
  }
  function unlockSessionHydration(sessionId: string, token: number) {
    if (sessionHydrationLocksRef.current.get(sessionId) === token) {
      sessionHydrationLocksRef.current.delete(sessionId);
    }
  }
  useEffect(() => {
    let cancelled = false;
    async function loadSessionMessages() {
      if (!activeSessionId) {
        setMessages([]);
        return;
      }
      if (sessionHydrationLocksRef.current.has(activeSessionId)) {
        return;
      }
      const hydrationVersion = sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0;
      try {
        const stored = await invoke<StoredChatMessage[]>("list_chat_messages", {
          sessionId: activeSessionId,
          session_id: activeSessionId,
        });
        if (cancelled || sessionHydrationLocksRef.current.has(activeSessionId) || (sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0) !== hydrationVersion) {
          return;
        }
        replaceTranscript(storedMessagesToTranscript(stored), activeSessionId);
      } catch {
        if (!cancelled && !sessionHydrationLocksRef.current.has(activeSessionId) && (sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0) === hydrationVersion) {
          setMessagesForSession(activeSessionId, [
            {
              id: 1,
              role: "system",
              content: "Unable to load this chat session.",
            },
          ]);
          nextMessageIdRef.current = Math.max(nextMessageIdRef.current, 2);
        }
      }
    }
    void loadSessionMessages();
    return () => {
      cancelled = true;
    };
  }, [activeSessionId, replaceTranscript, setMessages, setMessagesForSession, storedMessagesToTranscript]);
  useEffect(() => {
    let cancelled = false;

    async function loadQueuedMessages() {
      if (!activeSessionId) {
        setQueuedMessages([]);
        return;
      }
      const hydrationVersion = sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0;
      try {
        const queued = await invoke<QueuedMessageRecord[]>("get_queued_messages", {
          sessionId: activeSessionId,
          session_id: activeSessionId,
        });
        if (!cancelled && !sessionHydrationLocksRef.current.has(activeSessionId) && (sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0) === hydrationVersion) {
          setQueuedMessagesForSession(activeSessionId, queued);
        }
      } catch {
        if (!cancelled && !sessionHydrationLocksRef.current.has(activeSessionId) && (sessionHydrationVersionsRef.current.get(activeSessionId) ?? 0) === hydrationVersion) {
          setQueuedMessagesForSession(activeSessionId, []);
        }
      }
    }

    void loadQueuedMessages();

    return () => {
      cancelled = true;
    };
  }, [activeSessionId, setQueuedMessages, setQueuedMessagesForSession]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    async function registerBypassTelemetryListener() {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        if (cancelled) {
          return;
        }
        unlisten = await listen<OomuBypassEvent>("oomu-bypass-telemetry", (event) => {
          const payload = event.payload;
          const activeTurn = activeTurnsRef.current.get(payload.sessionId);
          if (!activeTurn || activeTurn.turnId !== payload.turnId || activeTurn.generationToken !== payload.generationToken) {
            return;
          }
          setBypassNoticeForSession(payload.sessionId, oomuBypassNotice(payload));
          setChatStatusForSession(payload.sessionId, payload.kind === "timeout" ? tRef.current("chat.status.preflight_timeout") : tRef.current("chat.status.preflight_bypassed"));
        });
      } catch {
        void 0;
      }
    }

    void registerBypassTelemetryListener();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [setBypassNoticeForSession, setChatStatusForSession]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      const transcript = transcriptScrollRef.current;
      if (transcript) {
        transcript.scrollTop = transcript.scrollHeight;
      }
    });

    return () => window.cancelAnimationFrame(frame);
  }, [activeSessionId, messages, isSending, pendingPlan, activeExecution?.logs.length, activeExecution?.status]);

  const persistSessionConfig = useCallback(
    (sessionId: string, nextRoute: RouteOverride) => {
      const cleanedSessionId = sessionId.trim();
      if (!cleanedSessionId || !legacySessionConfigWriteAllowed(sessions.find((session) => session.id === cleanedSessionId))) {
        return Promise.resolve(false);
      }
      const contextBudget = contextBudgetForRoute(nextRoute, configuredProviders, systemHardwareProfile);
      const cleanedReasoning = resolveReasoningFallback(nextRoute.reasoning || defaultReasoningForProviderRoute(configuredProviders, nextRoute.providerId), supportedReasoningLevelsForRoute(configuredProviders, nextRoute.providerId, nextRoute.modelId));
      const persistedRoute = normalizeRouteReasoning(
        {
          ...nextRoute,
          reasoning: cleanedReasoning,
          context: String(contextBudget),
        },
        configuredProviders,
      );
      latestSessionConfigRouteRef.current.set(cleanedSessionId, persistedRoute);
      pendingConfigPersistCountsRef.current.set(cleanedSessionId, (pendingConfigPersistCountsRef.current.get(cleanedSessionId) ?? 0) + 1);
      const prior = sessionConfigPersistPromisesRef.current.get(cleanedSessionId) ?? Promise.resolve(true);
      const pending = prior
        .then(() =>
          invoke<void>("save_session_config", {
            sessionId: cleanedSessionId,
            session_id: cleanedSessionId,
            reasoningDepth: cleanedReasoning,
            reasoning_depth: cleanedReasoning,
            contextBudget,
            context_budget: contextBudget,
            providerId: nextRoute.providerId,
            provider_id: nextRoute.providerId,
            modelId: nextRoute.modelId,
            model_id: nextRoute.modelId,
          }),
        )
        .then(() => true)
        .catch(() => false)
        .finally(() => {
          const remaining = (pendingConfigPersistCountsRef.current.get(cleanedSessionId) ?? 1) - 1;
          if (remaining > 0) {
            pendingConfigPersistCountsRef.current.set(cleanedSessionId, remaining);
          } else {
            pendingConfigPersistCountsRef.current.delete(cleanedSessionId);
          }
        });
      sessionConfigPersistPromisesRef.current.set(cleanedSessionId, pending);
      return pending;
    },
    [configuredProviders, sessions, systemHardwareProfile],
  );

  useEffect(() => {
    if (!activeSessionId || !activeAgentId) {
      return;
    }

    let cancelled = false;
    const token = sessionConfigHydrationTokenRef.current + 1;
    sessionConfigHydrationTokenRef.current = token;

    async function hydrateSessionConfig() {
      if ((pendingConfigPersistCountsRef.current.get(activeSessionId) ?? 0) > 0) {
        return;
      }

      let config: SessionConfigRecord | null = null;
      try {
        config = await invoke<SessionConfigRecord | null>("get_session_config", {
          sessionId: activeSessionId,
          session_id: activeSessionId,
        });
      } catch {
        config = null;
      }
      if (cancelled || sessionConfigHydrationTokenRef.current !== token) {
        return;
      }

      if ((pendingConfigPersistCountsRef.current.get(activeSessionId) ?? 0) > 0) {
        return;
      }

      const baseRoute = defaultRouteForAgent(selectedAgent, configuredProviders, primaryRoute, verifiedStartupModelId);
      const routeIdentity = authoritativeSessionConfigRouteIdentity(config);
      if (!routeIdentity) {
        return;
      }
      const providerId = routeIdentity.providerConfigId;
      if (typedProviderClassIdForRoute(configuredProviders, providerId) !== routeIdentity.providerType) {
        return;
      }
      const providerModels = modelsForProvider(configuredProviders, providerId);
      const configuredModel = providerModels.find((model) => model.modelId === routeIdentity.modelId);
      if (!configuredModel) {
        return;
      }
      const modelId = routeIdentity.modelId;
      const supportedLevels = configuredModel.supportedReasoningLevels;
      const contextBudgetBounds = contextBudgetBoundsForProvider(configuredProviders, providerId, systemHardwareProfile);
      const nextRoute = normalizeRouteReasoning(
        {
          providerId,
          providerType: routeIdentity.providerType,
          modelId,
          reasoning: resolveReasoningFallback(sessionConfigReasoning(config) ?? baseRoute.reasoning, supportedLevels),
          context: normalizeContextBudget(sessionConfigContextBudget(config) ?? baseRoute.context, contextBudgetBounds),
        },
        configuredProviders,
      );
      const latestLocalRoute = latestSessionConfigRouteRef.current.get(activeSessionId);
      if (latestLocalRoute && !sessionRoutesMatch(nextRoute, latestLocalRoute, configuredProviders, systemHardwareProfile)) {
        return;
      }
      if (latestLocalRoute) {
        latestSessionConfigRouteRef.current.delete(activeSessionId);
      }

      routeRef.current = nextRoute;
      setRouteOverrides((current) => ({
        ...current,
        [activeSessionId]: nextRoute,
      }));
    }

    void hydrateSessionConfig();

    return () => {
      cancelled = true;
    };
  }, [activeAgentId, activeSessionId, configuredProviders, primaryRoute, selectedAgent, routeRefreshNonce, systemHardwareProfile, verifiedStartupModelId]);

  function updateRoute(nextRoute: Partial<RouteOverride>) {
    if (!activeAgentId) return;

    const draftRoute = {
      ...routeRef.current,
      ...nextRoute,
    };
    const contextBudgetBounds = contextBudgetBoundsForProvider(configuredProviders, draftRoute.providerId, systemHardwareProfile);
    const mergedRoute = normalizeRouteReasoning(
      {
        ...draftRoute,
        context: nextRoute.context !== undefined ? normalizeContextBudget(nextRoute.context, contextBudgetBounds) : normalizeContextBudget(draftRoute.context, contextBudgetBounds),
      },
      configuredProviders,
    );
    routeRef.current = mergedRoute;
    setRouteOverrides((current) => {
      return {
        ...current,
        [activeSessionId || `agent:${activeAgentId}`]: mergedRoute,
      };
    });
    if (activeSessionId) {
      if (dynamicRoutingEnabled) {
        void handleDynamicRoutingToggle(true);
      } else if (legacySessionConfigWriteAllowed(activeSession)) {
        void persistSessionConfig(activeSessionId, mergedRoute);
      }
    }
  }

  function handleProviderChange(rawProviderId: string) {
    const providerId = providerConfigurationId(rawProviderId);
    const providerModels = modelsForProvider(configuredProviders, providerId);
    const nextBounds = contextBudgetBoundsForProvider(configuredProviders, providerId, systemHardwareProfile);
    const contextChangedClass = activeContextBudgetBounds.target !== nextBounds.target;
    updateRoute({
      providerId,
      providerType: typedProviderClassIdForRoute(configuredProviders, providerId),
      modelId: providerModels[0]?.modelId ?? "",
      reasoning: defaultReasoningForProviderRoute(configuredProviders, providerId),
      context: contextChangedClass ? String(nextBounds.defaultValue) : normalizeContextBudget(activeContextBudgetText, nextBounds),
    });
  }

  function handleModelChange(modelId: string) {
    updateRoute({
      modelId,
      context: normalizeContextBudget(activeContextBudgetText, activeContextBudgetBounds),
    });
  }

  const handleAgentChange = useCallback(
    async (agentId: string) => {
      setSelectedAgentId(agentId);
      const agent = agents.find((entry) => entry.id === agentId);
      const nextRoute = defaultRouteForAgent(agent, configuredProviders, primaryRoute, verifiedStartupModelId);
      const session = await createSessionInContext(agentId, routeBindingForDynamicRouting(dynamicRoutingDefaultForAgent(agent), nextRoute));
      if (session) {
        setRouteOverrides((current) => ({
          ...current,
          [session.id]: nextRoute,
        }));
        if (!sessionUsesDynamicBinding(session)) {
          void persistSessionConfig(session.id, {
            ...nextRoute,
            context: normalizeContextBudget(nextRoute.context, contextBudgetBoundsForProvider(configuredProviders, nextRoute.providerId, systemHardwareProfile)),
          });
        }
        onSelectSession(session.id);
      }
    },
    [agents, configuredProviders, createSessionInContext, onSelectSession, persistSessionConfig, primaryRoute, systemHardwareProfile, verifiedStartupModelId],
  );

  useEffect(() => {
    if (typeof window === "undefined" || !activeAgentId) {
      return;
    }
    window.localStorage.setItem(ACTIVE_AGENT_STORAGE_KEY, activeAgentId);
    window.dispatchEvent(
      new CustomEvent<AgentSelectionEventDetail>(ACTIVE_AGENT_CHANGED_EVENT, {
        detail: { agentId: activeAgentId },
      }),
    );
  }, [activeAgentId]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const applySidebarSelection = (agentId: string) => {
      const nextAgentId = agentId.trim();
      if (!nextAgentId || !agents.some((agent) => agent.id === nextAgentId)) {
        return;
      }
      if (nextAgentId === activeAgentId && activeSession?.agentId === nextAgentId) {
        window.localStorage.removeItem(PENDING_SIDEBAR_AGENT_STORAGE_KEY);
        return;
      }
      if (pendingSidebarAgentRef.current === nextAgentId) {
        return;
      }

      pendingSidebarAgentRef.current = nextAgentId;
      void handleAgentChange(nextAgentId).finally(() => {
        if (window.localStorage.getItem(PENDING_SIDEBAR_AGENT_STORAGE_KEY) === nextAgentId) {
          window.localStorage.removeItem(PENDING_SIDEBAR_AGENT_STORAGE_KEY);
        }
        if (pendingSidebarAgentRef.current === nextAgentId) {
          pendingSidebarAgentRef.current = null;
        }
      });
    };

    const handleSidebarSelect = (event: Event) => {
      const agentId = (event as CustomEvent<AgentSelectionEventDetail>).detail?.agentId;
      if (typeof agentId === "string") {
        applySidebarSelection(agentId);
      }
    };

    window.addEventListener(SIDEBAR_AGENT_SELECT_EVENT, handleSidebarSelect);
    const pendingAgentId = window.localStorage.getItem(PENDING_SIDEBAR_AGENT_STORAGE_KEY);
    if (pendingAgentId) {
      applySidebarSelection(pendingAgentId);
    }

    return () => {
      window.removeEventListener(SIDEBAR_AGENT_SELECT_EVENT, handleSidebarSelect);
    };
  }, [activeAgentId, activeSession?.agentId, agents, handleAgentChange]);

  async function attachLocalContextSelection(loadSelection: (sessionId: string, turnId: string) => Promise<ChooseLocalContextResponse>) {
    const targetSessionScope = chatSessionStateScope(activeSessionId);
    attachmentReadAbortRef.current?.abort();
    const controller = new AbortController();
    attachmentReadAbortRef.current = controller;
    const grantTurnId = `attachment-${Date.now()}-${nextMessageIdRef.current++}`;
    setIsReadingAttachmentsForSession(targetSessionScope, true);
    try {
      const selection = await loadSelection(targetSessionScope, grantTurnId);
      const selectionFailures: AttachmentFailure[] = selection.results.flatMap((result) => {
        if (!result.ok || !result.grantId) {
          return [{ name: result.name, errorCode: result.errorCode }];
        }
        if (!composerAttachmentIsSupported(result.mimeType)) {
          return [{ name: result.name, errorCode: "attachment_format_unsupported" }];
        }
        return [];
      });
      const candidates = selection.results
        .filter((result) => result.ok && result.grantId && composerAttachmentIsSupported(result.mimeType))
        .map((result) => ({
          name: result.name,
          decodedByteCount: result.decodedByteCount,
          encodedByteCount: result.encodedByteCount,
          process: async (signal: AbortSignal) => {
            if (signal.aborted) throw new DOMException("Aborted", "AbortError");
            const context = await invoke<LocalContextResponse>("read_local_context", {
              request: {
                grantId: result.grantId,
                sessionId: targetSessionScope,
                turnId: grantTurnId,
              },
            });
            if (signal.aborted) throw new DOMException("Aborted", "AbortError");
            return localContextToAttachment(context);
          },
        }));
      const processed = await processAttachmentsBounded(candidates, {
        signal: controller.signal,
        usage: {
          count: attachments.length,
          decodedBytes: attachments.reduce((sum, attachment) => sum + attachment.byte_count, 0),
          encodedBytes: attachments.reduce((sum, attachment) => sum + (attachment.data_base64?.length ?? 0), 0),
        },
      });
      const nextAttachments = processed.flatMap((result) => (result.ok ? [result.value] : []));
      const processingFailures: AttachmentFailure[] = processed.flatMap((result) => (result.ok ? [] : [{ name: result.name, errorCode: result.errorCode }]));
      const failures = [...selectionFailures, ...processingFailures];
      const failedCount = failures.length;
      if (nextAttachments.length > 0) {
        setAttachmentsForSession(targetSessionScope, (current) => [...current, ...nextAttachments]);
        setChatStatusForSession(targetSessionScope, t("chat.status.attachment_ready"));
      } else if (failedCount > 0) {
        setChatStatusForSession(targetSessionScope, t("chat.status.attachment_blocked"));
      }
      if (failedCount > 0) {
        setMessagesForSession(targetSessionScope, (current) => [
          ...current,
          ...failures.map(
            (failure) =>
              ({
                id: nextMessageIdRef.current++,
                role: "system",
                content: attachmentFailureCopy(failure, t),
              }) as ChatTranscriptMessage,
          ),
        ]);
      }
    } catch {
      setMessagesForSession(targetSessionScope, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content: t("chat.attachment_failed"),
        },
      ]);
      setChatStatusForSession(targetSessionScope, t("chat.status.attachment_blocked"));
    } finally {
      await invoke("revoke_local_context_grants", {
        request: { sessionId: targetSessionScope, turnId: grantTurnId },
      }).catch(() => undefined);
      if (attachmentReadAbortRef.current === controller) {
        attachmentReadAbortRef.current = null;
        setIsReadingAttachmentsForSession(targetSessionScope, false);
      }
    }
  }

  async function handleAttachmentRequest() {
    await attachLocalContextSelection((sessionId, turnId) =>
      invoke<ChooseLocalContextResponse>("choose_local_context", {
        request: {
          sessionId,
          turnId,
          operation: "read",
        },
      }),
    );
  }

  async function handleDroppedAttachment(dropId: string) {
    await attachLocalContextSelection((sessionId, turnId) =>
      invoke<ChooseLocalContextResponse>("claim_dropped_local_context", {
        request: { dropId, sessionId, turnId },
      }),
    );
  }

  async function validateModCompatibilityForMessage(message: string, routeBinding: { providerId: string; modelId: string }, explicitModId: string | null) {
    if (!isTauriRuntime || !selectedAgent || !routeBinding.providerId || !routeBinding.modelId) {
      return;
    }
    await invoke("validate_mod_compatibility_for_turn", {
      request: {
        agent_id: selectedAgent.id,
        provider_id: routeBinding.providerId,
        model_id: routeBinding.modelId,
        message,
        locale: language,
        explicit_mod_id: explicitModId,
      },
    });
  }

  const prepareVisualAttachmentsForTurn = useCallback(
    async (turnAttachments: ChatAttachment[], targetSessionId: string) => {
      if (!turnAttachments.some(shouldAnalyzeVisualChatAttachment)) {
        return turnAttachments;
      }

      if (!isTauriRuntime) {
        return turnAttachments.map((attachment) =>
          shouldAnalyzeVisualChatAttachment(attachment)
            ? {
                ...attachment,
                text: [`Visual analysis for ${attachment.name}`, `MIME type: ${attachment.mime_type}`, "", "Analysis blocked:", "Local visual analysis requires the OOMU desktop app."].join("\n"),
              }
            : attachment,
        );
      }

      const targetSessionScope = chatSessionStateScope(targetSessionId);
      setChatStatusForSession(targetSessionScope, tRef.current("chat.status.analyzing_image"));
      attachmentReadAbortRef.current?.abort();
      const controller = new AbortController();
      attachmentReadAbortRef.current = controller;
      let analyzed;
      try {
        analyzed = await processAttachmentsBounded(
          turnAttachments.map((attachment) => ({
            name: attachment.name,
            decodedByteCount: attachment.byte_count,
            encodedByteCount: attachment.data_base64?.length ?? 0,
            release: () => releaseAttachmentPayloads([attachment]),
            process: async (signal: AbortSignal) => {
              if (signal.aborted) throw new DOMException("Aborted", "AbortError");
              if (!shouldAnalyzeVisualChatAttachment(attachment)) return attachment;
              try {
                const analysis = await invoke<VisualArtifactAnalysis>("analyze_visual_artifact", {
                  request: visualAnalysisRequestForAttachment(attachment),
                });
                if (signal.aborted) throw new DOMException("Aborted", "AbortError");
                const analyzedAttachment = {
                  ...attachment,
                  text: visualAnalysisTextForAttachment(attachment, analysis),
                };
                releaseAttachmentPayloads([attachment]);
                return analyzedAttachment;
              } catch (error) {
                if (signal.aborted) throw new DOMException("Aborted", "AbortError");
                const failedAttachment = {
                  ...attachment,
                  text: visualAnalysisErrorTextForAttachment(attachment, error),
                };
                releaseAttachmentPayloads([attachment]);
                return failedAttachment;
              }
            },
          })),
          { signal: controller.signal },
        );
      } finally {
        if (attachmentReadAbortRef.current === controller) {
          attachmentReadAbortRef.current = null;
        }
      }
      const analyzedAttachments = analyzed.flatMap((result) => (result.ok ? [result.value] : []));
      setChatStatusForSession(targetSessionScope, tRef.current("chat.status.visual_ready"));
      return analyzedAttachments;
    },
    [setChatStatusForSession],
  );

  async function buildLocalSearchOutcome(query: string, owner: Pick<ChatTurnContext, "sessionId" | "turnId" | "generationToken">, options: LocalSearchRequestOptions = {}) {
    return fetchLocalSearchForTurn(query, owner, options, {
      searchControlEnabled: automatedWebGroundingEnabled,
      messages,
      translate: t,
      setStatus: (sessionId, status) => setChatStatusForSession(chatSessionStateScope(sessionId), status),
      setDebug: (sessionId, debug) => setHeadlessSearchDebugForSession(chatSessionStateScope(sessionId), debug),
    });
  }

  async function handleSubmit(nextMessageValue: string, options: ChatSubmitOptions = {}) {
    const { submittedMessage, nextMessage, recoveryPlan, resume, turnFiles, replayOrRecovery, hasTurnContent, submitSession } = createChatSubmissionSeed(nextMessageValue, options, attachments, activeSessionId);
    const calendarFollowup = !recoveryPlan && turnFiles.length === 0 ? calendarRecoveryFollowupForTranscript(nextMessage, messages, completedRecoveryActionKeys) : null;
    const isRecovery = isRecoverySubmission(recoveryPlan, Boolean(calendarFollowup), Boolean(resume));
    const submitScope = pendingSubmissions.scope();
    if (
      chatSubmissionIsBlocked({
        activeSessionMismatch: recoverySessionMismatch(replayOrRecovery, submitSession, activeSessionId),
        hasTurnContent,
        hasSelectedAgent: Boolean(selectedAgent),
        isSending,
        isReadingAttachments,
        queueIsExecuting: executingQueueSessionsRef.current.has(submitSession),
        submissionIsPending: pendingSubmissions.has(submitScope),
      })
    ) {
      return;
    }
    const attachedWorkspaceResources = workspaceDataResourcesForAttachments(turnFiles);
    const { ambiguousLocalAppTriageFailure, directLocalCommand, directLocalReadRequest, directMailReadRequest, directCalendarReadRequest, directAppleAppReadRequest, directAppleAppWriteRequest, hasPrivateAppCandidate } = await resolveDirectTurnRequests({
      message: nextMessage,
      recoveryTurn: isRecovery,
      attachedWorkspaceResources,
    });
    const contextualMarkdownRouting = contextualArtifactTurnRouting(nextMessage, messages, t("chat.status.planning_steps", { name: selectedAgent.name }), !recoveryPlan, hasLikelyLocalNativeTaskIntent(nextMessage), canFallbackAfterPlannerRejection(recoveryPlan, Boolean(directLocalCommand || directLocalReadRequest), hasPrivateAppCandidate, hasLikelyLocalNativeTaskIntent(nextMessage), hasExplicitBrowserNavigationIntent(nextMessage)));
    const { route: contextualMarkdownRoute, likelyLocalNativeTaskIntent, plannerConversationFallbackAllowed } = contextualMarkdownRouting;

    if (
      shouldWaitForLocalModelHydration({
        isRecovery,
        localModelIsHydrating,
        hasDirectLocalCommand: Boolean(directLocalCommand),
        hasDirectLocalRead: Boolean(directLocalReadRequest),
        hasDirectMailRead: Boolean(directMailReadRequest),
        hasDirectCalendarRead: Boolean(directCalendarReadRequest),
        hasDirectAppleRead: Boolean(directAppleAppReadRequest),
        hasDirectAppleWrite: Boolean(directAppleAppWriteRequest),
        isSystemDiagnostics: isSystemDiagnosticsPrompt(nextMessage),
        likelyLocalNativeTask: likelyLocalNativeTaskIntent,
        hasAmbiguousLocalAppIntent: ambiguousLocalAppTriageFailure,
      })
    ) {
      setChatStatus(t("chat.status.model_hydrating"));
      return;
    }
    if (!pendingSubmissions.begin(submitScope)) {
      return;
    }
    setBypassNotice(null);

    const explicitSlashCommand = unlessRecovery(isRecovery, () => slashCommandForMessage(availableSlashCommands, nextMessage));
    const headlessModSearch = isRecovery ? null : headlessModSearchForMessage(installedMods, nextMessage);
    const turnRouteBinding = routeBindingForDynamicRouting(dynamicRoutingEnabled && !explicitSlashCommand, route);
    const autoRouteLocalModelForTurn = route.modelId;
    const autoRouteCloudModelForTurn = autoRouteCloudModelId;
    const detectedPathAttachments: ChatAttachment[] = [];
    let attachmentsForTurn = [...turnFiles, ...detectedPathAttachments];
    const browserNavigationFailure =
      activeBrowserRoute?.url && browserFeedbackIndicatesFailedNavigation(nextMessage)
        ? {
            url: activeBrowserRoute.url,
            sessionId: activeBrowserRoute.sessionId ?? activeSessionId,
            searchQuery: browserSearchFallbackQuery(nextMessage, messages, activeBrowserRoute),
          }
        : null;
    reportBrowserNavigationFailure(browserNavigationFailure, registerFailedBrowserNavigation, () => setChatStatus(t("chat.browser.navigation_blocked_status")));
    let localMailAssistantText: string | null = null;
    let localCalendarResultText = "";
    let sessionId = activeSessionId;
    let turnProjectId = resolveTurnProjectId(projectId, activeSession?.projectId);
    let sessionToSelect: string | null = null;
    let hydrationLockToken: number | null = null;
    let turnMessage = "";
    let turnModelMessage = nextMessage;
    let readAttachments: ChatAttachment[] = [];
    let turnAcknowledged = false;
    let acknowledgedUserMessageId: number | null = null;
    let acceptedDurableUserMessageId: number | null = null;
    let browserPromptRouteEvaluated = false;
    let turnContext: ChatTurnContext | null = null;
    let turnMcpTools = conversationalMcpCapabilities;
    let projectDocumentTurn: ProjectChatDocumentRequest | null = null;
    let responseRequiresNativeExecutionReceipt = false;
    let outstandingNativeEffect: NativeEffectExpectation | null = null;
    let browserDirectiveGrantsForResponse = browserDirectiveGrantsForMessage(installedMods, nextMessage, activeBrowserRoute);
    const releaseTurnAttachments = () => releaseAttachmentPayloads(attachmentsForTurn);
    async function ensureTurnSession() {
      if (sessionId) {
        return true;
      }
      let session: ChatSession | null = null;
      try {
        session = await createSessionInContext(selectedAgent.id, turnRouteBinding);
      } catch {
        setMessages((current) => [
          ...current,
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: "Create or select a chat session before sending.",
          },
        ]);
        pendingSubmissions.end(submitScope);
        return false;
      }
      if (!session) {
        setMessages((current) => [
          ...current,
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: "Create or select a chat session before sending.",
          },
        ]);
        pendingSubmissions.end(submitScope);
        return false;
      }
      sessionId = session.id;
      turnProjectId = resolveTurnProjectId(projectId, session.projectId);
      sessionToSelect = session.id;
      return true;
    }
    function prepareTurnContext(attachmentsForReceipt: ChatAttachment[]) {
      if (!sessionId || turnContext) {
        return;
      }
      turnContext = createReplayAwareTurnContext(resume, {
        turnId: createChatTurnIdentity("turn"),
        generationToken: createChatTurnIdentity("generation"),
        sessionId,
        agentId: selectedAgent.id,
        projectId: turnProjectId,
        route: {
          providerId: directLocalCommand ? route.providerId : turnRouteBinding.providerId,
          modelId: directLocalCommand ? route.modelId : turnRouteBinding.modelId,
          reasoning: activeReasoningLevel,
          contextBudget: activeContextBudget,
          primaryRouteId: activePrimaryRouteId,
          fallbackRouteId: activeFallbackRouteId,
          dynamicRoutingEnabled: dynamicRoutingEnabled && !explicitSlashCommand,
          automatedWebGroundingEnabled,
        },
        attachmentGrants: attachmentsForReceipt.map((attachment) => ({
          name: attachment.name,
          mimeType: attachment.mime_type,
          byteCount: attachment.byte_count,
        })),
      });
      registerActiveTurn(turnContext);
    }
    function abandonPreparedTurn() {
      if (turnContext) {
        clearActiveTurn(turnContext);
      }
      if (sessionId && hydrationLockToken !== null) {
        unlockSessionHydration(sessionId, hydrationLockToken);
        hydrationLockToken = null;
      }
    }
    async function persistAcceptedTerminalResult(result: { role: "assistant" | "system"; content: string; status: "completed" | "failed" | "cancelled" | "escalated" }) {
      if (!turnContext) return;
      await finalizeTurnWithCompletionAttention(turnContext, result, finalizeDurableChatTurn, publishBackgroundCompletionAttention, turnContext);
    }
    function userTurnMessage(attachmentsForReceipt: ChatAttachment[]) {
      return directLocalCommand ? submittedMessage || "Run the local command." : directLocalReadRequest ? submittedMessage || "Please review the approved file." : messageWithAttachmentReceipt(submittedMessage || "Please review the attached file.", attachmentsForReceipt);
    }
    function acknowledgeTurn(attachmentsForReceipt: ChatAttachment[], options: { deferBrowserPromptRoute?: boolean } = {}) {
      if (!sessionId || turnAcknowledged) {
        return;
      }
      prepareTurnContext(attachmentsForReceipt);
      if (!turnContext) {
        return;
      }
      persistLegacySessionConfigIfAllowed(persistSessionConfig, sessions, sessionId, Boolean(resume) || turnContext.route.dynamicRoutingEnabled, route, activeReasoningLevel, activeContextBudgetText);
      if (!isRecovery) {
        setComposerResetSignalForSession(sessionId, (value) => value + 1);
        setAttachmentsForSession(sessionId, []);
      }
      hydrationLockToken = lockSessionHydration(sessionId);
      if (sessionToSelect) {
        onSelectSession(sessionToSelect);
      }
      setIsSendingForSession(sessionId, true);
      setIsProcessingForSession(sessionId, true);
      turnMessage = userTurnMessage(attachmentsForReceipt);
      const userMessage: ChatTranscriptMessage = {
        id: acceptedDurableUserMessageId ?? nextMessageIdRef.current++,
        role: "user",
        content: turnMessage,
        isPending: true,
        metadata: {
          turnId: turnContext.turnId,
          rootTurnId: turnContext.ancestry.rootTurnId,
          generationToken: turnContext.generationToken,
          turnState: "accepted",
        },
      };
      nextMessageIdRef.current = Math.max(nextMessageIdRef.current, userMessage.id + 1);
      acknowledgedUserMessageId = userMessage.id;
      void unlessRecovery(Boolean(resume), () => setMessagesForSession(sessionId, (current) => upsertByNumericId(current, userMessage)));
      if (!submitSession) {
        clearSessionMessages(NEW_CHAT_SESSION_SCOPE);
        clearSessionSending(NEW_CHAT_SESSION_SCOPE);
        clearSessionProcessing(NEW_CHAT_SESSION_SCOPE);
        clearSessionStream(NEW_CHAT_SESSION_SCOPE);
        clearSessionComposerReset(NEW_CHAT_SESSION_SCOPE);
        clearSessionDraft(NEW_CHAT_SESSION_SCOPE);
        clearSessionStatus(NEW_CHAT_SESSION_SCOPE);
        clearSessionBypassNotice(NEW_CHAT_SESSION_SCOPE);
        clearSessionAttachments(NEW_CHAT_SESSION_SCOPE);
        clearSessionAttachmentRead(NEW_CHAT_SESSION_SCOPE);
      }
      turnAcknowledged = true;
      if (!options.deferBrowserPromptRoute) {
        activateBrowserPromptRoute(attachmentsForReceipt);
      }
    }
    function activateBrowserPromptRoute(attachmentsForReceipt: ChatAttachment[]) {
      if (!sessionId || acknowledgedUserMessageId === null || browserPromptRouteEvaluated) {
        return;
      }
      browserPromptRouteEvaluated = true;
      const browserPromptRoute =
        isRecovery || directLocalCommand || directLocalReadRequest || directMailReadRequest || directCalendarReadRequest || directAppleAppReadRequest || directAppleAppWriteRequest || ambiguousLocalAppTriageFailure || headlessModSearch
          ? null
          : browserSplitRouteFromUserPrompt(nextMessage || turnMessage, messages, acknowledgedUserMessageId, sessionId, {
              searchControlEnabled: automatedWebGroundingEnabled,
              sources: attachmentsForReceipt.length > 0 ? [{ kind: "unknown_derived" }] : [{ kind: "user_text" }],
            });
      if (browserPromptRoute) {
        responseRequiresNativeExecutionReceipt = true;
        browserDirectiveGrantsForResponse = mergeBrowserDirectiveGrants(browserDirectiveGrantsForResponse, [{ modId: BROWSER_SPLIT_MOD_ID }]);
        activateBrowserSplitRoute(browserPromptRoute);
      }
    }
    function refreshAcknowledgedTurnReceipt(attachmentsForReceipt: ChatAttachment[]) {
      if (!turnContext || acknowledgedUserMessageId === null) {
        return;
      }
      const enrichedTurnMessage = userTurnMessage(attachmentsForReceipt);
      if (enrichedTurnMessage === turnMessage) {
        return;
      }
      turnMessage = enrichedTurnMessage;
      const userMessageId = acknowledgedUserMessageId;
      updateTurnMessages(turnContext, (current) => current.map((entry) => (entry.id === userMessageId ? { ...entry, content: enrichedTurnMessage } : entry)));
    }
    if (!(await ensureTurnSession())) {
      releaseTurnAttachments();
      return;
    }
    prepareTurnContext(attachmentsForTurn);
    let preparedTurnContext = turnContext as ChatTurnContext | null;
    if (!preparedTurnContext) {
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }
    try {
      const accepted = await acceptDurableChatTurn(preparedTurnContext, userTurnMessage(attachmentsForTurn), resume?.turnState === "interrupted");
      acceptedDurableUserMessageId = accepted.messageId;
    } catch {
      abandonPreparedTurn();
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }
    void (!isRecovery && setComposerDraftForSession(preparedTurnContext.sessionId, ""));
    acknowledgeTurn(attachmentsForTurn, { deferBrowserPromptRoute: true });
    options.onAccepted?.();
    const acceptedTurnContext = preparedTurnContext;
    async function endAcceptedTurnWithFailure(content: string, status: string, assistantOutcome = false, terminalStatus: "completed" | "failed" | "cancelled" = assistantOutcome ? "completed" : "failed") {
      const role: "assistant" | "system" = assistantOutcome ? "assistant" : "system";
      await persistAcceptedTerminalResult({
        role,
        content,
        status: terminalStatus,
      });
      updateTurnMessages(acceptedTurnContext, (current) => [...current, { id: nextMessageIdRef.current++, role, content }]);
      updateTurnStatus(acceptedTurnContext, status);
      abandonPreparedTurn();
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      continueAfterTurn(acceptedTurnContext.sessionId);
    }
    if (projectDocumentRequestNeedsProjectScope(nextMessage, acceptedTurnContext.projectId, attachmentsForTurn.length)) { const guidance = t("chat.project_scope.required_for_files"); await endAcceptedTurnWithFailure(guidance, guidance, true); return; }
    if (calendarFollowup) {
      const outcome = await resolveCalendarRecoveryFollowup(calendarFollowup, handleResolveCalendarRecovery);
      const content = t(outcome.contentKey, outcome.contentVariables);
      await persistAcceptedTerminalResult({
        role: outcome.role,
        content,
        status: outcome.status,
      });
      updateTurnMessages(acceptedTurnContext, (current) => [...current, { id: nextMessageIdRef.current++, role: outcome.role, content }]);
      updateTurnStatus(acceptedTurnContext, t(outcome.statusKey));
      abandonPreparedTurn();
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      await refreshSessionMessages(acceptedTurnContext.sessionId).catch(() => false);
      return;
    }
    let localSearchOutcome: LocalSearchOutcome | null = null;
    let searchContinuationState = createSearchContinuationState(preparedTurnContext, nextMessage);
    try {
      const shouldSkipLocalSearch = !runWeb(isRecovery, resume) || directLocalCommand || directLocalReadRequest || directMailReadRequest || directCalendarReadRequest || directAppleAppReadRequest || directAppleAppWriteRequest || ambiguousLocalAppTriageFailure;
      if (!shouldSkipLocalSearch) {
        localSearchOutcome = await buildLocalSearchOutcome(
          nextMessage,
          preparedTurnContext,
          browserNavigationFailure
            ? {
                searchQuery: browserNavigationFailure.searchQuery,
                targetSessionId: sessionId,
                sources: attachmentsForTurn.length > 0 ? [{ kind: "unknown_derived" }] : [{ kind: "user_text" }],
              }
            : headlessModSearch
              ? {
                  activePageAvailable: Boolean(activeBrowserRoute),
                  searchQuery: headlessModSearch.query,
                  targetSessionId: sessionId,
                  sources: attachmentsForTurn.length > 0 ? [{ kind: "unknown_derived" }] : [{ kind: "user_text" }],
                  modId: headlessModSearch.modId,
                }
              : {
                  activePageAvailable: Boolean(activeBrowserRoute),
                  targetSessionId: sessionId,
                  sources: attachmentsForTurn.length > 0 ? [{ kind: "unknown_derived" }] : [{ kind: "user_text" }],
                },
        );
      }
    } catch (error) {
      const notice = chatFailureNotice(error, t);
      await endAcceptedTurnWithFailure(notice.content, notice.status);
      return;
    }
    if (!turnIsCurrent(preparedTurnContext)) {
      if (isTauriRuntime) {
        await invoke("cancel_sovereign_search", {
          request: {
            sessionId: preparedTurnContext.sessionId,
            originTurnId: preparedTurnContext.turnId,
            originGenerationToken: preparedTurnContext.generationToken,
          },
        }).catch(() => undefined);
      }
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      releaseSucceededLocalSearchOutcome(localSearchOutcome, releaseAttachmentPayloads);
      releaseTurnAttachments();
      return;
    }
    if (localSearchOutcome && "errorCode" in localSearchOutcome && localSearchOutcomeStopsInference(localSearchOutcome)) {
      const failureText = localSearchFailureMessage(localSearchOutcome.errorCode, t);
      await endAcceptedTurnWithFailure(failureText, failureText, true, localSearchTerminalStatus(localSearchOutcome.errorCode));
      return;
    }
    if (localSearchOutcome && "errorCode" in localSearchOutcome && !localSearchOutcome.explicit) {
      try {
        const checkpoint = await invoke<AcceptedTurnCheckpointReceipt>("record_accepted_chat_turn_checkpoint", {
          request: {
            sessionId: preparedTurnContext.sessionId,
            turnId: preparedTurnContext.turnId,
            generationToken: preparedTurnContext.generationToken,
            kind: "web_grounding_unavailable",
          },
        });
        if (checkpoint.sessionId !== preparedTurnContext.sessionId || checkpoint.turnId !== preparedTurnContext.turnId || checkpoint.generationToken !== preparedTurnContext.generationToken || checkpoint.kind !== "web_grounding_unavailable" || checkpoint.localizationKey !== "chat.search_errors.ambient_unavailable") {
          throw new Error("accepted_turn_checkpoint_mismatch");
        }
        nextMessageIdRef.current = Math.max(nextMessageIdRef.current, checkpoint.messageId + 1);
        updateTurnMessages(preparedTurnContext, (current) =>
          current.some((message) => message.id === checkpoint.messageId)
            ? current
            : [
                ...current,
                {
                  id: checkpoint.messageId,
                  role: "system",
                  content: t(checkpoint.localizationKey),
                },
              ],
        );
      } catch {
        if (!turnIsCurrent(preparedTurnContext)) {
          await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
          pendingSubmissions.end(submitScope);
          releaseTurnAttachments();
          return;
        }
        const failureText = localSearchFailureMessage(localSearchOutcome.errorCode, t);
        await endAcceptedTurnWithFailure(failureText, failureText);
        return;
      }
    }
    attachmentsForTurn = incorporateSucceededLocalSearch(attachmentsForTurn, localSearchOutcome, refreshAcknowledgedTurnReceipt);
    searchContinuationState = bindInitialSearchOutcome(searchContinuationState, localSearchOutcome);
    void (turnAcknowledged && activateBrowserPromptRoute(attachmentsForTurn));
    if (directLocalReadRequest) {
      if (!isTauriRuntime) {
        const failureText = t("chat.status.desktop_required");
        await endAcceptedTurnWithFailure(failureText, failureText);
        return;
      }
      try {
        updateTurnStatus(preparedTurnContext, t("chat.status.waiting_approval"));
        ({ attachments: readAttachments, modelMessage: turnModelMessage } = await prepareDirectLocalReadTurn(directLocalReadRequest, nextMessage, preparedTurnContext, attachmentsForTurn));
        attachmentsForTurn = [...attachmentsForTurn, ...readAttachments];
        updateTurnStatus(preparedTurnContext, t("chat.status.attachment_ready"));
      } catch (error) {
        const failureText = localCommandFailureText(error, t);
        await endAcceptedTurnWithFailure(failureText, t("chat.status.attachment_blocked"));
        return;
      }
    }
    if (directMailReadRequest) {
      if (!isTauriRuntime) {
        const failureText = t("chat.status.desktop_required");
        await endAcceptedTurnWithFailure(failureText, t("chat.status.desktop_required"));
        return;
      }
      try {
        updateTurnStatus(preparedTurnContext, directMailReadRequest.unreadOnly ? t("chat.status.reading_unread_mail") : t("chat.status.reading_mail"));
        const outcome = await runPermissionRecoverableAppleRead(
          async () => {
            const result = await invoke<McpToolCallResult>("read_system_emails", {
              maxMessages: directMailReadRequest.maxMessages,
              unreadOnly: directMailReadRequest.unreadOnly,
              turnContext: mcpTurnContextRequest(acceptedTurnContext),
            });
            return {
              result,
              resultText: localMailToolResultText(result, verifiedExecutionCopy),
            };
          },
          (error) => requestDirectApplePermissionRecovery(acceptedTurnContext, "mail", error),
        );
        if (outcome.status === "cancelled") {
          await endAcceptedTurnWithFailure(t("tasks.error_cancelled"), t("tasks.error_cancelled"), false, "cancelled");
          return;
        }
        const scopedResult = scopedLocalMailReadResult(directMailReadRequest, parseLocalMailReadResult(outcome.value.resultText, outcome.value.result.structuredContent), new Date());
        const scopedResultText = localMailReadResultText(scopedResult);
        localMailAssistantText = buildDirectLocalMailReadAssistantText(directMailReadRequest, scopedResultText);
        attachmentsForTurn = [...attachmentsForTurn, localMailToolAttachment(directMailReadRequest, scopedResultText)];
        updateTurnStatus(preparedTurnContext, t("chat.status.mail_ready"));
      } catch (error) {
        const failureText = t(localMailFailureKey(error));
        await endAcceptedTurnWithFailure(failureText, failureText);
        return;
      }
    }
    if (directCalendarReadRequest) {
      if (!isTauriRuntime) {
        const failureText = t("chat.errors.calendar_desktop_required");
        await endAcceptedTurnWithFailure(failureText, t("chat.status.desktop_required"));
        return;
      }
      try {
        updateTurnStatus(preparedTurnContext, t("chat.status.reading_calendar"));
        const outcome = await runPermissionRecoverableAppleRead(
          () =>
            invoke<McpToolCallResult>("read_system_calendar", {
              calendarName: directCalendarReadRequest.calendarName,
              startDate: directCalendarReadRequest.startDate,
              endDate: directCalendarReadRequest.endDate,
              hoursAhead: 24,
              turnContext: mcpTurnContextRequest(acceptedTurnContext),
            }),
          (error) => requestDirectApplePermissionRecovery(acceptedTurnContext, "calendar", error),
        );
        if (outcome.status === "cancelled") {
          await endAcceptedTurnWithFailure(t("tasks.error_cancelled"), t("tasks.error_cancelled"), false, "cancelled");
          return;
        }
        await refreshSessionMessages(sessionId, { hydrationLockToken }).catch(() => false);
        localCalendarResultText = mcpToolResultText(outcome.value, verifiedExecutionCopy);
        attachmentsForTurn = [...attachmentsForTurn, localCalendarToolAttachment(directCalendarReadRequest, localCalendarResultText, outcome.value.structuredContent, t)];
        updateTurnStatus(preparedTurnContext, t("chat.status.calendar_ready"));
      } catch (error) {
        const failureText = calendarToolFailureMessage(error, t);
        await endAcceptedTurnWithFailure(failureText, t("chat.status.calendar_blocked"));
        return;
      }
    }
    if (directAppleAppReadRequest) {
      if (!isTauriRuntime) {
        const failureText = ["read_system_contacts", "read_system_music", "read_system_photos"].includes(directAppleAppReadRequest.toolName) ? t(protectedAppleLibraryDesktopKey(directAppleAppReadRequest.toolName as ProtectedAppleLibraryToolName)) : t("chat.status.desktop_required");
        await endAcceptedTurnWithFailure(failureText, t("chat.status.desktop_required"));
        return;
      }
      const permissionCapability = localProductivityAppKindForTool(directAppleAppReadRequest.toolName) ?? (directAppleAppReadRequest.toolName === "read_apple_app_ui" ? "accessibility" : "");
      try {
        updateTurnStatus(
          preparedTurnContext,
          t("chat.status.reading_app", {
            app: directAppleAppReadRequest.appLabel,
          }),
        );
        const outcome = await runPermissionRecoverableAppleRead(
          async () => {
            const result = await executeSystemAppleAppTool(directAppleAppReadRequest.toolName, directAppleAppReadRequest.argumentsValue, acceptedTurnContext);
            const resultText = mcpToolResultText(result, verifiedExecutionCopy);
            if (isUiSnapshotBlocked(resultText)) throw { code: "accessibility_permission_required" };
            return resultText;
          },
          (error) => (permissionCapability ? requestDirectApplePermissionRecovery(acceptedTurnContext, permissionCapability, error) : null),
        );
        if (outcome.status === "cancelled") {
          await endAcceptedTurnWithFailure(t("tasks.error_cancelled"), t("tasks.error_cancelled"), false, "cancelled");
          return;
        }
        attachmentsForTurn = [...attachmentsForTurn, localAppleAppToolAttachment(directAppleAppReadRequest, outcome.value)];
        updateTurnStatus(
          preparedTurnContext,
          t("chat.status.app_context_ready", {
            app: directAppleAppReadRequest.appLabel,
          }),
        );
      } catch (error) {
        const protectedLibrary = ["read_system_contacts", "read_system_music", "read_system_photos"].includes(directAppleAppReadRequest.toolName);
        const failureText = protectedLibrary ? t(protectedAppleLibraryFailureKey(directAppleAppReadRequest.toolName as ProtectedAppleLibraryToolName, error)) : verifiedExecutionCopy.toolFailureWithoutDetails;
        await endAcceptedTurnWithFailure(
          failureText,
          t("chat.status.app_blocked", {
            app: directAppleAppReadRequest.appLabel,
          }),
        );
        return;
      }
    }
    if (directAppleAppWriteRequest) {
      if (!isTauriRuntime) {
        const failureText = t("sprint_301.apple_app_write.desktop_required", {
          app: directAppleAppWriteRequest.appLabel,
        });
        await endAcceptedTurnWithFailure(failureText, t("chat.status.desktop_required"));
        return;
      }
      let actionResultText = "";
      try {
        updateTurnStatus(
          preparedTurnContext,
          t("chat.status.preparing_app_approval", {
            app: directAppleAppWriteRequest.appLabel,
          }),
        );
        const result = await executeSystemAppleAppTool(directAppleAppWriteRequest.toolName, directAppleAppWriteRequest.argumentsValue, preparedTurnContext);
        actionResultText = mcpToolResultText(result, verifiedExecutionCopy);
      } catch {
        const failureText = t("sprint_301.apple_app_write.failed", {
          app: directAppleAppWriteRequest.appLabel,
        });
        await endAcceptedTurnWithFailure(
          failureText,
          t("chat.status.app_blocked", {
            app: directAppleAppWriteRequest.appLabel,
          }),
        );
        return;
      }

      updateTurnMessages(preparedTurnContext, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "assistant",
          content: actionResultText,
        },
      ]);
      updateTurnStatus(
        preparedTurnContext,
        t("chat.status.app_action_complete", {
          app: directAppleAppWriteRequest.appLabel,
        }),
      );
      const recorded = await invoke<ChatTurnResponse>("record_browser_chat_turn", {
        request: {
          ...nativeTurnContextRequest(preparedTurnContext),
          agent_id: preparedTurnContext.agentId,
          message: turnMessage,
          assistant_text: actionResultText,
          session_id: preparedTurnContext.sessionId,
          provider_id: preparedTurnContext.route.providerId,
          model_id: preparedTurnContext.route.modelId,
        },
      }).catch(() => null);
      if (!recorded) {
        const receiptFailure = t("chat.status.app_action_receipt_failed", {
          app: directAppleAppWriteRequest.appLabel,
        });
        await persistAcceptedTerminalResult({
          role: "system",
          content: receiptFailure,
          status: "failed",
        });
        updateTurnMessages(preparedTurnContext, (current) => [
          ...current,
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: receiptFailure,
          },
        ]);
        updateTurnStatus(preparedTurnContext, receiptFailure);
      } else if (turnIsCurrent(preparedTurnContext) && chatStreamResponseMatches(preparedTurnContext, recorded)) {
        await refreshSessionMessages(recorded.session_id ?? preparedTurnContext.sessionId, {
          hydrationLockToken,
        }).catch(() => undefined);
        compactSessionHistory(recorded.session_id ?? preparedTurnContext.sessionId, selectedAgent.id);
      }
      void invoke<ChatSession[]>("list_chat_sessions")
        .then(onSessionsChange)
        .catch(() => undefined);
      abandonPreparedTurn();
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      continueAfterTurn(preparedTurnContext.sessionId);
      return;
    }

    if (!turnIsCurrent(preparedTurnContext)) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }
    try {
      attachmentsForTurn = await prepareVisualAttachmentsForTurn(attachmentsForTurn, submitSession);
      attachmentsForTurn = await attachPrivateDataProvenance(attachmentsForTurn, preparedTurnContext.turnId);
    } catch (error) {
      const notice = chatFailureNotice(error, t);
      await endAcceptedTurnWithFailure(notice.content, notice.status);
      return;
    }
    if (!turnIsCurrent(preparedTurnContext)) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }
    if (turnContext) {
      const reboundTurnContext = rebindChatTurnAttachments(turnContext, attachmentsForTurn);
      turnContext = reboundTurnContext;
      registerActiveTurn(reboundTurnContext);
      preparedTurnContext = reboundTurnContext;
    }
    if (!turnIsCurrent(preparedTurnContext)) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }

    if (!directLocalCommand && !isSystemDiagnosticsPrompt(nextMessage)) {
      try {
        await validateModCompatibilityForMessage(nextMessage, turnRouteBinding, explicitSlashCommand?.modId ?? null);
      } catch (error) {
        const notice = chatFailureNotice(error, t);
        await endAcceptedTurnWithFailure(notice.content, notice.status);
        return;
      }
    }

    if (!turnIsCurrent(preparedTurnContext)) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      releaseTurnAttachments();
      return;
    }

    if (!turnAcknowledged) {
      if (!(await ensureTurnSession())) {
        releaseTurnAttachments();
        return;
      }
      acknowledgeTurn(attachmentsForTurn);
    }

    if (!sessionId) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      setIsSending(false);
      releaseTurnAttachments();
      return;
    }
    const immutableTurnContext = turnContext as ChatTurnContext | null;
    if (!immutableTurnContext) {
      await abandonAcceptedTurnAfterOwnershipLoss(preparedTurnContext);
      pendingSubmissions.end(submitScope);
      setIsSendingForSession(sessionId, false);
      setIsProcessingForSession(sessionId, false);
      releaseTurnAttachments();
      return;
    }
    const durableTurnContext = immutableTurnContext;
    const turnRoute = immutableTurnContext.route;
    let assistantMessageId: number | null = null;
    assistantMessageId = projectDocumentPendingAssistantId(turnModelMessage, immutableTurnContext.projectId, () => nextMessageIdRef.current++);
    updateTurnMessages(immutableTurnContext, (current) => ensurePendingAssistantMessage(current, assistantMessageId));
    let terminalReconciliationSessionId = "";
    async function recordAcceptedAssistantResult(assistantText: string) {
      const recorded = await invoke<ChatTurnResponse>("record_browser_chat_turn", {
        request: {
          ...nativeTurnContextRequest(durableTurnContext),
          agent_id: selectedAgent.id,
          message: turnMessage,
          assistant_text: assistantText,
          session_id: sessionId,
          provider_id: route.providerId,
          model_id: route.modelId,
        },
      });
      if (!chatStreamResponseMatches(durableTurnContext, recorded)) {
        throw { code: "chat_turn_response_mismatch" };
      }
      if (!turnIsCurrent(durableTurnContext)) {
        return;
      }
      const recordedSessionId = recorded.session_id ?? sessionId;
      await refreshSessionMessages(recordedSessionId, {
        hydrationLockToken,
      }).catch(() => undefined);
      compactSessionHistory(recordedSessionId, selectedAgent.id);
      return recorded;
    }

    try {
      updateTurnStatus(immutableTurnContext, t("chat.status.thinking"));
      if (directLocalCommand) {
        const pendingAssistantMessageId = nextMessageIdRef.current++;
        assistantMessageId = pendingAssistantMessageId;
        updateTurnMessages(immutableTurnContext, (current) => [
          ...current,
          {
            id: pendingAssistantMessageId,
            role: "assistant",
            content: "",
            isPending: true,
          },
        ]);

        let assistantText = "";
        let commandFailed = false;
        try {
          if (directLocalCommand.kind === "list") {
            updateTurnStatus(immutableTurnContext, t("chat.status.listing_files"));
            if (isHostLocalPath(directLocalCommand.path)) {
              const response = await nativeDirectFileAccess("file_list", directLocalCommand.path, immutableTurnContext);
              assistantText = directExecuteCommandText(response, t("chat.errors.local_action_unavailable.content"), verifiedExecutionCopy);
            } else {
              if (!mcp) {
                throw new Error("Local tools are unavailable for this turn.");
              }
              const result = await mcp.executeTool(
                "local_filesystem",
                "list_directory",
                { path: directLocalCommand.path },
                {
                  requestApproval: (approval) => requestMcpShieldApproval(approval, immutableTurnContext),
                  isExecutionContextCurrent: () => turnIsCurrent(immutableTurnContext),
                  turnContext: mcpTurnContextRequest(immutableTurnContext),
                },
              );
              assistantText = mcpToolResultText(result, verifiedExecutionCopy);
            }
          } else if (directLocalCommand.kind === "write") {
            updateTurnStatus(immutableTurnContext, t("chat.status.preparing_write_approval"));
            if (isHostLocalPath(directLocalCommand.path)) {
              const response = await invoke<ExecuteCommandResponse>("execute_command", {
                request: {
                  action: {
                    kind: "file_write",
                    path: directLocalCommand.path,
                    content: directLocalCommand.content,
                  },
                  logical_certificate: null,
                  session_id: sessionId,
                  turn_id: immutableTurnContext.turnId,
                  generation_token: immutableTurnContext.generationToken,
                  agent_id: immutableTurnContext.agentId,
                  provider_id: turnRoute.providerId,
                  model_id: turnRoute.modelId,
                  parent_turn_id: immutableTurnContext.ancestry.parentTurnId,
                  root_turn_id: immutableTurnContext.ancestry.rootTurnId,
                  turn_kind: immutableTurnContext.ancestry.kind,
                },
              });
              assistantText = directExecuteCommandText(response, `Unable to verify file creation at ${directLocalCommand.path}.`, verifiedExecutionCopy);
            } else {
              if (!mcp) {
                throw new Error("Local tools are unavailable for this turn.");
              }
              const result = await mcp.executeTool(
                "local_filesystem",
                "write_file",
                {
                  path: directLocalCommand.path,
                  content: directLocalCommand.content,
                },
                {
                  requestApproval: (approval) => requestMcpShieldApproval(approval, immutableTurnContext),
                  isExecutionContextCurrent: () => turnIsCurrent(immutableTurnContext),
                  turnContext: mcpTurnContextRequest(immutableTurnContext),
                },
              );
              assistantText = mcpToolResultText(result, verifiedExecutionCopy);
            }
          } else if (directLocalCommand.kind === "delete") {
            updateTurnStatus(immutableTurnContext, t("chat.status.preparing_delete_approval"));
            if (isHostLocalPath(directLocalCommand.path)) {
              const response = await invoke<ExecuteCommandResponse>("execute_command", {
                request: {
                  action: {
                    kind: "delete_file",
                    path: directLocalCommand.path,
                  },
                  logical_certificate: null,
                  session_id: sessionId,
                  turn_id: immutableTurnContext.turnId,
                  generation_token: immutableTurnContext.generationToken,
                  agent_id: immutableTurnContext.agentId,
                  provider_id: turnRoute.providerId,
                  model_id: turnRoute.modelId,
                  parent_turn_id: immutableTurnContext.ancestry.parentTurnId,
                  root_turn_id: immutableTurnContext.ancestry.rootTurnId,
                  turn_kind: immutableTurnContext.ancestry.kind,
                },
              });
              assistantText = directExecuteCommandText(response, `Unable to verify deletion of ${directLocalCommand.path}.`, verifiedExecutionCopy);
            } else {
              if (!mcp) {
                throw new Error("Local tools are unavailable for this turn.");
              }
              const result = await mcp.executeTool(
                "local_filesystem",
                "delete_file",
                { path: directLocalCommand.path },
                {
                  requestApproval: (approval) => requestMcpShieldApproval(approval, immutableTurnContext),
                  isExecutionContextCurrent: () => turnIsCurrent(immutableTurnContext),
                  turnContext: mcpTurnContextRequest(immutableTurnContext),
                },
              );
              assistantText = mcpToolResultText(result, verifiedExecutionCopy);
            }
          } else {
            updateTurnStatus(immutableTurnContext, t("chat.status.preparing_command_approval"));
            const response = await invoke<ExecuteCommandResponse>("execute_command", {
              request: {
                action: {
                  kind: "shell_command",
                  content: directLocalCommand.command,
                },
                logical_certificate: null,
                session_id: sessionId,
                ...nativeProjectTurnContextRequest(immutableTurnContext),
                agent_id: immutableTurnContext.agentId,
                provider_id: turnRoute.providerId,
                model_id: turnRoute.modelId,
              },
            });
            assistantText = directExecuteCommandText(response, "The shell command did not return a verified successful exit.", verifiedExecutionCopy);
          }
        } catch (error) {
          commandFailed = true;
          assistantText = localCommandFailureText(error, t);
        }

        updateTurnMessages(immutableTurnContext, (current) => current.map((entry) => (entry.id === pendingAssistantMessageId ? { ...entry, content: assistantText, isPending: false } : entry)));
        await recordAcceptedAssistantResult(assistantText);
        void invoke<ChatSession[]>("list_chat_sessions")
          .then(onSessionsChange)
          .catch(() => undefined);
        updateTurnStatus(immutableTurnContext, commandFailed ? t("chat.status.command_failed") : t("chat.status.ready"));
        return;
      }

      if (directMailReadRequest) {
        const assistantText = localMailAssistantText ?? "I tried to check Mail, but the local Mail result could not be read.";
        const pendingAssistantMessageId = nextMessageIdRef.current++;
        assistantMessageId = pendingAssistantMessageId;
        updateTurnMessages(immutableTurnContext, (current) => [
          ...current,
          {
            id: pendingAssistantMessageId,
            role: "assistant",
            content: assistantText,
          },
        ]);
        await recordAcceptedAssistantResult(assistantText);
        void invoke<ChatSession[]>("list_chat_sessions")
          .then(onSessionsChange)
          .catch(() => undefined);
        updateTurnStatus(immutableTurnContext, t("chat.status.ready"));
        return;
      }

      if (isSystemDiagnosticsPrompt(nextMessage)) {
        updateTurnStatus(immutableTurnContext, t("chat.status.running_diagnostics"));
        const pendingAssistantMessageId = nextMessageIdRef.current++;
        assistantMessageId = pendingAssistantMessageId;
        updateTurnMessages(immutableTurnContext, (current) => [
          ...current,
          {
            id: pendingAssistantMessageId,
            role: "assistant",
            content: "Running system diagnostics...",
          },
        ]);
        const report = await invoke<SystemDiagnosticsReport>("run_system_diagnostics", {
          request: systemDiagnosticsRequest(immutableTurnContext),
        });
        const assistantText = systemDiagnosticsChatSummary(report);
        updateTurnMessages(immutableTurnContext, (current) => current.map((entry) => (entry.id === pendingAssistantMessageId ? { ...entry, content: assistantText } : entry)));
        await recordAcceptedAssistantResult(assistantText);
        void invoke<ChatSession[]>("list_chat_sessions")
          .then(onSessionsChange)
          .catch(() => undefined);
        updateTurnStatus(immutableTurnContext, report.status === "passed" ? t("chat.status.diagnostics_complete") : t("chat.status.diagnostics_attention"));
        return;
      }

      let toolRegistryOfflineForTurn = false;
      if (!explicitSlashCommand) {
        if (directLocalReadRequest && !approvedLocalFilesContextReady(readAttachments, attachmentsForTurn)) {
          throw { code: "approved_file_unavailable" };
        }
        let routeDecision: ChatIntentRouteDecision = await preferProjectDocumentRoute(projectDocumentRouteDecision(turnModelMessage, immutableTurnContext.projectId, t("chat.status.thinking")), async () =>
          preferContextualArtifactRoute(
            contextualMarkdownRoute,
            recoveryPlan
              ? recoveryPlanRouteDecision(t("chat.status.planning_steps", { name: selectedAgent.name }))
              : directLocalReadRequest
                ? verifiedDirectFileReadRouteDecision(t("chat.status.thinking"))
                : ambiguousLocalAppTriageFailure
                  ? {
                      route: "conversational_stream",
                      requires_local_access: false,
                      decision_source: "frontend_ambiguous_local_app_filter",
                      reason: "Ambiguous app wording remained conversational after local triage became unavailable.",
                      matched_signals: [],
                      status_label: t("chat.status.thinking"),
                    }
                  : await invoke<ChatIntentRouteDecision>("classify_chat_intent_route", {
                      sessionId,
                      session_id: sessionId,
                      selectedProviderId: turnRouteBinding.providerId,
                      selected_provider_id: turnRouteBinding.providerId,
                      selectedModelId: turnRouteBinding.modelId,
                      selected_model_id: turnRouteBinding.modelId,
                      request: {
                        prompt: turnModelMessage || "Please review the attached file.",
                        automated_web_grounding_enabled: automatedWebGroundingEnabled,
                        attachments: attachmentsForTurn.map((attachment) => ({
                          name: attachment.name,
                          mime_type: attachment.mime_type,
                          byte_count: attachment.byte_count,
                          text: attachment.text,
                        })),
                      },
                    }),
          ),
        );
        const directPrivateAppReadResource = directCalendarReadRequest ? "calendar" : workspaceResourceForAppleReadTool(directAppleAppReadRequest?.toolName ?? "");
        const hasHydratedDirectPrivateAppResult = Boolean(directPrivateAppReadResource && workspaceDataResourcesForAttachments(attachmentsForTurn).has(directPrivateAppReadResource));
        if (hasHydratedDirectPrivateAppResult) {
          routeDecision = {
            ...routeDecision,
            route: "conversational_stream",
            requires_local_access: false,
            decision_source: "hydrated_direct_private_app_result",
          };
        }
        [projectDocumentTurn, routeDecision, turnModelMessage] = prepareProjectChatDocumentTurn(turnModelMessage, routeDecision, immutableTurnContext.projectId, t("chat.status.thinking"));
        outstandingNativeEffect = nativeEffectExpectationForRouteDecision(routeDecision);
        if (
          await completeOneTimeRoutineHandoff({
            decision: routeDecision,
            prompt: turnModelMessage,
            onOpenRoutine,
            complete: endAcceptedTurnWithFailure,
            content: t("chat.routine_handoff.content"),
            status: t("chat.routine_handoff.status"),
          })
        )
          return;
        responseRequiresNativeExecutionReceipt ||= routeDecision.requires_local_access;
        updateTurnStatus(immutableTurnContext, routeDecision.route === "agentic_planner" ? t("chat.status.planning_steps", { name: selectedAgent.name }) : t("chat.status.thinking"));
        turnMcpTools = projectDocumentMcpCapabilities(projectDocumentTurn, recoveryPlan ? [] : mcpCapabilitiesForContextualTurn(conversationalMcpCapabilities, routeDecision, attachmentsForTurn));
        const useConversationalMcpBridge = !recoveryPlan && shouldUseConversationalMcpBridge(turnModelMessage, routeDecision, turnMcpTools);
        toolRegistryOfflineForTurn = routeDecision.decision_source === "dynamic_routing_disabled";
        const shouldForceLocalNativePlanner = !recoveryPlan && !ambiguousLocalAppTriageFailure && !hasHydratedDirectPrivateAppResult && likelyLocalNativeTaskIntent && (localModelIsHydrating || toolRegistryOfflineForTurn);

        if (!ambiguousLocalAppTriageFailure && shouldDelegateToTaskFlow(turnModelMessage, routeDecision)) {
          updateTurnStatus(immutableTurnContext, t("chat.status.compiling_taskflow", { name: selectedAgent.name }));
          const pendingAssistantMessageId = nextMessageIdRef.current++;
          assistantMessageId = pendingAssistantMessageId;
          let taskFlowLines = ["TaskFlow delegation accepted.", "Compiling local execution state..."];
          const renderTaskFlowLines = () => {
            const content = taskFlowLines.join("\n");
            updateTurnMessages(immutableTurnContext, (current) => current.map((entry) => (entry.id === pendingAssistantMessageId ? { ...entry, content } : entry)));
            return content;
          };
          const appendTaskFlowLine = (line: string) => {
            taskFlowLines = [...taskFlowLines, line].slice(-36);
            renderTaskFlowLines();
          };
          updateTurnMessages(immutableTurnContext, (current) => [
            ...current,
            {
              id: pendingAssistantMessageId,
              role: "assistant",
              content: taskFlowLines.join("\n"),
            },
          ]);

          const taskFlowTurnContext = {
            ...nativeTurnContextRequest(immutableTurnContext),
            session_id: immutableTurnContext.sessionId,
            agent_id: immutableTurnContext.agentId,
            provider_id: turnRoute.providerId,
            model_id: turnRoute.modelId,
          };

          const taskFlow = await createTaskFlow({
            directive: buildChatTaskFlowDirective(turnModelMessage || "Compile a TaskFlow for the attached local context.", attachmentsForTurn),
            parent_session_id: sessionId,
            ...taskFlowTurnContext,
          });
          if (!turnIsCurrent(immutableTurnContext)) {
            await abandonAcceptedTurnAfterOwnershipLoss(immutableTurnContext);
            return;
          }
          taskFlowLines = ["TaskFlow compiled.", `Flow ID: ${taskFlow.flow_id}`, `Mission ID: ${taskFlow.mission_id}`, `${taskFlow.steps.length} steps queued.`];
          renderTaskFlowLines();

          const unlistenTaskFlow = await subscribeToTaskFlowEvents({
            flowId: taskFlow.flow_id,
            onProgress: (event) => appendTaskFlowLine(taskFlowChatLine(event)),
            onThought: (event) => appendTaskFlowLine(taskFlowChatLine(event)),
          });
          let response: TaskFlowExecutionResponse;
          try {
            updateTurnStatus(immutableTurnContext, t("chat.status.executing_taskflow"));
            response = await executeTaskFlow(taskFlow.flow_id, taskFlowTurnContext);
          } finally {
            unlistenTaskFlow();
          }

          const taskFlowVerified = taskFlowExecutionIsVerified(response);
          taskFlowLines = [
            ...taskFlowLines,
            "",
            taskFlowVerified ? t("chat.status.taskflow_receipt_verified") : t("chat.status.taskflow_receipt_missing"),
            t("chat.status.taskflow_completed_steps", {
              completed: response.completed_steps,
              total: response.flow.steps.length,
            }),
          ];
          if (response.diagnostic) {
            taskFlowLines.push(`Diagnostic: ${response.diagnostic.reason}`);
            taskFlowLines.push(`Suggested fix: ${response.diagnostic.suggested_fix}`);
          }
          const assistantText = renderTaskFlowLines();
          await recordAcceptedAssistantResult(assistantText);
          void invoke<ChatSession[]>("list_chat_sessions")
            .then(onSessionsChange)
            .catch(() => undefined);
          updateTurnStatus(immutableTurnContext, taskFlowVerified ? t("chat.status.taskflow_complete") : t("chat.status.taskflow_attention"));
          return;
        }

        if ((recoveryPlan || routeDecision.route === "agentic_planner" || shouldForceLocalNativePlanner) && !useConversationalMcpBridge) {
          updateTurnStatus(immutableTurnContext, t("chat.status.planning_steps", { name: selectedAgent.name }));
          const planPrompt = [turnModelMessage, ...attachmentsForTurn.filter((attachment) => attachment.text).map((attachment) => `Local text attachment: ${attachment.name}\n${attachment.text}`)].filter(Boolean).join("\n\n");
          let plan: ActionPlan | null = null;
          try {
            plan = await invoke<ActionPlan>("process_agent_objective", {
              request: {
                agent_id: selectedAgent.id,
                user_objective: turnModelMessage || "Review the attached local context.",
                prompt: planPrompt || "Compile an action plan for the attached local context.",
                session_id: sessionId,
                ...nativeProjectTurnContextRequest(immutableTurnContext),
                provider_id: turnRoute.providerId,
                model_id: turnRoute.modelId,
                automated_web_grounding_enabled: turnRoute.automatedWebGroundingEnabled,
                ...plannerRequestRoute(configuredProviders, turnRoute),
              },
            });
          } catch (error) {
            if (!invokeErrorHasCode(error, "agent_objective_not_executable")) {
              throw error;
            }
            if (!plannerConversationFallbackAllowed) {
              throw error;
            }
            responseRequiresNativeExecutionReceipt = false;
            turnMcpTools = [];
            toolRegistryOfflineForTurn = false;
            updateTurnStatus(immutableTurnContext, t("chat.status.thinking"));
          }
          if (plan) {
            if (!turnIsCurrent(immutableTurnContext)) {
              await abandonAcceptedTurnAfterOwnershipLoss(immutableTurnContext);
              return;
            }
            if (plan.trusted_automatic_execution) {
              await executeAgentPlan({ plan, turnContext: immutableTurnContext }, false);
              return;
            }
            const planSummary = localizedAgentPlanSummary(t, plan);
            const planReceipt = await recordAcceptedAssistantResult(planSummary);
            if (!planReceipt || !turnIsCurrent(immutableTurnContext)) {
              return;
            }
            const pendingExecution = {
              plan,
              turnContext: executionTurnContextFromPlanReceipt(immutableTurnContext, planReceipt.metadata),
            };
            setPendingPlanForSession(immutableTurnContext.sessionId, pendingExecution);
            options.onPlanReady?.();
            void invoke<ChatSession[]>("list_chat_sessions")
              .then(onSessionsChange)
              .catch(() => undefined);
            updateTurnStatus(immutableTurnContext, t("chat.status.plan_awaiting_approval"));
            return;
          }
        }
      }

      updateTurnStatus(immutableTurnContext, inferenceProgressStatus("contacting", turnRoute.dynamicRoutingEnabled, modelLabel(configuredProviders, turnRoute.providerId, turnRoute.modelId), t));
      const pendingAssistantMessageId = assistantMessageId ?? nextMessageIdRef.current++;
      assistantMessageId = pendingAssistantMessageId;
      activeAssistantMessageIdsRef.current.set(sessionId, pendingAssistantMessageId);
      updateTurnMessages(immutableTurnContext, (current) => ensurePendingAssistantMessage(current, pendingAssistantMessageId));
      const streamId = crypto.randomUUID();
      registerActiveTurn(immutableTurnContext, streamId);
      const streamController = createProjectedChatStreamController({
        streamId,
        turn: immutableTurnContext,
        ownsTurn: () => turnIsCurrent(immutableTurnContext) && !pendingSteerSupersedesTurn(immutableTurnContext),
        requiresNativeReceipt: responseRequiresNativeExecutionReceipt,
        assistantMessageId: pendingAssistantMessageId,
        updateMessages: (update: (messages: ChatTranscriptMessage[]) => ChatTranscriptMessage[]) => updateTurnMessages(immutableTurnContext, update),
        directiveSessionId: sessionId,
        directiveGrants: browserDirectiveGrantsForResponse,
        activateDirective: activateAuthorizedBrowserDirective,
        activateRoute: activateBrowserSplitRoute,
        onFirstToken: () => updateTurnStatus(immutableTurnContext, inferenceProgressStatus("streaming", turnRoute.dynamicRoutingEnabled, modelLabel(configuredProviders, turnRoute.providerId, turnRoute.modelId), t)),
      });
      await streamController.listen();

      let response: ChatTurnResponse;
      let selectedAutoRouteChoice: "local" | "cloud" | null = options.autoRouteResumeChoice ?? null;
      let projectCloudConfirmed = false;
      try {
        if (turnRoute.dynamicRoutingEnabled) {
          updateTurnStatus(immutableTurnContext, t("chat.status.choosing_model"));
          const sessionConfigReady = await (sessionConfigPersistPromisesRef.current.get(immutableTurnContext.sessionId) ?? Promise.resolve(true));
          if (!sessionConfigReady) {
            throw {
              code: "auto_route_session_baseline_persistence_failed",
              message: t("chat.auto_route_attention.baseline_save_failed"),
            };
          }
        }
        for (;;) {
          try {
            const projectRoute = projectDocumentNativeRequestRoute(projectDocumentTurn, autoRouteSessionReadiness, turnRoute, routeUsesLocalModel(configuredProviders, turnRoute.providerId), selectedAutoRouteChoice, t("documents.create_failed"), turnMcpTools);
            response = await invoke<ChatTurnResponse>("chat_turn", {
              request: {
                turn_id: immutableTurnContext.turnId,
                generation_token: immutableTurnContext.generationToken,
                parent_turn_id: immutableTurnContext.ancestry.parentTurnId,
                root_turn_id: immutableTurnContext.ancestry.rootTurnId,
                turn_kind: immutableTurnContext.ancestry.kind,
                agent_id: immutableTurnContext.agentId,
                message: turnModelMessage || "Please review the attached file.",
                display_message: turnMessage || nextMessage || "Please review the attached file.",
                attachments: attachmentsForTurn,
                session_id: immutableTurnContext.sessionId,
                locale: language,
                requested_mod_id: explicitSlashCommand?.modId ?? null,
                stream_id: streamId,
                reasoning: turnRoute.reasoning,
                context: turnRoute.contextBudget?.toString(),
                context_budget: turnRoute.contextBudget,
                primary_route_id: turnRoute.primaryRouteId,
                fallback_route_id: turnRoute.fallbackRouteId,
                automated_web_grounding_enabled: ambiguousLocalAppTriageFailure ? false : turnRoute.automatedWebGroundingEnabled,
                project_cloud_confirmed: projectCloudConfirmed,
                ...projectRoute,
              },
            });
            break;
          } catch (error) {
            const consent = await resolveChatCloudConsentBoundary({
              error,
              turn: immutableTurnContext,
              projectId: immutableTurnContext.projectId,
              projectDestination: (turnRoute.dynamicRoutingEnabled ? compactExecutionModelLabel(autoRouteCloudModelForTurn) : modelLabel(configuredProviders, turnRoute.providerId, turnRoute.modelId)) || t("chat.project_cloud_consent.configured_destination"),
              privateDestination: (providerId, modelId) => modelLabel(configuredProviders, providerId, modelId) || modelId,
              requestProjectConsent: cloudConsent.requestProjectCloudConsent,
              requestPrivateConsent: cloudConsent.requestPrivateEgressConsent,
            });
            if (consent !== null) {
              projectCloudConfirmed = consent;
              continue;
            }
            if (!turnRoute.dynamicRoutingEnabled || !isAutoRouteAttentionError(error)) {
              throw error;
            }
            const choice = await requestAutoRouteTurnChoice(immutableTurnContext, turnRoute.providerId, autoRouteLocalModelForTurn, autoRouteCloudModelForTurn, error, autoRouteSessionReadiness.recommendedLocalProviderId ?? "", autoRouteSessionReadiness.recommendedLocalModelId ?? "");
            if (choice === "cancel") {
              throw { code: "auto_route_choice_cancelled" };
            }
            selectedAutoRouteChoice = choice === "retry" ? null : choice;
            updateTurnStatus(immutableTurnContext, t("chat.status.choosing_model"));
          }
        }
        await streamController.awaitValidatedDrain(response.text, undefined, !response.route_escalation);
      } finally {
        streamController.teardown();
      }

      if (!turnIsCurrent(immutableTurnContext) || pendingSteerSupersedesTurn(immutableTurnContext) || !chatStreamResponseMatches(immutableTurnContext, response)) {
        return;
      }

      if (!ambiguousLocalAppTriageFailure && response.route_escalation?.route === "agentic_planner") {
        const escalation = response.route_escalation;
        const responseSessionId = response.session_id ?? sessionId;
        updateTurnStatus(immutableTurnContext, t("chat.status.planning_steps", { name: selectedAgent.name }));
        updateTurnMessages(immutableTurnContext, (current) =>
          current.map((entry) =>
            entry.id === pendingAssistantMessageId
              ? {
                  ...entry,
                  content: ["Local action route detected.", `Route: ${escalation.decision_source}`, escalation.reason, "", "Compiling an approval-gated action plan..."].join("\n"),
                }
              : entry,
          ),
        );
        const planPrompt = [turnModelMessage, ...attachmentsForTurn.filter((attachment) => attachment.text).map((attachment) => `Local text attachment: ${attachment.name}\n${attachment.text}`)].filter(Boolean).join("\n\n");
        let plan: ActionPlan | null = null;
        try {
          plan = await invoke<ActionPlan>("process_agent_objective", {
            request: {
              agent_id: selectedAgent.id,
              user_objective: turnModelMessage || "Review the attached local context.",
              prompt: planPrompt || "Compile an action plan for the attached local context.",
              session_id: responseSessionId,
              ...nativeProjectTurnContextRequest(immutableTurnContext),
              provider_id: turnRoute.providerId,
              model_id: turnRoute.modelId,
              automated_web_grounding_enabled: turnRoute.automatedWebGroundingEnabled,
              ...plannerRequestRoute(configuredProviders, turnRoute),
            },
          });
        } catch (error) {
          if (!invokeErrorHasCode(error, "agent_objective_not_executable") || !plannerConversationFallbackAllowed || Boolean(explicitSlashCommand)) {
            throw error;
          }

          const retryContext = immutableTurnContext;
          const retryResponse = await invoke<ChatTurnResponse>("chat_turn", {
            request: {
              ...nativeTurnContextRequest(retryContext),
              agent_id: retryContext.agentId,
              message: turnModelMessage || "Please review the attached file.",
              display_message: turnMessage || nextMessage || "Please review the attached file.",
              attachments: attachmentsForTurn,
              session_id: retryContext.sessionId,
              provider_id: retryContext.route.providerId,
              model_id: retryContext.route.modelId,
              locale: language,
              requested_mod_id: null,
              reasoning: retryContext.route.reasoning,
              context: retryContext.route.contextBudget?.toString(),
              context_budget: retryContext.route.contextBudget,
              primary_route_id: retryContext.route.primaryRouteId,
              fallback_route_id: retryContext.route.fallbackRouteId,
              automated_web_grounding_enabled: retryContext.route.automatedWebGroundingEnabled,
              dynamic_routing_override: retryContext.route.dynamicRoutingEnabled,
              auto_route_choice: selectedAutoRouteChoice,
              auto_route_cloud_confirmed: selectedAutoRouteChoice === "cloud",
              project_cloud_confirmed: projectCloudConfirmed,
              mcp_tool_capabilities: [],
            },
          });
          if (!turnIsCurrent(immutableTurnContext) || !chatStreamResponseMatches(retryContext, retryResponse)) {
            return;
          }
          if (retryResponse.route_escalation?.route === "agentic_planner") {
            throw error;
          }
          response = retryResponse;
          responseRequiresNativeExecutionReceipt = false;
          turnMcpTools = [];
          toolRegistryOfflineForTurn = false;
          updateTurnStatus(immutableTurnContext, t("chat.status.thinking"));
        }
        if (!turnIsCurrent(immutableTurnContext)) {
          await abandonAcceptedTurnAfterOwnershipLoss(immutableTurnContext);
          return;
        }
        if (plan) {
          if (plan.trusted_automatic_execution) {
            await executeAgentPlan({ plan, turnContext: immutableTurnContext }, false);
            return;
          }
          const planSummary = localizedAgentPlanSummary(t, plan);
          const planReceipt = await recordAcceptedAssistantResult(planSummary);
          if (!planReceipt || !turnIsCurrent(immutableTurnContext)) {
            return;
          }
          const pendingExecution = {
            plan,
            turnContext: executionTurnContextFromPlanReceipt(immutableTurnContext, planReceipt.metadata),
          };
          setPendingPlanForSession(immutableTurnContext.sessionId, pendingExecution);
          options.onPlanReady?.();
          void invoke<ChatSession[]>("list_chat_sessions")
            .then(onSessionsChange)
            .catch(() => undefined);
          updateTurnStatus(immutableTurnContext, t("chat.status.plan_awaiting_approval"));
          return;
        }
      }

      response.text = await createProjectChatDocumentForTurn(projectDocumentTurn, response, immutableTurnContext, language, updateTurnStatus, t);
      const { metadata: responseMetadata, text: sanitizedResponseText } = localizedAssistantResponse(response.text, response.metadata, turnRouteBinding.providerId, turnRouteBinding.modelId, t);
      activateAuthorizedBrowserDirective(sanitizedResponseText, pendingAssistantMessageId, response.session_id ?? sessionId, browserDirectiveGrantsForResponse, activateBrowserSplitRoute);
      const { searchRequest: searchContinuationRequest, mcpRequest: mcpToolRequest, displayText: assistantDisplayText } = assistantControlProjection(sanitizedResponseText, t);
      const unverifiedActionClaim = !mcpToolRequest && !searchContinuationRequest && shouldBlockOutstandingNativeEffectClaim(assistantDisplayText, nextMessage, responseRequiresNativeExecutionReceipt || hasLikelyLocalNativeTaskIntent(nextMessage) || hasExplicitBrowserNavigationIntent(nextMessage), outstandingNativeEffect, responseMetadata?.verifiedNativeExecutionReceipt === true);
      updateTurnMessages(immutableTurnContext, (current) =>
        current.map((entry) =>
          entry.id === assistantMessageId
            ? {
                ...entry,
                role: unverifiedActionClaim ? "system" : entry.role,
                content: unverifiedActionClaim ? t("trust.unverified_action_claim") : assistantDisplayText,
                providerId: responseMetadata?.executingProviderId ?? turnRouteBinding.providerId,
                modelId: responseMetadata?.executingModelId ?? turnRouteBinding.modelId,
                metadata: responseMetadata,
                isPending: false,
              }
            : entry,
        ),
      );
      const responseSessionId = response.session_id ?? sessionId;
      void invoke<ChatSession[]>("list_chat_sessions")
        .then(onSessionsChange)
        .catch(() => undefined);
      if (searchContinuationRequest) {
        await handleSearchContinuationRequest(searchContinuationRequest, searchContinuationTurnContext(immutableTurnContext, pendingAssistantMessageId, searchContinuationState, turnMcpTools, 0));
      } else if (mcpToolRequest) {
        await handleConversationalMcpToolRequest(mcpToolRequest, {
          turnContext: immutableTurnContext,
          sessionId: responseSessionId,
          agentId: immutableTurnContext.agentId,
          providerId: turnRoute.providerId,
          modelId: turnRoute.modelId,
          reasoning: turnRoute.reasoning,
          context: turnRoute.contextBudget?.toString(),
          contextBudget: turnRoute.contextBudget,
          primaryRouteId: turnRoute.primaryRouteId,
          fallbackRouteId: turnRoute.fallbackRouteId,
          automatedWebGroundingEnabled: turnRoute.automatedWebGroundingEnabled,
          capabilities: turnMcpTools,
          toolLoopDepth: 0,
          outstandingNativeEffect,
        });
      } else {
        terminalReconciliationSessionId = responseSessionId;
        updateTurnStatus(immutableTurnContext, t("chat.status.ready"));
        await publishBackgroundCompletionAttention(responseSessionId, immutableTurnContext.turnId);
      }
    } catch (error) {
      const errorCode = stableErrorCode(error);
      setMessagesForSession(immutableTurnContext.sessionId, (current) => markAcceptedTurnTerminalAfterError(current, immutableTurnContext.turnId, errorCode));
      if (!turnIsCurrent(immutableTurnContext)) {
        if (!cancelledGenerationTokensRef.current.has(immutableTurnContext.generationToken)) {
          await abandonAcceptedTurnAfterOwnershipLoss(immutableTurnContext, hydrationLockToken);
        }
        return;
      }
      if (pendingSteerSupersedesTurn(immutableTurnContext)) {
        await abandonAcceptedTurnAfterOwnershipLoss(immutableTurnContext, hydrationLockToken);
        return;
      }
      setIsSendingForSession(immutableTurnContext.sessionId, false);
      if (errorCode === "chat_turn_already_running") {
        updateTurnMessages(immutableTurnContext, (current) =>
          current.map((entry) =>
            entry.id === assistantMessageId
              ? {
                  ...entry,
                  content: t("chat.errors.turn_in_progress.content"),
                  isPending: false,
                }
              : entry,
          ),
        );
        updateTurnStatus(immutableTurnContext, t("chat.errors.turn_in_progress.status"));
        const reconciliation = await reconcileTerminalChatTurn(immutableTurnContext, hydrationLockToken, () => turnIsCurrent(immutableTurnContext) && !pendingSteerSupersedesTurn(immutableTurnContext));
        if (reconciliation === "terminal") {
          compactSessionHistory(immutableTurnContext.sessionId, immutableTurnContext.agentId);
        } else if (reconciliation === "timed_out" && turnIsCurrent(immutableTurnContext)) {
          updateTurnMessages(immutableTurnContext, (current) =>
            current.map((entry) =>
              entry.id === assistantMessageId
                ? {
                    ...entry,
                    content: t("chat.errors.turn_delayed.content"),
                    isPending: false,
                  }
                : entry,
            ),
          );
          updateTurnStatus(immutableTurnContext, t("chat.errors.turn_delayed.status"));
        }
        return;
      }
      if (errorCode === "private_egress_user_denied") {
        const keptPrivateContent = t("chat.private_egress_consent.kept_private_content");
        await finalizeDurableChatTurn(immutableTurnContext, {
          role: "system",
          content: keptPrivateContent,
          status: "cancelled",
        }).catch(() => undefined);
        updateTurnMessages(immutableTurnContext, (current) => [
          ...current.filter((entry) => entry.id !== assistantMessageId),
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: keptPrivateContent,
          },
        ]);
        updateTurnStatus(immutableTurnContext, t("chat.private_egress_consent.kept_private_status"));
        return;
      }
      if (await surfaceStoppedTurn(errorCode, immutableTurnContext, hydrationLockToken, assistantMessageId)) return;
      const notice = chatFailureNotice(error, t);
      const clarification = errorCode === "contextual_filename_required";
      await finalizeDurableChatTurn(immutableTurnContext, {
        role: clarification ? "assistant" : "system",
        content: notice.content,
        status: clarification ? "escalated" : "failed",
      }).catch(() => undefined);
      updateTurnMessages(immutableTurnContext, (current) => [
        ...current.filter((entry) => entry.id !== assistantMessageId),
        {
          id: nextMessageIdRef.current++,
          role: clarification ? "assistant" : "system",
          content: notice.content,
        },
      ]);
      updateTurnStatus(immutableTurnContext, notice.status);
    } finally {
      const wasCancelled = cancelledGenerationTokensRef.current.delete(immutableTurnContext.generationToken);
      clearActiveTurn(immutableTurnContext);
      if (hydrationLockToken !== null) {
        unlockSessionHydration(sessionId, hydrationLockToken);
      }
      if (terminalReconciliationSessionId) {
        const reconciliationSessionId = terminalReconciliationSessionId;
        void refreshSessionMessages(reconciliationSessionId)
          .then((refreshed) => {
            if (refreshed && !activeTurnForSession(reconciliationSessionId) && !executingQueueSessionsRef.current.has(reconciliationSessionId)) {
              compactSessionHistory(reconciliationSessionId, selectedAgent.id);
            }
          })
          .catch(() => undefined);
      }
      pendingSubmissions.end(submitScope);
      releaseAttachmentPayloads(attachmentsForTurn);
      if (!wasCancelled) {
        continueAfterTurn(sessionId);
      }
    }
  }
  async function handleQueueMessage(nextMessageValue: string, options?: { sessionId?: string }) {
    const nextMessage = nextMessageValue.trim();
    const queueSessionId = options?.sessionId ?? activeSessionId;
    const queuedDuringTurn = Boolean(queueSessionId && activeTurnForSession(queueSessionId));
    if ((!nextMessage && attachments.length === 0) || !selectedAgent || isReadingAttachments || executingQueueSessionsRef.current.has(activeSessionId) || !route.modelId || localModelIsHydrating) {
      if (localModelIsHydrating) {
        setChatStatus(t("chat.status.model_hydrating"));
      }
      return;
    }
    setBypassNotice(null);
    const queuedLocalIntent = detectDirectLocalCommand(nextMessage);
    const queuedLocalRead = queuedLocalIntent?.kind === "read" ? queuedLocalIntent : null;
    const detectedPathAttachments: ChatAttachment[] = [];
    let attachmentsForTurn = [...attachments, ...detectedPathAttachments];
    attachmentsForTurn = await prepareVisualAttachmentsForTurn(attachmentsForTurn, queueSessionId);
    let mutationSessionId = "";
    let hydrationLockToken: number | null = null;
    let queuedSuccessfully = false;
    try {
      const parentTurn = activeTurnForSession(options?.sessionId ?? activeSessionId);
      const mutation = await ensureActiveSessionForMutation(options?.sessionId ?? parentTurn?.sessionId);
      mutationSessionId = mutation.sessionId;
      hydrationLockToken = mutation.hydrationLockToken;
      if (!mutationSessionId) {
        throw new Error("Create or select a chat session before queueing.");
      }
      const queuedRouteBinding = routeBindingForDynamicRouting(dynamicRoutingEnabled, route);
      const queuedTurnId = createChatTurnIdentity("turn");
      const queuedGenerationToken = createChatTurnIdentity("generation");
      const buildQueuedContext = () => {
        const attachmentGrants = attachmentsForTurn.map((attachment) => ({
          name: attachment.name,
          mimeType: attachment.mime_type,
          byteCount: attachment.byte_count,
        }));
        return parentTurn
          ? deriveChatTurnContext(parentTurn, "queued", {
              turnId: queuedTurnId,
              generationToken: queuedGenerationToken,
              attachmentGrants,
            })
          : createChatTurnContext({
              turnId: queuedTurnId,
              generationToken: queuedGenerationToken,
              sessionId: mutationSessionId,
              agentId: selectedAgent.id,
              projectId: projectId ?? sessions.find((session) => session.id === mutationSessionId)?.projectId ?? null,
              route: {
                providerId: queuedRouteBinding.providerId,
                modelId: queuedRouteBinding.modelId,
                reasoning: activeReasoningLevel,
                contextBudget: activeContextBudget,
                primaryRouteId: activePrimaryRouteId,
                fallbackRouteId: activeFallbackRouteId,
                dynamicRoutingEnabled,
                automatedWebGroundingEnabled,
              },
              attachmentGrants,
            });
      };
      let queuedContext = buildQueuedContext();
      if (queuedLocalRead) {
        setChatStatusForSession(mutationSessionId, t("chat.status.waiting_approval"));
        attachmentsForTurn = [...attachmentsForTurn, await approvedLocalFileAttachment(queuedLocalRead.path, nextMessage, queuedContext, attachmentsForTurn.length)];
        queuedContext = buildQueuedContext();
      }
      attachmentsForTurn = await attachPrivateDataProvenance(attachmentsForTurn, queuedContext.turnId);
      queuedContext = buildQueuedContext();
      await invoke<QueuedMessageRecord>("queue_message", {
        request: {
          turn_id: queuedContext.turnId,
          generation_token: queuedContext.generationToken,
          parent_turn_id: queuedContext.ancestry.parentTurnId,
          root_turn_id: queuedContext.ancestry.rootTurnId,
          turn_kind: queuedContext.ancestry.kind,
          agent_id: queuedContext.agentId,
          message: queuedLocalRead ? approvedLocalFilePrompt(nextMessage, queuedLocalRead.path) : nextMessage || "Please review the attached file.",
          attachments: attachmentsForTurn,
          session_id: queuedContext.sessionId,
          provider_id: queuedContext.route.providerId,
          model_id: queuedContext.route.modelId,
          reasoning: queuedContext.route.reasoning,
          context: queuedContext.route.contextBudget?.toString(),
          context_budget: queuedContext.route.contextBudget,
          primary_route_id: queuedContext.route.primaryRouteId,
          fallback_route_id: queuedContext.route.fallbackRouteId,
          automated_web_grounding_enabled: queuedContext.route.automatedWebGroundingEnabled,
          dynamic_routing_override: queuedContext.route.dynamicRoutingEnabled,
        },
      });
      persistLegacySessionConfigIfAllowed(persistSessionConfig, sessions, mutationSessionId, queuedContext.route.dynamicRoutingEnabled, route, (queuedContext.route.reasoning ?? "medium") as ReasoningLevel, queuedContext.route.contextBudget?.toString() ?? activeContextBudgetText);
      setComposerResetSignalForSession(mutationSessionId, (value) => value + 1);
      setAttachmentsForSession(mutationSessionId, []);
      await refreshQueuedMessages(mutationSessionId, {
        hydrationLockToken,
      }).catch(() => undefined);
      setChatStatusForSession(mutationSessionId, t("chat.status.message_queued"));
      queuedSuccessfully = true;
    } catch (error) {
      const content = queuedLocalRead ? localCommandFailureText(error, t) : safeErrorMessage(error, "Unable to queue this message.");
      const targetSessionScope = chatSessionStateScope(mutationSessionId || queueSessionId);
      setMessagesForSession(targetSessionScope, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content,
        },
      ]);
      setChatStatusForSession(targetSessionScope, t("chat.status.queue_blocked"));
    } finally {
      if (queuedSuccessfully) {
        setAttachmentsForSession(chatSessionStateScope(mutationSessionId || queueSessionId), []);
        releaseAttachmentPayloads(attachmentsForTurn);
      } else {
        releaseAttachmentPayloads(detectedPathAttachments);
      }
      if (mutationSessionId && hydrationLockToken !== null) {
        unlockSessionHydration(mutationSessionId, hydrationLockToken);
      }
      if (queuedSuccessfully && queuedDuringTurn && mutationSessionId && !activeTurnForSession(mutationSessionId) && !executingQueueSessionsRef.current.has(mutationSessionId)) {
        void drainQueueAfterTurn(mutationSessionId);
      }
    }
    return queuedSuccessfully;
  }
  function discardPendingSteer(sessionId: string) {
    const pendingSteer = pendingSteersRef.current.get(sessionId);
    if (pendingSteer) {
      if (pendingSteer.userMessageId !== null) {
        setMessagesForSession(sessionId, (current) => current.filter((entry) => entry.id !== pendingSteer.userMessageId));
      }
      releaseAttachmentPayloads(pendingSteer.attachments);
      pendingSteersRef.current.delete(sessionId);
    }
  }

  function pendingSteerSupersedesTurn(context: ChatTurnContext) {
    const pendingSteer = pendingSteersRef.current.get(context.sessionId);
    return pendingSteer?.turnContext.ancestry.parentTurnId === context.turnId;
  }

  function takePendingSteer(sessionId: string) {
    const pendingSteer = pendingSteersRef.current.get(sessionId) ?? null;
    pendingSteersRef.current.delete(sessionId);
    return pendingSteer;
  }

  function continueAfterTurn(sessionId: string) {
    const pendingSteer = takePendingSteer(sessionId);
    if (pendingSteer) {
      void runSteeredContinuation(pendingSteer);
      return;
    }
    void drainQueueAfterTurn(sessionId);
  }

  async function runSteeredContinuation(pendingSteer: PendingSteerTurn) {
    const turnContext = pendingSteer.turnContext;
    const outstandingNativeEffect = pendingSteer.outstandingNativeEffect ?? null;
    const steeredPendingPostcondition = requiresPendingNativePostcondition(pendingSteer.executableActionExpected || outstandingNativeEffect !== null, outstandingNativeEffect === null && pendingSteer.verifiedNativeExecutionReceipt);
    const steeredBrowserDirectiveGrants = browserDirectiveGrantsForMessage(installedMods, pendingSteer.message, activeBrowserRoute);
    const hydrationLockToken = lockSessionHydration(turnContext.sessionId);
    registerActiveTurn(turnContext);
    setIsSendingForSession(turnContext.sessionId, true);
    setIsProcessingForSession(turnContext.sessionId, true);
    setChatStatusForSession(turnContext.sessionId, t("chat.status.applying_steer"));

    const steeredAssistantMessageId = pendingSteer.assistantMessageId ?? nextMessageIdRef.current++;
    setMessagesForSession(turnContext.sessionId, (current) => {
      const withoutInterruptedAssistant = current.filter((entry) => entry.id !== steeredAssistantMessageId);
      const withVisibleSteer =
        pendingSteer.userMessageId === null || withoutInterruptedAssistant.some((entry) => entry.id === pendingSteer.userMessageId)
          ? withoutInterruptedAssistant
          : [
              ...withoutInterruptedAssistant,
              {
                id: pendingSteer.userMessageId,
                role: "user" as const,
                content: messageWithAttachmentReceipt(pendingSteer.message, pendingSteer.attachments),
              },
            ];
      return [
        ...withVisibleSteer,
        {
          id: steeredAssistantMessageId,
          role: "assistant",
          content: "",
          isPending: true,
        },
      ];
    });
    activeAssistantMessageIdsRef.current.set(turnContext.sessionId, steeredAssistantMessageId);

    const streamId = crypto.randomUUID();
    registerActiveTurn(turnContext, streamId);
    const streamController = createProjectedChatStreamController({
      streamId,
      turn: turnContext,
      ownsTurn: () => turnIsCurrent(turnContext) && !pendingSteerSupersedesTurn(turnContext),
      requiresNativeReceipt: steeredPendingPostcondition,
      assistantMessageId: steeredAssistantMessageId,
      updateMessages: (update: (messages: ChatTranscriptMessage[]) => ChatTranscriptMessage[]) => updateTurnMessages(turnContext, update),
      directiveSessionId: pendingSteer.sessionId,
      directiveGrants: steeredBrowserDirectiveGrants,
      activateDirective: activateAuthorizedBrowserDirective,
      activateRoute: activateBrowserSplitRoute,
      onFirstToken: () =>
        updateTurnStatus(
          turnContext,
          t("chat.status.steering_model", {
            model: modelLabel(configuredProviders, turnContext.route.providerId, turnContext.route.modelId),
          }),
        ),
    });

    try {
      await streamController.listen();

      let response: ChatTurnResponse;
      let projectCloudConfirmed = false;
      for (;;) {
        try {
          response = await invoke<ChatTurnResponse>("chat_turn", {
            request: {
              turn_id: turnContext.turnId,
              generation_token: turnContext.generationToken,
              parent_turn_id: turnContext.ancestry.parentTurnId,
              root_turn_id: turnContext.ancestry.rootTurnId,
              turn_kind: turnContext.ancestry.kind,
              agent_id: turnContext.agentId,
              message: pendingSteer.message,
              attachments: pendingSteer.attachments,
              session_id: turnContext.sessionId,
              provider_id: turnContext.route.providerId,
              model_id: turnContext.route.modelId,
              stream_id: streamId,
              reasoning: turnContext.route.reasoning,
              context: turnContext.route.contextBudget?.toString(),
              context_budget: turnContext.route.contextBudget,
              primary_route_id: turnContext.route.primaryRouteId,
              fallback_route_id: turnContext.route.fallbackRouteId,
              steering: pendingSteer.message || "Apply the attached steering context.",
              steering_only: true,
              persist_steering_message: pendingSteer.userMessageId !== null,
              verified_native_execution_receipt: pendingSteer.verifiedNativeExecutionReceipt,
              native_execution_receipt_id: pendingSteer.nativeExecutionReceiptId ?? null,
              automated_web_grounding_enabled: turnContext.route.automatedWebGroundingEnabled,
              dynamic_routing_override: turnContext.route.dynamicRoutingEnabled,
              mcp_tool_capabilities: pendingSteer.mcpToolCapabilities,
              project_cloud_confirmed: projectCloudConfirmed,
            },
          });
          break;
        } catch (error) {
          const consent = await resolveChatCloudConsentBoundary({
            error,
            turn: turnContext,
            projectId: turnContext.projectId,
            projectDestination: modelLabel(configuredProviders, pendingSteer.providerId, pendingSteer.modelId) || t("chat.project_cloud_consent.configured_destination"),
            privateDestination: (providerId, modelId) => modelLabel(configuredProviders, providerId, modelId) || modelId,
            requestProjectConsent: cloudConsent.requestProjectCloudConsent,
            requestPrivateConsent: cloudConsent.requestPrivateEgressConsent,
          });
          if (consent === null) throw error;
          projectCloudConfirmed = consent;
        }
      }
      await streamController.awaitValidatedDrain(response.text);
      if (!turnIsCurrent(turnContext) || pendingSteerSupersedesTurn(turnContext) || !chatStreamResponseMatches(turnContext, response)) {
        return;
      }
      const { metadata: responseMetadata, text: sanitizedResponseText } = localizedAssistantResponse(response.text, response.metadata, turnContext.route.providerId, turnContext.route.modelId, t);
      activateAuthorizedBrowserDirective(sanitizedResponseText, steeredAssistantMessageId, response.session_id ?? pendingSteer.sessionId, steeredBrowserDirectiveGrants, activateBrowserSplitRoute);
      const { searchRequest: searchContinuationRequest, mcpRequest: mcpToolRequest, displayText: assistantDisplayText } = assistantControlProjection(sanitizedResponseText, t, Boolean(pendingSteer.searchContinuationState));
      const terminalAfterResponse = pendingSteer.terminalAfterResponse === true;
      const permittedSearchContinuationRequest = terminalAfterResponse ? null : searchContinuationRequest;
      const permittedMcpToolRequest = !terminalAfterResponse && mcpToolRequest && conversationalMcpToolIsAvailable(mcpToolRequest.call, pendingSteer.mcpToolCapabilities) ? mcpToolRequest : null;
      const unverifiedActionClaim = !permittedMcpToolRequest && !permittedSearchContinuationRequest && shouldBlockOutstandingNativeEffectClaim(assistantDisplayText, pendingSteer.message, pendingSteer.executableActionExpected, outstandingNativeEffect, responseMetadata?.verifiedNativeExecutionReceipt === true || pendingSteer.verifiedNativeExecutionReceipt);
      updateTurnMessages(turnContext, (current) =>
        current.map((entry) =>
          entry.id === steeredAssistantMessageId
            ? {
                ...entry,
                role: unverifiedActionClaim ? "system" : entry.role,
                content: unverifiedActionClaim ? t("trust.unverified_action_claim") : assistantDisplayText,
                providerId: responseMetadata?.executingProviderId ?? turnContext.route.providerId,
                modelId: responseMetadata?.executingModelId ?? turnContext.route.modelId,
                metadata: responseMetadata,
                isPending: false,
              }
            : entry,
        ),
      );
      const responseSessionId = response.session_id ?? pendingSteer.sessionId;
      void invoke<ChatSession[]>("list_chat_sessions")
        .then(onSessionsChange)
        .catch(() => undefined);
      if (permittedSearchContinuationRequest && pendingSteer.searchContinuationState) {
        await handleSearchContinuationRequest(permittedSearchContinuationRequest, searchContinuationTurnContext(turnContext, steeredAssistantMessageId, pendingSteer.searchContinuationState, pendingSteer.mcpToolCapabilities, pendingSteer.toolLoopDepth, pendingSteer.context));
      } else if (permittedMcpToolRequest) {
        await handleConversationalMcpToolRequest(permittedMcpToolRequest, {
          turnContext,
          sessionId: responseSessionId,
          agentId: pendingSteer.agentId,
          providerId: pendingSteer.providerId,
          modelId: pendingSteer.modelId,
          reasoning: pendingSteer.reasoning,
          context: pendingSteer.context,
          contextBudget: pendingSteer.contextBudget,
          primaryRouteId: pendingSteer.primaryRouteId,
          fallbackRouteId: pendingSteer.fallbackRouteId,
          automatedWebGroundingEnabled: pendingSteer.automatedWebGroundingEnabled,
          capabilities: pendingSteer.mcpToolCapabilities,
          toolLoopDepth: pendingSteer.toolLoopDepth,
          outstandingNativeEffect: pendingSteer.outstandingNativeEffect ?? null,
        });
      } else {
        await refreshSessionMessages(responseSessionId, {
          hydrationLockToken,
        }).catch(() => undefined);
        compactSessionHistory(responseSessionId, pendingSteer.agentId);
        updateTurnStatus(turnContext, t("chat.status.ready"));
        await publishBackgroundCompletionAttention(responseSessionId, turnContext.turnId);
      }
    } catch (error) {
      if (!turnIsCurrent(turnContext)) {
        if (!cancelledGenerationTokensRef.current.has(turnContext.generationToken)) {
          await abandonAcceptedTurnAfterOwnershipLoss(turnContext, hydrationLockToken);
        }
        return;
      }
      if (pendingSteerSupersedesTurn(turnContext)) {
        await abandonAcceptedTurnAfterOwnershipLoss(turnContext, hydrationLockToken);
        return;
      }
      const errorCode = stableErrorCode(error);
      if (errorCode === "chat_turn_already_running") {
        updateTurnMessages(turnContext, (current) =>
          current.map((entry) =>
            entry.id === steeredAssistantMessageId
              ? {
                  ...entry,
                  content: t("chat.errors.turn_in_progress.content"),
                  isPending: false,
                }
              : entry,
          ),
        );
        updateTurnStatus(turnContext, t("chat.errors.turn_in_progress.status"));
        const reconciliation = await reconcileTerminalChatTurn(turnContext, hydrationLockToken, () => turnIsCurrent(turnContext) && !pendingSteerSupersedesTurn(turnContext));
        if (reconciliation === "terminal") {
          compactSessionHistory(turnContext.sessionId, pendingSteer.agentId);
        } else if (reconciliation === "timed_out" && turnIsCurrent(turnContext)) {
          updateTurnMessages(turnContext, (current) =>
            current.map((entry) =>
              entry.id === steeredAssistantMessageId
                ? {
                    ...entry,
                    content: t("chat.errors.turn_delayed.content"),
                    isPending: false,
                  }
                : entry,
            ),
          );
          updateTurnStatus(turnContext, t("chat.errors.turn_delayed.status"));
        }
        return;
      }
      if (errorCode === "private_egress_user_denied") {
        const keptPrivateContent = t("chat.private_egress_consent.kept_private_content");
        await finalizeDurableChatTurn(turnContext, {
          role: "system",
          content: keptPrivateContent,
          status: "cancelled",
        }).catch(() => undefined);
        updateTurnMessages(turnContext, (current) => [
          ...current.filter((entry) => entry.id !== steeredAssistantMessageId),
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: keptPrivateContent,
          },
        ]);
        updateTurnStatus(turnContext, t("chat.private_egress_consent.kept_private_status"));
        return;
      }
      if (await surfaceStoppedTurn(errorCode, turnContext, hydrationLockToken, steeredAssistantMessageId)) return;
      const notice = chatFailureNotice(error, t);
      await finalizeDurableChatTurn(turnContext, {
        role: "system",
        content: notice.content,
        status: "failed",
      }).catch(() => undefined);
      updateTurnMessages(turnContext, (current) => [
        ...current.filter((entry) => entry.id !== steeredAssistantMessageId),
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content: notice.content,
        },
      ]);
      updateTurnStatus(turnContext, notice.status);
    } finally {
      streamController.teardown();
      const wasCancelled = cancelledGenerationTokensRef.current.delete(turnContext.generationToken);
      clearActiveTurn(turnContext);
      unlockSessionHydration(turnContext.sessionId, hydrationLockToken);
      releaseAttachmentPayloads(pendingSteer.attachments);
      if (!wasCancelled) {
        continueAfterTurn(pendingSteer.sessionId);
      }
    }
  }
  async function handleExecuteQueuedMessages(options?: { sessionId?: string; count?: number }) {
    const sessionId = options?.sessionId ?? activeSessionId;
    const count = options?.count ?? queuedMessages.length;
    if (!sessionId || executingQueueSessionsRef.current.has(sessionId) || count === 0 || localModelIsHydrating) {
      if (localModelIsHydrating) {
        setChatStatus(t("chat.status.model_hydrating"));
      }
      return;
    }
    const hydrationLockToken = lockSessionHydration(sessionId);
    executingQueueSessionsRef.current.add(sessionId);
    setIsQueueExecutingForSession(sessionId, true);
    setChatStatusForSession(sessionId, count === 1 ? t("chat.status.running_queued_one") : t("chat.status.running_queued_many", { count }));
    try {
      const results = await invoke<QueuedMessageExecutionRecord[]>("execute_queued_messages", {
        request: {
          session_id: sessionId,
          limit: count,
        },
      });
      await refreshSessionMessages(sessionId, { hydrationLockToken }).catch(() => undefined);
      await refreshQueuedMessages(sessionId, { hydrationLockToken }).catch(() => undefined);
      if (results.some((result) => result.status === "completed")) {
        const sessionAgentId = sessions.find((session) => session.id === sessionId)?.agentId;
        if (sessionAgentId) {
          compactSessionHistory(sessionId, sessionAgentId);
        }
      }
      void invoke<ChatSession[]>("list_chat_sessions")
        .then(onSessionsChange)
        .catch(() => undefined);
      const failedCount = results.filter((result) => result.status !== "completed").length;
      setChatStatusForSession(sessionId, failedCount ? (failedCount === 1 ? t("chat.status.queued_failed_one") : t("chat.status.queued_failed_many", { count: failedCount })) : t("chat.status.queue_complete"));
    } catch (error) {
      const content = safeErrorMessage(error, "Unable to execute queued messages.");
      setMessagesForSession(sessionId, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content,
        },
      ]);
      setChatStatusForSession(sessionId, t("chat.status.queue_blocked"));
      await refreshQueuedMessages(sessionId, { hydrationLockToken }).catch(() => undefined);
    } finally {
      unlockSessionHydration(sessionId, hydrationLockToken);
      executingQueueSessionsRef.current.delete(sessionId);
      setIsQueueExecutingForSession(sessionId, false);
    }
  }
  async function drainQueueAfterTurn(sessionId: string) {
    if (!sessionId) {
      return;
    }
    try {
      const queued = await refreshQueuedMessages(sessionId);
      if (queued.length === 0) {
        return;
      }
      await handleExecuteQueuedMessages({ sessionId, count: queued.length });
    } catch {
      void 0;
    }
  }

  async function handleSteerNow(nextMessageValue: string) {
    if (!isSending || localModelIsHydrating) {
      if (localModelIsHydrating) {
        setChatStatus(t("chat.status.model_hydrating"));
      }
      return;
    }
    const steerMessage = nextMessageValue.trim();
    if ((!steerMessage && attachments.length === 0) || !selectedAgent || isReadingAttachments || !route.modelId) {
      return;
    }

    const parentTurn = activeTurnForSession(activeSessionId);
    if (!parentTurn) {
      setChatStatus(t("chat.status.steer_needs_session"));
      return;
    }
    const sessionId = parentTurn.sessionId;
    setBypassNotice(null);

    const steerLocalIntent = detectDirectLocalCommand(steerMessage);
    const steerLocalRead = steerLocalIntent?.kind === "read" ? steerLocalIntent : null;
    const detectedPathAttachments: ChatAttachment[] = [];
    let attachmentsForTurn = [...attachments, ...detectedPathAttachments];
    if (attachmentsForTurn.some(shouldAnalyzeVisualChatAttachment)) {
      attachmentsForTurn = await prepareVisualAttachmentsForTurn(attachmentsForTurn, sessionId);
    }
    const steerTurnId = createChatTurnIdentity("turn");
    const steerGenerationToken = createChatTurnIdentity("generation");
    const buildSteerContext = () =>
      deriveChatTurnContext(parentTurn, "steer", {
        turnId: steerTurnId,
        generationToken: steerGenerationToken,
        attachmentGrants: attachmentsForTurn.map((attachment) => ({
          name: attachment.name,
          mimeType: attachment.mime_type,
          byteCount: attachment.byte_count,
        })),
      });
    let steerContext = buildSteerContext();
    if (steerLocalRead) {
      try {
        setChatStatusForSession(sessionId, t("chat.status.waiting_approval"));
        attachmentsForTurn = [...attachmentsForTurn, await approvedLocalFileAttachment(steerLocalRead.path, steerMessage, steerContext, attachmentsForTurn.length)];
        steerContext = buildSteerContext();
      } catch (error) {
        setMessagesForSession(sessionId, (current) => [
          ...current,
          {
            id: nextMessageIdRef.current++,
            role: "system",
            content: localCommandFailureText(error, t),
          },
        ]);
        setChatStatusForSession(sessionId, t("chat.status.attachment_blocked"));
        releaseAttachmentPayloads(attachmentsForTurn);
        return;
      }
    }
    attachmentsForTurn = await attachPrivateDataProvenance(attachmentsForTurn, steerContext.turnId);
    steerContext = buildSteerContext();
    try {
      await acceptDurableChatTurn(steerContext, steerMessage);
    } catch {
      releaseAttachmentPayloads(detectedPathAttachments);
      setChatStatusForSession(sessionId, t("chat.status.something_wrong"));
      return false;
    }
    const assistantMessageId = activeAssistantMessageIdsRef.current.get(sessionId) ?? null;
    const userMessageId = nextMessageIdRef.current++;
    discardPendingSteer(sessionId);
    pendingSteersRef.current.set(sessionId, {
      turnContext: steerContext,
      sessionId,
      agentId: steerContext.agentId,
      userMessageId,
      message: steerLocalRead ? approvedLocalFilePrompt(steerMessage, steerLocalRead.path) : steerMessage,
      attachments: attachmentsForTurn,
      providerId: steerContext.route.providerId,
      modelId: steerContext.route.modelId,
      reasoning: steerContext.route.reasoning,
      context: steerContext.route.contextBudget?.toString(),
      contextBudget: steerContext.route.contextBudget,
      primaryRouteId: steerContext.route.primaryRouteId,
      fallbackRouteId: steerContext.route.fallbackRouteId,
      automatedWebGroundingEnabled: steerContext.route.automatedWebGroundingEnabled,
      assistantMessageId,
      mcpToolCapabilities: conversationalMcpCapabilities,
      toolLoopDepth: 0,
      executableActionExpected: hasLikelyLocalNativeTaskIntent(steerMessage) || hasExplicitBrowserNavigationIntent(steerMessage),
      verifiedNativeExecutionReceipt: false,
    });
    const optimisticSteerMessage: ChatTranscriptMessage = {
      id: userMessageId,
      role: "user",
      content: messageWithAttachmentReceipt(steerMessage, attachmentsForTurn),
    };
    setMessagesForSession(sessionId, (current) => {
      const withoutInterruptedAssistant = assistantMessageId === null ? current : current.filter((entry) => entry.id !== assistantMessageId);
      const nextMessages = [...withoutInterruptedAssistant, optimisticSteerMessage];
      if (assistantMessageId !== null) {
        nextMessages.push({
          id: assistantMessageId,
          role: "assistant",
          content: "",
          isPending: true,
        });
      }
      return nextMessages;
    });
    setComposerDraftForSession(sessionId, "");
    setComposerResetSignalForSession(sessionId, (value) => value + 1);
    setAttachmentsForSession(sessionId, []);
    setChatStatusForSession(sessionId, t("chat.status.steering_response"));

    const streamId = activeStreamIdsRef.current.get(sessionId);
    if (streamId) {
      handleStopGeneration(streamId, sessionId);
    }
    return true;
  }

  const handleStopGeneration = (streamId = activeStreamIdsRef.current.get(activeSessionId), sessionId = activeSessionId, invalidateGeneration = false, surfaceCancellation = false) => {
    attachmentReadAbortRef.current?.abort();
    attachmentReadAbortRef.current = null;
    void mcp?.cancelRemoteOperations();
    if (!streamId) return;
    setChatStatusForSession(sessionId, t("chat.status.stopping_generation"));
    if (invalidateGeneration) {
      const context = activeTurnForSession(sessionId);
      if (context) {
        cancelledGenerationTokensRef.current.add(context.generationToken);
        if (surfaceCancellation) {
          const assistantMessageId = activeAssistantMessageIdsRef.current.get(sessionId) ?? null;
          setMessagesForSession(sessionId, (current) => visibleCancelledTurnMessages(current, assistantMessageId, context.turnId, "local_inference_cancelled", t("tasks.error_cancelled"), () => nextMessageIdRef.current++));
          setChatStatusForSession(sessionId, t("chat.status.generation_stopped"));
        }
      }
      activeTurnsRef.current.delete(sessionId);
      activeStreamIdsRef.current.delete(sessionId);
      activeAssistantMessageIdsRef.current.delete(sessionId);
      discardPendingSteer(sessionId);
      setActiveStreamIdForSession(sessionId, null);
      setIsSendingForSession(sessionId, false);
      setIsProcessingForSession(sessionId, false);
    }
    void invoke<boolean>("cancel_chat_stream", { streamId });
  };

  async function handleWebGroundingToggle() {
    if (!selectedAgent || isSavingWebGroundingOverride) {
      return;
    }

    const nextEnabled = !automatedWebGroundingEnabled;
    let mutationSessionId = "";
    let hydrationLockToken: number | null = null;
    setIsSavingWebGroundingOverride(true);
    try {
      const mutation = activeSessionId
        ? {
            sessionId: activeSessionId,
            hydrationLockToken: lockSessionHydration(activeSessionId),
          }
        : await ensureActiveSessionForMutation();
      mutationSessionId = mutation.sessionId;
      hydrationLockToken = mutation.hydrationLockToken;
      if (!mutationSessionId) {
        throw new Error("Create or select a chat session before changing search grounding.");
      }
      const updatedSession = await invoke<ChatSession>("update_chat_session_web_grounding_override", {
        sessionId: mutationSessionId,
        session_id: mutationSessionId,
        webGroundingOverride: nextEnabled,
        web_grounding_override: nextEnabled,
      });
      onSessionsChange([updatedSession, ...sessions.filter((session) => session.id !== updatedSession.id)]);
      setChatStatus(nextEnabled ? t("chat.status.web_grounding_on") : t("chat.status.web_grounding_off"));
    } catch (error) {
      setMessages((current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content: toolErrorMessage(error),
        },
      ]);
      setChatStatus(t("chat.status.search_setting_blocked"));
    } finally {
      if (mutationSessionId && hydrationLockToken !== null) {
        unlockSessionHydration(mutationSessionId, hydrationLockToken);
      }
      setIsSavingWebGroundingOverride(false);
    }
  }

  async function ensureActiveSessionForMutation(preferredSessionId?: string | null, persistCreatedRoute = true) {
    const existingSessionId = preferredSessionId?.trim() || activeSessionId;
    if (existingSessionId) {
      return {
        sessionId: existingSessionId,
        hydrationLockToken: lockSessionHydration(existingSessionId),
      };
    }
    if (!selectedAgent) {
      return {
        sessionId: "",
        hydrationLockToken: null,
      };
    }
    const nextRoute = selectedAgentDefaultRoute;
    const session = await createSessionInContext(selectedAgent.id, routeBindingForDynamicRouting(dynamicRoutingDefaultForAgent(selectedAgent), nextRoute));
    if (!session) {
      return {
        sessionId: "",
        hydrationLockToken: null,
      };
    }
    setRouteOverrides((current) => ({
      ...current,
      [session.id]: nextRoute,
    }));
    if (persistCreatedRoute && !sessionUsesDynamicBinding(session)) {
      void persistSessionConfig(session.id, nextRoute);
    }
    const hydrationLockToken = lockSessionHydration(session.id);
    onSelectSession(session.id);
    return {
      sessionId: session.id,
      hydrationLockToken,
    };
  }

  async function executeAgentPlan(pendingExecution: PendingActionPlan, principalApproved: boolean) {
    const requestedSessionId = pendingExecution.turnContext.sessionId;
    if (!selectedAgent || isExecutingPlan || executionStartRequestsRef.current.has(requestedSessionId)) {
      return;
    }
    const plan = pendingExecution.plan;
    const turnContext = pendingExecution.turnContext;
    const sessionId = turnContext.sessionId;
    if (!sessionId || sessionId !== activeSessionId || turnContext.agentId !== selectedAgent.id) {
      return;
    }
    executionStartRequestsRef.current.add(sessionId);
    const hydrationLockToken = lockSessionHydration(sessionId);
    const ownershipVersion = sessionHydrationVersionsRef.current.get(sessionId) ?? 0;
    const executionStillOwned = () => (sessionHydrationVersionsRef.current.get(sessionId) ?? 0) === ownershipVersion;
    let executionStarted = false;
    setIsExecutingPlanForSession(sessionId, true);
    setChatStatusForSession(sessionId, t("chat.status.starting_background"));
    try {
      const executionRequest = {
        plan,
        turn_context: agentPlanTurnContextRequest(turnContext),
        principal_approved: principalApproved,
      };
      const authority = await invoke<AgentPlanAuthorityResponse>("request_agent_plan_authority", {
        request: {
          request: executionRequest,
          locale: language,
        },
      });
      const startResponse = await invoke<AgentExecutionStartResponse>("spawn_agent_execution", {
        request: {
          ...executionRequest,
          authority_proof_id: authority.authorityProofId ?? null,
        },
      });
      if (!executionStillOwned()) {
        return;
      }
      const executionId = executionIdFromStartResponse(startResponse);
      if (!executionId) {
        throw new Error("Native runtime did not return an execution ID.");
      }
      const responseSessionId = sessionIdFromStartResponse(startResponse, sessionId);
      const responsePlanId = planIdFromStartResponse(startResponse, plan.id);
      if (responseSessionId !== sessionId || responsePlanId !== plan.id) {
        throw new Error("Native runtime returned execution ownership for another session or plan.");
      }
      const execution: ActiveAgentExecution = {
        executionId,
        planId: responsePlanId,
        sessionId: responseSessionId,
        status: "running",
        logs: [],
        lastSeenId: streamStartAfterLogIdFromResponse(startResponse),
        streamStartAfterLogId: streamStartAfterLogIdFromResponse(startResponse),
        startedAtMs: Date.now(),
      };
      executionStarted = true;
      persistActiveExecution(execution);
      setActiveExecutionForSession(sessionId, execution);
      setPendingPlanForSession(sessionId, null);
      setChatStatusForSession(sessionId, t("gateway.auto_turn.retrieving"));
    } catch (error) {
      if (!executionStillOwned()) {
        return;
      }
      const errorCode = stableErrorCode(error);
      const rawMessage = ["permission_denied", "permission_request"].includes(chatErrorGroup(errorCode, safeErrorMessage(error, ""))) ? chatFailureNotice(error, t).content : safeErrorMessage(error, "The plan stopped before it finished.");
      const originStale = errorCode === "agent_execution_origin_stale";
      const alreadyStarted = errorCode === "agent_execution_already_started";
      const errorMsg = originStale || alreadyStarted ? t(originStale ? "chat.errors.origin_stale.content" : "chat.errors.turn_in_progress.content") : [errorCode ? `(${errorCode})` : "", rawMessage].filter(Boolean).join(" ");
      const isHalted = errorCode === "local_workflow_decision_halted" || errorMsg.includes("local_workflow_decision_halted") || errorMsg.includes("Local Gemma halted");
      const boundaryMessage = errorMsg.replace("(local_workflow_decision_halted)", "").trim();
      if (originStale || alreadyStarted) {
        setPendingPlanForSession(sessionId, null);
      }
      setMessagesForSession(sessionId, (current) => [
        ...current,
        {
          id: nextMessageIdRef.current++,
          role: "system",
          content: isHalted ? `Execution Boundary: ${boundaryMessage || "Local verifier halted execution."}` : errorMsg,
        },
      ]);
      setChatStatusForSession(sessionId, originStale || alreadyStarted ? t(originStale ? "chat.errors.origin_stale.status" : "chat.errors.turn_in_progress.status") : isHalted ? t("chat.status.halted") : t("chat.status.something_wrong"));
      void refreshSessionMessages(sessionId, { hydrationLockToken }).catch(() => undefined);
    } finally {
      executionStartRequestsRef.current.delete(sessionId);
      unlockSessionHydration(sessionId, hydrationLockToken);
      if (!executionStarted && executionStillOwned()) {
        setIsExecutingPlanForSession(sessionId, false);
      }
    }
  }

  async function handleExecutePendingPlan() {
    if (!pendingPlan) return;
    await executeAgentPlan(pendingPlan, true);
  }

  const recoveryReceiptActions = useAgentExecutionRecoveryHandlers({
    activeSessionId,
    setActiveExecution: setActiveExecutionForSession,
    setCompletedActions: setCompletedRecoveryActionKeysForSession,
    setExecuting: setIsExecutingPlanForSession,
    setProcessing: setIsProcessingForSession,
    setStatus: setChatStatusForSession,
    translate: t,
  });
  const handleResolveCalendarRecovery = recoveryReceiptActions.onResolveCalendar;

  const handleComposerAttachmentRequest = useStableEvent(() => {
    void handleAttachmentRequest();
  });
  const handleComposerAttachmentDrop = useStableEvent((dropId: string) => {
    void handleDroppedAttachment(dropId);
  });
  const handleComposerRemoveAttachment = useCallback(
    (index: number) => {
      setAttachments((current) => current.filter((_, itemIndex) => itemIndex !== index));
    },
    [setAttachments],
  );
  const handleComposerExecuteQueuedMessages = useStableEvent(() => {
    void handleExecuteQueuedMessages();
  });
  const handleComposerSubmit = useStableEvent((nextMessage: string) => {
    return new Promise<ChatSubmissionOutcome>((resolve) => {
      let accepted = false;
      void handleSubmit(nextMessage, {
        onAccepted: () => {
          accepted = true;
          resolve(ACCEPTED_CHAT_SUBMISSION);
        },
      })
        .then(() => {
          if (!accepted) resolve(REJECTED_CHAT_SUBMISSION);
        })
        .catch(() => resolve(REJECTED_CHAT_SUBMISSION));
    });
  });
  const handleStartNewAgentPlan = useStableEvent(async (executionId: string) => {
    const sessionId = activeSessionId.trim();
    await startNewAgentRecoveryPlan({
      executionId,
      sessionId,
      currentSessionId: () => activeSessionId,
      submit: handleSubmit,
    });
    setCompletedRecoveryActionKeysForSession(sessionId, (current) => new Set(current).add(agentRecoveryActionKey(executionId, "start_new_plan")));
  });
  const handleComposerQueueMessage = useStableEvent(async (nextMessage: string) => {
    return (await handleQueueMessage(nextMessage)) ? ACCEPTED_CHAT_SUBMISSION : REJECTED_CHAT_SUBMISSION;
  });
  const handleComposerCompactSession = useStableEvent(() => sessionContextController.compactNow());
  const handleComposerSteerNow = useStableEvent((nextMessage: string) => {
    return handleSteerNow(nextMessage).then((accepted) => (accepted ? ACCEPTED_CHAT_SUBMISSION : REJECTED_CHAT_SUBMISSION));
  });
  const handleComposerStopGeneration = useStableEvent(() => {
    handleStopGeneration(undefined, activeSessionId, true, true);
  });
  const handleComposerWebGroundingToggle = useStableEvent(() => {
    void handleWebGroundingToggle();
  });
  const handleComposerDynamicRoutingToggle = useStableEvent(() => {
    void handleDynamicRoutingToggle();
  });
  const handleComposerToggleSendMenu = useCallback(() => {
    setIsSendMenuOpen((current) => !current);
  }, []);
  const handleComposerCloseSendMenu = useCallback(() => {
    setIsSendMenuOpen(false);
  }, []);

  return (
    <section className="flex h-full min-h-0 bg-[var(--background)] text-[var(--foreground)]" ref={chatRootRef}>
      <ChatSessionsSidebar
        activeSessionId={activeSessionId}
        agentById={agentById}
        canCreateSession={Boolean(selectedAgent && route.modelId)}
        editingSessionId={editingSessionId}
        editingSessionTitle={editingSessionTitle}
        isProcessingForSession={isProcessingForSession}
        isRenamingSession={isRenamingSession}
        onAbortRename={abortRenameSession}
        onBeginRename={beginRenameSession}
        onCommitRename={commitRenameSession}
        onCreateSession={handleNewChat}
        onDeleteSession={handleDeleteChatSession}
        onEditingTitleChange={setEditingSessionTitle}
        onSelectSession={onSelectSession}
        onStartGlobalChat={onStartGlobalChat}
        projectId={projectId} projectName={projectName}
        sessions={sessions}
        skipRenameCommitRef={skipRenameCommitRef}
        width={fittedPanels.sessions}
      />

      <ResizeHandle label={t("chat.resize_list")} panel={sessionsPanel} value={fittedPanels.sessions} />

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <ChatWorkspaceHeader
          activeAgentId={activeAgentId}
          agents={agents}
          hasSelectedAgent={Boolean(selectedAgent)}
          hasSplitPanelContent={hasSplitPanelContent}
          isDrawerOpen={isDrawerOpen}
          isSplitPanelOpen={isSplitPanelOpen}
          onAgentChange={handleAgentChange}
          onManageAgents={onManageAgents}
          onOpenDocuments={onOpenDocuments}
          onOpenTasks={onOpenTasks}
          onToggleSplit={() => {
            if (!activeSplitRouteIdentity) return;
            if (isSplitPanelOpen) dismissSplitRoute(activeSplitRouteIdentity);
            else restoreSplitRoute(activeSplitRouteIdentity);
          }}
          onToggleTuning={() => setIsDrawerOpen(!isDrawerOpen)}
        />

        <div className="flex min-h-0 flex-1 relative overflow-hidden">
          <main className="flex min-h-0 min-w-0 flex-1 flex-col border-r border-transparent transition-all">
            <div className="custom-scrollbar flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-5 py-6" ref={transcriptScrollRef}>
              <ChatEmptyState agentDescription={selectedAgent?.description ?? null} agentName={selectedAgent?.name ?? null} decisionBriefCompletion={decisionBriefCompletion} onStarterAction={onStarterAction} sessionCount={sessions.length} sessionsLoaded={sessionsLoaded} transcriptLoaded={!activeSessionId || transcriptHydrated} transcriptEmpty={messages.length === 0} />

              {messages.map((entry) =>
                verticalTemplateIds.has(entry.id) || splitViewDirectiveIds.has(entry.id) ? null : (
                  <ChatMessageBubble assistantName={selectedAgent?.name ?? t("chat.agent_fallback")} completedRecoveryActionKeys={effectiveRecoveryActionKeys} recoveryReceiptAuthority={recoveryReceiptAuthorities.get(entry.id)} recoveryExecutionStateSnapshot={recoveryExecutionStateSnapshot} onRefreshRecoveryExecutionStates={refreshRecoveryExecutionStates} key={entry.id} message={entry} onStartNewRecoveryPlan={handleStartNewAgentPlan} recoveryActions={recoveryReceiptActions} />
                ),
              )}

              <ChatTurnRecoveryCards activeSessionId={activeSessionId} autoRouteAttention={autoRouteAttention} directApplePermissionActions={directApplePermissionActions} directApplePermissionAttention={directApplePermissionAttention} onAutoRouteChoice={resolveAutoRouteTurnChoice} t={t} />

              {autoRouteActivationFailure?.sessionId === activeSessionId ? <AutoRouteActivationRecoveryCard failure={autoRouteActivationFailure} onChooseModel={onOpenModels} onDismiss={keepAutoRoute} onRetry={() => handleDynamicRoutingToggle(autoRouteActivationFailure.desiredEnabled)} t={t} /> : null}

              <ChatConsentCards activeSessionId={activeSessionId} consent={cloudConsent} t={t} />

              <ChatThinkingIndicator agentName={selectedAgent?.name ?? null} visible={isProcessing && !messages.some((entry) => entry.role === "assistant" && entry.isPending)} />

              {bypassNotice && (
                <div className="relative max-w-3xl self-start rounded-[var(--radius-sm)] border border-[var(--warning)] bg-[var(--warning-background)] py-3 pl-4 pr-10 text-sm text-[var(--foreground)]">
                  <p className="text-xs font-semibold uppercase tracking-wide text-[var(--foreground-muted)]">{bypassNotice.title}</p>
                  <p className="mt-1 leading-6">{bypassNotice.body}</p>
                  <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">{bypassNotice.detail}</p>
                  <button aria-label={t("common.dismiss")} className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]" onClick={() => setBypassNotice(null)} type="button">
                    <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
                      <path d="M6 6l12 12M18 6 6 18" />
                    </svg>
                  </button>
                </div>
              )}

              {pendingPlan && (
                <section className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--border-strong)] bg-[var(--accent-background)] px-5 py-4" data-oomu-plan-preview="true" id="oomu-plan-preview">
                  <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                    <div className="min-w-0">
                      <p className="text-xs font-semibold text-[var(--foreground-muted)]">{t("chat.plan.preview")}</p>
                      <h3 className="mt-2 break-words text-sm font-semibold text-[var(--foreground)]">{pendingPlan.plan.objective}</h3>
                      <p className="mt-2 break-words text-xs leading-5 text-[var(--foreground-muted)]">{pendingPlan.plan.model_route.reason}</p>
                    </div>
                    <button className="rounded-[var(--radius-sm)] inline-flex min-h-10 shrink-0 items-center justify-center bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-40" data-oomu-plan-approval="execute" disabled={isExecutingPlan} id="oomu-plan-approve-execute" onClick={handleExecutePendingPlan} type="button">
                      {isExecutingPlan ? t("chat.plan.executing") : t("chat.plan.approve_execute")}
                    </button>
                  </div>
                  <ol className="mt-4 grid gap-2">
                    {pendingPlan.plan.steps.map((step, index) => (
                      <li className="break-words rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-3 py-2 text-sm leading-6" key={`${pendingPlan.plan.id}-${index}`}>
                        <span className="font-bold">{index + 1}. </span>
                        {actionPlanStepPresentation(step, t)}
                        <span className="ml-2 text-xs font-semibold text-[var(--foreground-subtle)]">
                          {getToolKindLabel(step.tool.kind)} · {getRiskLevelLabel(step.risk_level)}
                        </span>
                      </li>
                    ))}
                  </ol>
                </section>
              )}

              {activeExecution?.sessionId === activeSessionId && <ActiveExecutionProgress execution={activeExecution} onTrackInTasks={onOpenTasks} />}
            </div>

            <ChatComposer
              activeStreamId={activeStreamId}
              attachments={attachments}
              automatedWebGroundingEnabled={automatedWebGroundingEnabled}
              draft={composerDraft}
              dynamicRoutingEnabled={dynamicRoutingEnabled}
              hasRouteModel={Boolean(route.modelId)}
              hasSelectedAgent={Boolean(selectedAgent)}
              isQueueExecuting={isQueueExecuting}
              isReadingAttachments={isReadingAttachments}
              isSavingDynamicRoutingOverride={isSavingDynamicRoutingOverride}
              isSavingWebGroundingOverride={isSavingWebGroundingOverride}
              isSendMenuOpen={isSendMenuOpen}
              isSending={isSending}
              key={`${activeSessionId}:${composerResetSignal}`}
              localModelIsHydrating={localModelIsHydrating}
              canSubmitWhileLocalModelHydrating={(message) => canSubmitLocalToolWorkflowWhileHydrating(message)}
              onAttachmentDrop={handleComposerAttachmentDrop}
              onAttachmentRequest={handleComposerAttachmentRequest}
              onCloseSendMenu={handleComposerCloseSendMenu}
              onCompactSession={handleComposerCompactSession}
              onDynamicRoutingToggle={handleComposerDynamicRoutingToggle}
              onDraftChange={setComposerDraft}
              onExecuteQueuedMessages={handleComposerExecuteQueuedMessages}
              onQueueMessage={handleComposerQueueMessage}
              onRemoveAttachment={handleComposerRemoveAttachment}
              onSteerNow={handleComposerSteerNow}
              onStopGeneration={handleComposerStopGeneration}
              onSubmitMessage={handleComposerSubmit}
              onToggleSendMenu={handleComposerToggleSendMenu}
              onWebGroundingToggle={handleComposerWebGroundingToggle}
              queuedMessageCount={queuedMessages.length}
              routingIndicator={
                <RoutingIndicator
                  activityStatus={isSending ? chatStatus : null}
                  isLocal={routingIndicatorState.isLocal}
                  modelId={routingIndicatorState.modelId}
                  mode={dynamicRoutingEnabled ? "auto" : "manual"}
                  autoRouteStatus={autoRouteSessionReadiness.status}
                  localModelId={autoRouteSessionReadiness.localModelId ?? route.modelId}
                  cloudModelId={autoRouteCloudModelId}
                  classifierModelId={autoRouteSessionReadiness.classifierModelId}
                  readinessGeneration={autoRouteSessionReadiness.readinessGeneration}
                />
              }
              selectedAgentName={selectedAgent?.name ?? null}
              sessionId={activeSessionId}
              slashCommands={availableSlashCommands}
            />
          </main>

          {splitInlineOpen && activeSplitModProvider && (
            <>
              <ResizeHandle label={activeSplitModProvider.resizeLabel} panel={splitPanel} value={fittedPanels.split} />
              <aside aria-label={activeSplitModProvider.label} className="flex h-full shrink-0 flex-col overflow-hidden border-l border-[var(--border-soft)] bg-[var(--accent-background)] animate-in slide-in-from-right-4 duration-200" key={activeSplitModProvider.id} style={{ width: fittedPanels.split }}>
                {activeSplitModProvider.render(fittedPanels.split)}
              </aside>
            </>
          )}

          {isDrawerOpen && (
            <>
              {tuningIsOverlay ? <button aria-label={t("common.close")} className="absolute inset-0 z-20 bg-black/10" onClick={() => setIsDrawerOpen(false)} type="button" /> : <ResizeHandle label={t("chat.drawer.resize_tuning")} panel={tuningPanel} value={fittedPanels.tuning} />}
              <aside
                className={tuningIsOverlay ? "absolute inset-y-0 right-0 z-30 flex flex-col gap-4 overflow-y-auto border-l border-[var(--border-soft)] bg-[var(--accent-background)] p-4 shadow-[var(--shadow-raised)] animate-in slide-in-from-right-4 duration-200" : "flex h-full shrink-0 flex-col gap-4 overflow-y-auto bg-[var(--accent-background)] p-4 animate-in slide-in-from-right-4 duration-200"}
                style={{
                  width: tuningIsOverlay ? overlayTuningWidth : fittedPanels.tuning,
                }}
              >
                <section className="rounded-[var(--radius-md)] bg-[var(--background)] p-4 border border-[var(--border-soft)]">
                  <h2 className="text-xs font-semibold text-[var(--foreground)]">{t("chat.drawer.active_route")}</h2>
                  <div className="mt-4 grid gap-3 text-sm">
                    <label>
                      <span className="text-xs font-medium text-[var(--foreground-subtle)]">{t("chat.drawer.provider")}</span>
                      <select className={`mt-1 w-full appearance-none px-3 py-2 ${fieldClass}`} data-oomu-routing-control="provider" id="oomu-active-route-provider" onChange={(event) => handleProviderChange(event.target.value as RouteProviderId)} value={route.providerId} disabled={tuningControlsDisabled || providerOptions.length === 0}>
                        {providerOptions.length === 0 ? (
                          <option value="">{t("chat.drawer.no_configured_providers")}</option>
                        ) : (
                          providerOptions.map((provider) => (
                            <option key={provider.id} value={provider.id}>
                              {provider.label}
                            </option>
                          ))
                        )}
                      </select>
                    </label>

                    <label>
                      <span className="text-xs font-medium text-[var(--foreground-subtle)]">{t("chat.drawer.model")}</span>
                      {modelOptions.length > 0 ? (
                        <select className={`mt-1 w-full appearance-none px-3 py-2 ${fieldClass}`} data-oomu-routing-control="model" disabled={tuningControlsDisabled} id="oomu-active-route-model" onChange={(event) => handleModelChange(event.target.value)} value={route.modelId}>
                          {modelOptions.map((model) => (
                            <option key={`${model.providerId}-${model.modelId}`} value={model.modelId}>
                              {model.label}
                            </option>
                          ))}
                        </select>
                      ) : (
                        <input className={`mt-1 w-full px-3 py-2 placeholder:text-[var(--foreground-subtle)] ${fieldClass}`} data-oomu-routing-control="model" disabled={tuningControlsDisabled} id="oomu-active-route-model" onChange={(event) => updateRoute({ modelId: event.target.value })} placeholder={t("chat.drawer.model_id_placeholder")} value={route.modelId} />
                      )}
                    </label>

                    <ContextBudgetSlider bounds={activeContextBudgetBounds} currentValue={activeContextBudget} disabled={tuningControlsDisabled} helpText={contextBudgetWarningText(activeContextBudgetBounds, activeContextBudget, t)} label={t("chat.drawer.context_budget")} onChange={(value) => updateRoute({ context: String(value) })} />
                  </div>
                </section>

                <section className="rounded-[var(--radius-md)] bg-[var(--background)] p-4 border border-[var(--border-soft)]">
                  <div className="flex items-center justify-between gap-3">
                    <h2 className="text-xs font-semibold text-[var(--foreground)]">{t("chat.drawer.reasoning")}</h2>
                    {activeReasoningLevels.length > 1 && (
                      <span className="text-[11px] font-medium text-[var(--foreground-subtle)]">
                        {t("chat.drawer.level_count", {
                          count: activeReasoningLevels.length,
                        })}
                      </span>
                    )}
                  </div>
                  {activeReasoningLevels.length <= 1 ? (
                    <div className="mt-3">
                      <span className="inline-flex items-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)]">{t(`chat.drawer.reasoning_levels.${activeReasoningLevel}`)}</span>
                      <p className="mt-2 text-[11px] leading-4 text-[var(--foreground-subtle)]">{t("chat.drawer.reasoning_fixed")}</p>
                    </div>
                  ) : (
                    <>
                      <div aria-label={t("chat.drawer.reasoning_intensity")} aria-disabled={tuningControlsDisabled} className={`mt-3 flex w-full gap-0.5 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-0.5 ${tuningControlsDisabled ? "opacity-60" : ""}`} role="radiogroup">
                        {activeReasoningLevels.map((level) => {
                          const selected = level === activeReasoningLevel;
                          return (
                            <button aria-checked={selected} className={`flex-1 rounded-[var(--radius-xs)] px-2 py-1.5 text-xs font-medium transition-colors ${selected ? "bg-[var(--background)] text-[var(--foreground)] shadow-[var(--shadow-card)]" : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"} disabled:cursor-not-allowed disabled:opacity-50`} disabled={tuningControlsDisabled} key={level} onClick={() => updateRoute({ reasoning: level })} role="radio" type="button">
                              {t(`chat.drawer.reasoning_levels.${level}`)}
                            </button>
                          );
                        })}
                      </div>
                      <p className="mt-2 text-[11px] leading-4 text-[var(--foreground-subtle)]">{t("chat.drawer.reasoning_help")}</p>
                    </>
                  )}
                </section>
                <SessionContextPanel controller={sessionContextController} disabled={tuningControlsDisabled} />
                {isDeveloperBuild && headlessSearchDebug && <WozniakSearchDebug debug={headlessSearchDebug} translate={t} />}

                <section className="rounded-[var(--radius-md)] bg-[var(--background)] p-4 border border-[var(--border-soft)]">
                  <h2 className="text-xs font-semibold text-[var(--foreground)]">{t("chat.drawer.session")}</h2>
                  <div className="mt-3 flex flex-col gap-2.5">
                    <p className="truncate text-sm font-semibold text-[var(--foreground)]">{activeSession?.title ?? t("chat.drawer.unsaved")}</p>
                    <div className="flex flex-wrap items-center gap-2">
                      {pendingPlan ? (
                        <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--warning)] bg-[var(--warning-background)] px-2 py-0.5 text-[11px] font-medium text-[var(--warning)]">{t("chat.drawer.approval")}</span>
                      ) : dynamicRoutingEnabled ? (
                        <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--border-strong)] bg-[var(--background)] px-2 py-0.5 text-[11px] font-medium text-[var(--foreground-muted)]">
                          <AutoRouteGlyph />
                          {t("chat.route.auto")}
                        </span>
                      ) : (
                        <span className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium ${activeRouteUsesLocalModel ? "border-[var(--route-local-border)] bg-[var(--route-local-background)] text-[var(--route-local)]" : "border-[var(--route-cloud-border)] bg-[var(--route-cloud-background)] text-[var(--route-cloud)]"}`}>
                          <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
                          {modelLabel(configuredProviders, route.providerId, route.modelId)}
                        </span>
                      )}
                      {chatStatus !== t("chat.status.ready") && <span className="inline-flex items-center rounded-full border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-0.5 text-[11px] font-medium text-[var(--foreground-muted)]">{chatStatus}</span>}
                    </div>
                    {(attachments.length > 0 || queuedMessages.length > 0) && (
                      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] font-medium text-[var(--foreground-muted)]">
                        {attachments.length > 0 && (
                          <span>
                            {t("chat.drawer.attached_count", {
                              count: attachments.length,
                            })}
                          </span>
                        )}
                        {queuedMessages.length > 0 && (
                          <span>
                            {queuedMessages.length === 1
                              ? t("chat.queued_one")
                              : t("chat.queued_many", {
                                  count: queuedMessages.length,
                                })}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                </section>
              </aside>
            </>
          )}
        </div>
      </div>
    </section>
  );
}
