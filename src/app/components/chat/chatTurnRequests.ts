import type { ChatTurnContext } from "@/lib/chatTurnContext";

export function nativeTurnContextRequest(context: ChatTurnContext) {
  return {
    turn_id: context.turnId,
    generation_token: context.generationToken,
    parent_turn_id: context.ancestry.parentTurnId,
    root_turn_id: context.ancestry.rootTurnId,
    turn_kind: context.ancestry.kind,
  };
}

export function nativeProjectTurnContextRequest(context: ChatTurnContext) {
  return { ...nativeTurnContextRequest(context), project_id: context.projectId };
}

export function mcpTurnContextRequest(context: ChatTurnContext) {
  return {
    turnId: context.turnId,
    generationToken: context.generationToken,
    sessionId: context.sessionId,
    agentId: context.agentId,
    providerId: context.route.providerId,
    modelId: context.route.modelId,
    parentTurnId: context.ancestry.parentTurnId,
    rootTurnId: context.ancestry.rootTurnId,
    turnKind: context.ancestry.kind,
  };
}

export function systemDiagnosticsRequest(context: ChatTurnContext) {
  return {
    exportMarkdown: true,
    includeMemoryAudit: true,
    includePreAlphaAudit: true,
    preAlphaRuns: 1,
    turnContext: mcpTurnContextRequest(context),
  };
}

export function agentPlanTurnContextRequest(context: ChatTurnContext) {
  return {
    turnId: context.turnId,
    generationToken: context.generationToken,
    sessionId: context.sessionId,
    agentId: context.agentId,
    projectId: context.projectId,
    providerId: context.route.providerId,
    modelId: context.route.modelId,
    parentTurnId: context.ancestry.parentTurnId,
    rootTurnId: context.ancestry.rootTurnId,
    turnKind: context.ancestry.kind,
    reasoning: context.route.reasoning ?? null,
    contextBudget: context.route.contextBudget ?? null,
    primaryRouteId: context.route.primaryRouteId ?? null,
    fallbackRouteId: context.route.fallbackRouteId ?? null,
    dynamicRoutingEnabled: context.route.dynamicRoutingEnabled,
    automatedWebGroundingEnabled: context.route.automatedWebGroundingEnabled,
    attachmentGrants: context.attachmentGrants.map((grant) => ({
      name: grant.name,
      mimeType: grant.mimeType,
      byteCount: grant.byteCount,
    })),
    createdAtMs: context.createdAtMs,
  };
}
