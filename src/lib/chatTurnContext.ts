type ChatTurnKind = "root" | "queued" | "steer" | "retry";

type ChatTurnRoute = Readonly<{
  providerId: string;
  modelId: string;
  reasoning?: string;
  contextBudget?: number;
  primaryRouteId?: string | null;
  fallbackRouteId?: string | null;
  dynamicRoutingEnabled: boolean;
  automatedWebGroundingEnabled: boolean;
}>;

type ChatTurnAttachmentGrant = Readonly<{
  name: string;
  mimeType: string;
  byteCount: number;
}>;

type ChatTurnAncestry = Readonly<{
  kind: ChatTurnKind;
  parentTurnId: string | null;
  rootTurnId: string;
}>;

export type ChatTurnContext = Readonly<{
  turnId: string;
  generationToken: string;
  sessionId: string;
  agentId: string;
  projectId: string | null;
  route: ChatTurnRoute;
  attachmentGrants: ReadonlyArray<ChatTurnAttachmentGrant>;
  ancestry: ChatTurnAncestry;
  createdAtMs: number;
}>;

type CreateChatTurnContextInput = {
  turnId: string;
  generationToken: string;
  sessionId: string;
  agentId: string;
  projectId?: string | null;
  route: ChatTurnRoute;
  attachmentGrants?: ReadonlyArray<ChatTurnAttachmentGrant>;
  ancestry?: Partial<ChatTurnAncestry>;
  createdAtMs?: number;
};

let fallbackIdentityCounter = 0;

export function createChatTurnIdentity(prefix = "turn") {
  const randomUuid = globalThis.crypto?.randomUUID?.bind(globalThis.crypto);
  if (randomUuid) {
    return randomUuid();
  }
  fallbackIdentityCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${fallbackIdentityCounter.toString(36)}`;
}

function requiredIdentity(value: string, label: string) {
  const cleaned = value.trim();
  if (!cleaned) {
    throw new Error(`${label} must not be empty.`);
  }
  return cleaned;
}

function immutableRoute(route: ChatTurnRoute): ChatTurnRoute {
  return Object.freeze({
    providerId: requiredIdentity(route.providerId, "providerId"),
    modelId: requiredIdentity(route.modelId, "modelId"),
    reasoning: route.reasoning?.trim() || undefined,
    contextBudget:
      typeof route.contextBudget === "number" && Number.isFinite(route.contextBudget)
        ? route.contextBudget
        : undefined,
    primaryRouteId: route.primaryRouteId?.trim() || null,
    fallbackRouteId: route.fallbackRouteId?.trim() || null,
    dynamicRoutingEnabled: route.dynamicRoutingEnabled,
    automatedWebGroundingEnabled: route.automatedWebGroundingEnabled,
  });
}

function immutableAttachmentGrants(
  grants: ReadonlyArray<ChatTurnAttachmentGrant>,
): ReadonlyArray<ChatTurnAttachmentGrant> {
  return Object.freeze(
    grants.map((grant) =>
      Object.freeze({
        name: requiredIdentity(grant.name, "attachment name"),
        mimeType: requiredIdentity(grant.mimeType, "attachment mimeType"),
        byteCount: Math.max(0, Math.trunc(grant.byteCount)),
      }),
    ),
  );
}

export function createChatTurnContext(input: CreateChatTurnContextInput): ChatTurnContext {
  const turnId = requiredIdentity(input.turnId, "turnId");
  const kind = input.ancestry?.kind ?? "root";
  const parentTurnId = input.ancestry?.parentTurnId?.trim() || null;
  if (kind === "root" && parentTurnId) {
    throw new Error("A root turn cannot have a parentTurnId.");
  }
  if (kind !== "root" && !parentTurnId) {
    throw new Error(`${kind} turns require a parentTurnId.`);
  }
  const rootTurnId = requiredIdentity(
    input.ancestry?.rootTurnId ?? (kind === "root" ? turnId : ""),
    "rootTurnId",
  );

  return Object.freeze({
    turnId,
    generationToken: requiredIdentity(input.generationToken, "generationToken"),
    sessionId: requiredIdentity(input.sessionId, "sessionId"),
    agentId: requiredIdentity(input.agentId, "agentId"),
    projectId: input.projectId?.trim() || null,
    route: immutableRoute(input.route),
    attachmentGrants: immutableAttachmentGrants(input.attachmentGrants ?? []),
    ancestry: Object.freeze({ kind, parentTurnId, rootTurnId }),
    createdAtMs: input.createdAtMs ?? Date.now(),
  });
}

export function deriveChatTurnContext(
  parent: ChatTurnContext,
  kind: Exclude<ChatTurnKind, "root">,
  input: {
    turnId: string;
    generationToken: string;
    attachmentGrants?: ReadonlyArray<ChatTurnAttachmentGrant>;
    createdAtMs?: number;
  },
): ChatTurnContext {
  return createChatTurnContext({
    turnId: input.turnId,
    generationToken: input.generationToken,
    sessionId: parent.sessionId,
    agentId: parent.agentId,
    projectId: parent.projectId,
    route: parent.route,
    attachmentGrants: input.attachmentGrants ?? parent.attachmentGrants,
    ancestry: {
      kind,
      parentTurnId: parent.turnId,
      rootTurnId: parent.ancestry.rootTurnId,
    },
    createdAtMs: input.createdAtMs,
  });
}

export function rebindChatTurnAttachments(
  context: ChatTurnContext,
  attachments: ReadonlyArray<{ name: string; mime_type: string; byte_count: number }>,
): ChatTurnContext {
  return createChatTurnContext({
    turnId: context.turnId,
    generationToken: context.generationToken,
    sessionId: context.sessionId,
    agentId: context.agentId,
    projectId: context.projectId,
    route: context.route,
    attachmentGrants: attachments.map((attachment) => ({
      name: attachment.name,
      mimeType: attachment.mime_type,
      byteCount: attachment.byte_count,
    })),
    ancestry: context.ancestry,
    createdAtMs: context.createdAtMs,
  });
}

export function rebindChatTurnExecutionRoute(
  context: ChatTurnContext,
  providerId: string,
  modelId: string,
): ChatTurnContext {
  return createChatTurnContext({
    turnId: context.turnId,
    generationToken: context.generationToken,
    sessionId: context.sessionId,
    agentId: context.agentId,
    projectId: context.projectId,
    route: {
      ...context.route,
      providerId,
      modelId,
    },
    attachmentGrants: context.attachmentGrants,
    ancestry: context.ancestry,
    createdAtMs: context.createdAtMs,
  });
}

export function chatTurnContextMatches(
  expected: Pick<ChatTurnContext, "turnId" | "generationToken" | "sessionId" | "agentId" | "projectId" | "attachmentGrants">,
  received: Pick<ChatTurnContext, "turnId" | "generationToken" | "sessionId" | "agentId" | "projectId" | "attachmentGrants">,
) {
  return (
    expected.turnId === received.turnId &&
    expected.generationToken === received.generationToken &&
    expected.sessionId === received.sessionId &&
    expected.agentId === received.agentId &&
    expected.projectId === received.projectId &&
    expected.attachmentGrants.length === received.attachmentGrants.length &&
    expected.attachmentGrants.every((grant, index) => {
      const candidate = received.attachmentGrants[index];
      return Boolean(
        candidate &&
        grant.name === candidate.name &&
        grant.mimeType === candidate.mimeType &&
        grant.byteCount === candidate.byteCount
      );
    })
  );
}
