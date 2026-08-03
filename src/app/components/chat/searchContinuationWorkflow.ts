import {
  createChatTurnIdentity,
  deriveChatTurnContext,
  type ChatTurnContext,
} from "@/lib/chatTurnContext";
import type { LocalSearchOutcome } from "./localSearchContext";
import {
  authorizeSearchContinuation,
  type ParsedSearchContinuationRequest,
  type SearchContinuationState,
} from "./searchContinuationCoordinator";

export type SearchContinuationExecution =
  | { kind: "rejected" }
  | {
      kind: "superseded";
      attachments: Extract<LocalSearchOutcome, { kind: "succeeded" }>["attachments"];
    }
  | { kind: "failed"; errorCode?: string }
  | {
      kind: "ready";
      attachments: Extract<LocalSearchOutcome, { kind: "succeeded" }>["attachments"];
      continuationContext: ChatTurnContext;
      state: SearchContinuationState;
    };

export type SearchContinuationTurnContext<Capability> = {
  turnContext: ChatTurnContext;
  assistantMessageId: number;
  state: SearchContinuationState;
  capabilities: Capability[];
  toolLoopDepth: number;
  context?: string;
};

export function searchContinuationTurnContext<Capability>(
  turnContext: ChatTurnContext,
  assistantMessageId: number,
  state: SearchContinuationState,
  capabilities: Capability[],
  toolLoopDepth: number,
  context?: string,
): SearchContinuationTurnContext<Capability> {
  return { turnContext, assistantMessageId, state, capabilities, toolLoopDepth, context };
}

export type SearchContinuationPendingSteer<Capability, Attachment> = {
  turnContext: ChatTurnContext;
  sessionId: string;
  agentId: string;
  userMessageId: null;
  message: string;
  attachments: Attachment[];
  providerId: string;
  modelId: string;
  reasoning?: string;
  context?: string;
  contextBudget?: number;
  primaryRouteId?: string | null;
  fallbackRouteId?: string | null;
  automatedWebGroundingEnabled: boolean;
  assistantMessageId: number;
  mcpToolCapabilities: Capability[];
  toolLoopDepth: number;
  executableActionExpected: false;
  verifiedNativeExecutionReceipt: true;
  searchContinuationState: SearchContinuationState;
};

type WorkflowInput = {
  request: ParsedSearchContinuationRequest;
  turnContext: ChatTurnContext;
  state: SearchContinuationState;
  isCurrent: () => boolean;
  runSearch: (
    objective: string,
    state: SearchContinuationState,
    options: {
      searchQuery: string;
      targetSessionId: string;
      sources: [{ kind: "user_text" }];
    },
  ) => Promise<LocalSearchOutcome>;
};

export async function executeSearchContinuation(
  input: WorkflowInput,
): Promise<SearchContinuationExecution> {
  if (!input.isCurrent()) return { kind: "superseded", attachments: [] };
  const belongsToRoot = input.turnContext.turnId === input.state.turnId ||
    input.turnContext.ancestry.rootTurnId === input.state.turnId;
  const authorization = authorizeSearchContinuation(
    input.state,
    {
      sessionId: input.state.sessionId,
      turnId: input.state.turnId,
      generationToken: input.state.generationToken,
    },
    input.request.query,
  );
  if (!belongsToRoot || !authorization.allowed) return { kind: "rejected" };
  const outcome = await input.runSearch(input.state.objective, input.state, {
    searchQuery: input.request.query,
    targetSessionId: input.state.sessionId,
    sources: [{ kind: "user_text" }],
  });
  if (!input.isCurrent()) {
    return outcome.kind === "succeeded"
      ? { kind: "superseded", attachments: outcome.attachments }
      : { kind: "superseded", attachments: [] };
  }
  if (outcome.kind !== "succeeded") {
    return {
      kind: "failed",
      errorCode: "errorCode" in outcome ? outcome.errorCode : undefined,
    };
  }
  const continuationContext = deriveChatTurnContext(input.turnContext, "steer", {
    turnId: createChatTurnIdentity("turn"),
    generationToken: createChatTurnIdentity("generation"),
    attachmentGrants: outcome.attachments.map((attachment) => ({
      name: attachment.name,
      mimeType: attachment.mime_type,
      byteCount: attachment.byte_count,
    })),
  });
  return {
    kind: "ready",
    attachments: outcome.attachments,
    continuationContext,
    state: authorization.state,
  };
}

type UiRuntime<Capability, Attachment> = {
  isCurrent: (turnContext: ChatTurnContext) => boolean;
  runSearch: WorkflowInput["runSearch"];
  setStatus: (turnContext: ChatTurnContext, status: string) => void;
  setFailure: (turnContext: ChatTurnContext, assistantMessageId: number, content: string) => void;
  replacePending: (
    sessionId: string,
    pending: SearchContinuationPendingSteer<Capability, Attachment>,
  ) => void;
  releaseAttachments: (attachments: Attachment[]) => void;
  searchingStatus: string;
  readyStatus: string;
  incompleteMessage: string;
  failureMessage: (errorCode: string) => string;
};

export async function handleSearchContinuationRequest<Capability, Attachment>(
  request: ParsedSearchContinuationRequest,
  context: SearchContinuationTurnContext<Capability>,
  runtime: UiRuntime<Capability, Attachment>,
) {
  runtime.setStatus(context.turnContext, runtime.searchingStatus);
  const result = await executeSearchContinuation({
    request,
    turnContext: context.turnContext,
    state: context.state,
    isCurrent: () => runtime.isCurrent(context.turnContext),
    runSearch: runtime.runSearch,
  });
  if (result.kind === "superseded") {
    runtime.releaseAttachments(result.attachments as Attachment[]);
    return;
  }
  if (result.kind === "rejected" || result.kind === "failed") {
    const content = result.kind === "failed" && result.errorCode
      ? runtime.failureMessage(result.errorCode)
      : runtime.incompleteMessage;
    runtime.setFailure(context.turnContext, context.assistantMessageId, content);
    runtime.setStatus(context.turnContext, content);
    return;
  }
  const { turnContext } = context;
  runtime.replacePending(turnContext.sessionId, {
    turnContext: result.continuationContext,
    sessionId: turnContext.sessionId,
    agentId: turnContext.agentId,
    userMessageId: null,
    message: "Use the newly verified public evidence to finish the originating request. Answer directly; request another bounded search only if essential.",
    attachments: result.attachments as Attachment[],
    providerId: turnContext.route.providerId,
    modelId: turnContext.route.modelId,
    reasoning: turnContext.route.reasoning,
    context: context.context,
    contextBudget: turnContext.route.contextBudget,
    primaryRouteId: turnContext.route.primaryRouteId,
    fallbackRouteId: turnContext.route.fallbackRouteId,
    automatedWebGroundingEnabled: turnContext.route.automatedWebGroundingEnabled,
    assistantMessageId: context.assistantMessageId,
    mcpToolCapabilities: context.capabilities,
    toolLoopDepth: context.toolLoopDepth,
    executableActionExpected: false,
    verifiedNativeExecutionReceipt: true,
    searchContinuationState: result.state,
  });
  runtime.setStatus(turnContext, runtime.readyStatus);
}
