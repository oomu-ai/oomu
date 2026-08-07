import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type { ChatIntentRouteDecision } from "./chatIntentRouting";

export type ProjectChatDocumentRequest = { modelMessage: string };

export type ProjectDocumentExecutionRoute = {
  providerId: string;
  modelId: string;
};

export function projectDocumentLocalExecutionRoute(
  request: ProjectChatDocumentRequest | null,
  readiness: {
    localProviderId: string | null;
    localModelId: string | null;
    recommendedLocalProviderId: string | null;
    recommendedLocalModelId: string | null;
  },
  selectedRoute: ProjectDocumentExecutionRoute,
  selectedRouteIsLocal: boolean,
): ProjectDocumentExecutionRoute | null {
  if (!request) return null;
  const providerId = readiness.localProviderId ?? readiness.recommendedLocalProviderId;
  const modelId = readiness.localModelId ?? readiness.recommendedLocalModelId;
  if (providerId && modelId) return { providerId, modelId };
  return selectedRouteIsLocal ? selectedRoute : null;
}

export function requireProjectDocumentLocalExecutionRoute(request: ProjectChatDocumentRequest | null, readiness: Parameters<typeof projectDocumentLocalExecutionRoute>[1], selectedRoute: ProjectDocumentExecutionRoute, selectedRouteIsLocal: boolean, errorMessage: string) {
  const route = projectDocumentLocalExecutionRoute(request, readiness, selectedRoute, selectedRouteIsLocal);
  if (request && !route)
    throw {
      code: "project_document_local_model_required",
      message: errorMessage,
    };
  return route;
}

export function projectDocumentNativeRequestRoute<T>(
  request: ProjectChatDocumentRequest | null,
  readiness: Parameters<typeof projectDocumentLocalExecutionRoute>[1],
  selectedRoute: ProjectDocumentExecutionRoute & {
    dynamicRoutingEnabled: boolean;
  },
  selectedRouteIsLocal: boolean,
  autoRouteChoice: string | null,
  errorMessage: string,
  mcpToolCapabilities: T[],
) {
  const localRoute = requireProjectDocumentLocalExecutionRoute(request, readiness, selectedRoute, selectedRouteIsLocal, errorMessage);
  return {
    provider_id: localRoute?.providerId ?? selectedRoute.providerId,
    model_id: localRoute?.modelId ?? selectedRoute.modelId,
    dynamic_routing_override: request ? false : selectedRoute.dynamicRoutingEnabled,
    auto_route_choice: request ? null : autoRouteChoice,
    auto_route_cloud_confirmed: request ? false : autoRouteChoice === "cloud",
    mcp_tool_capabilities: mcpToolCapabilities,
    project_document_composition: Boolean(request),
  };
}

export function projectDocumentOutputRequested(message: string, projectId: string | null | undefined) {
  return !!projectId && /\b(?:word|docx)\b/i.test(message) && /\bpdf\b/i.test(message);
}

export function projectDocumentRouteDecision(message: string, projectId: string | null | undefined, statusLabel: string): ChatIntentRouteDecision | null {
  if (!projectDocumentOutputRequested(message, projectId)) return null;
  return {
    route: "agentic_planner",
    requires_local_access: true,
    decision_source: "native_artifact_creation_filter",
    reason: "The Project request requires native Word and PDF output.",
    matched_signals: ["Project Word and PDF deliverables"],
    status_label: statusLabel,
  };
}

export async function preferProjectDocumentRoute<T>(projectRoute: T | null, fallback: () => T | Promise<T>): Promise<T> {
  return projectRoute ?? fallback();
}

export function projectDocumentPendingAssistantId(message: string, projectId: string | null | undefined, createId: () => number) {
  return projectDocumentOutputRequested(message, projectId) ? createId() : null;
}

export function ensurePendingAssistantMessage<T extends { id: number }>(messages: T[], id: number | null): T[] {
  if (id === null || messages.some((message) => message.id === id)) return messages;
  return [...messages, { id, role: "assistant", content: "", isPending: true } as unknown as T];
}

export function projectChatDocumentRequest(message: string, routeDecision: ChatIntentRouteDecision, projectId: string | null | undefined): ProjectChatDocumentRequest | null {
  if (!projectDocumentOutputRequested(message, projectId) || routeDecision.decision_source !== "native_artifact_creation_filter") return null;
  const contentBrief = message
    .replace(/\busing\s+only\s+the\s+files\s+in\s+this\s+project\s*,?\s*/i, "")
    .replace(/\bproduce\b[^.]*\b(?:word|docx)\b[^.]*\bpdf\b[^.]*\.?/gi, "")
    .trim();
  return {
    modelMessage: ["Compose the complete document body from the approved Project knowledge supplied with this turn.", "Use only that evidence. Mark every unsupported claim clearly and do not invent facts.", "Return polished Markdown only, beginning with one descriptive level-one heading. Do not discuss tools or claim that files were created.", "Content brief:", contentBrief || message.trim()].join("\n\n"),
  };
}

export function prepareProjectChatDocumentTurn<T extends ChatIntentRouteDecision>(message: string, routeDecision: T, projectId: string | null | undefined, thinking: string): [ProjectChatDocumentRequest | null, T, string] {
  const request = projectChatDocumentRequest(message, routeDecision, projectId);
  if (!request) return [null, routeDecision, message];
  return [
    request,
    {
      ...routeDecision,
      route: "conversational_stream",
      requires_local_access: false,
      decision_source: "project_document_composition",
      reason: "The approved Project knowledge is being composed into the requested documents.",
      status_label: thinking,
    },
    request.modelMessage,
  ];
}

export function projectDocumentMcpCapabilities<T>(request: ProjectChatDocumentRequest | null, capabilities: T[]): T[] {
  return request ? [] : capabilities;
}

type ProjectDocumentResponse = {
  text: string;
  session_id: string;
  turn_id: string;
  generation_token: string;
};

type Translate = (key: string) => string;

function documentTitle(content: string) {
  const heading = content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => /^#\s+\S/.test(line))
    ?.replace(/^#\s+/, "")
    .trim();
  return heading?.slice(0, 240) || "Project Document";
}

export async function createProjectChatDocumentForTurn(request: ProjectChatDocumentRequest | null, response: ProjectDocumentResponse, turn: ChatTurnContext, locale: string, updateStatus: (turn: ChatTurnContext, status: string) => void, translate: Translate) {
  if (!request) return response.text;
  if (!turn.projectId)
    throw {
      code: "document_project_required",
      message: translate("documents.create_failed"),
    };
  updateStatus(turn, translate("documents.preparing"));
  const created = await invoke<{ artifactId: string; version: number }>("create_project_chat_document", {
    request: {
      sessionId: response.session_id || turn.sessionId,
      turnId: response.turn_id,
      generationToken: response.generation_token,
      projectId: turn.projectId,
      title: documentTitle(response.text),
      content: response.text,
      locale,
    },
  });
  window.sessionStorage.setItem("oomu.documents.focus", `word:${created.artifactId}`);
  return translate("documents.created");
}
