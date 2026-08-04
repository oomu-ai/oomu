import type {
  ApprovalResult,
  ApprovalScopeKind,
} from "@/lib/approvalContracts";

type McpApprovalCandidate = {
  approvalScopeKinds?: readonly string[];
  chatSessionApproved?: boolean;
  nativeShieldApproved?: boolean;
  serverName: string;
  toolName: string;
};

type ChatMcpApprovalCandidate = McpApprovalCandidate & {
  approvalToken: string;
  capabilityReason?: string;
  capabilityRiskTier?: string;
  message: string;
};

type ChatApprovalTurn = {
  generationToken: string;
  sessionId: string;
  turnId: string;
};

type NativeApprovalPresentation = {
  actionLabel: string;
  actionType: string;
  preview: string;
};

export const DENIED_ONCE_APPROVAL = {
  decision: "deny",
  scopeKind: "once",
} as const satisfies ApprovalResult;

const APPROVED_ONCE = {
  decision: "approve",
  scopeKind: "once",
} as const satisfies ApprovalResult;

export function isPublicSearchTool(
  request: Pick<McpApprovalCandidate, "serverName" | "toolName">,
) {
  return request.serverName.trim().toLowerCase() === "local_search" &&
    request.toolName.trim().toLowerCase() === "search_web";
}

export function mcpApprovalScopeFields(
  request: McpApprovalCandidate,
  sessionId?: string,
  actionTypeOverride?: string,
) {
  const chatSessionAvailable = Boolean(
    !actionTypeOverride &&
    sessionId?.trim() &&
    isPublicSearchTool(request) &&
    request.approvalScopeKinds?.includes("chat_session"),
  );
  return {
    actionClass: chatSessionAvailable ? "public_web_search" : undefined,
    actionType: actionTypeOverride ?? (
      chatSessionAvailable ? "public_web_search" : "mcp_tool_call"
    ),
    approvalScopeKinds: (
      chatSessionAvailable ? ["once", "chat_session"] : ["once"]
    ) as ApprovalScopeKind[],
    scopeTrustAvailable: chatSessionAvailable,
    sessionId: chatSessionAvailable ? sessionId : undefined,
  };
}

export function chatMcpShieldApprovalRequest(
  request: ChatMcpApprovalCandidate,
  turn: ChatApprovalTurn,
  nativeApproval: NativeApprovalPresentation | null,
  targetPath: string | null,
) {
  return {
    ...mcpApprovalScopeFields(request, turn.sessionId, nativeApproval?.actionType),
    approvalToken: request.approvalToken,
    sessionId: turn.sessionId,
    turnId: turn.turnId,
    generationToken: turn.generationToken,
    actionLabel: nativeApproval?.actionLabel ?? `${request.serverName}/${request.toolName}`,
    targetPath,
    principal: "Conversational agent",
    riskTier: request.capabilityRiskTier ?? "UNKNOWN",
    reason: request.capabilityReason ?? request.message,
    estimatedTokenCosts: null,
    requestedAtMs: Date.now(),
    preview: nativeApproval?.preview ?? "",
  };
}

export function preapprovedMcpResult(request: McpApprovalCandidate) {
  return request.nativeShieldApproved || request.chatSessionApproved
    ? APPROVED_ONCE
    : null;
}

export function normalizedMcpScopeKind(
  request: McpApprovalCandidate,
  selectedScopeKind: string,
): ApprovalScopeKind {
  return selectedScopeKind === "chat_session" &&
    isPublicSearchTool(request) &&
    request.approvalScopeKinds?.includes("chat_session")
    ? "chat_session"
    : "once";
}
