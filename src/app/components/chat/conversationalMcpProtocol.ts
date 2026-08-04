import { sanitizeInferenceText } from "@/lib/InferenceService";
import type { McpServerState } from "@/hooks/useMcp";
import {
  assistantTextForSearchContinuation,
  parseSearchContinuationRequest,
} from "./searchContinuationCoordinator";
import { canonicalAssistantDisplayText } from "./canonicalAssistantDisplay";
import type { ChatAttachment } from "./attachments";

export type ConversationalMcpToolCapability = {
  serverName: string;
  toolName: string;
  description: string;
  inputSchema: unknown;
};

export type ConversationalMcpToolCall = {
  serverName: string;
  toolName: string;
  argumentsValue: unknown;
};

export type ParsedConversationalMcpToolRequest = {
  call: ConversationalMcpToolCall;
  blockText: string;
};

type Translate = (key: string) => string;

const toolFencePattern =
  /```[ \t]*(?:json[ \t]+)?oomu_mcp_tool_call[^\n]*\n([\s\S]*?)```/i;
export const maxConversationalMcpToolLoopDepth = 3;

export function conversationalMcpCapabilitiesFromServers(
  servers: Record<string, McpServerState> | undefined,
): ConversationalMcpToolCapability[] {
  if (!servers) return [];
  return Object.values(servers)
    .filter((server) => server.status === "connected")
    .flatMap((server) => server.tools
      .filter((tool) => Boolean(server.name.trim() && tool.name.trim()))
      .map((tool) => ({
        serverName: server.name, toolName: tool.name,
        description: tool.description, inputSchema: tool.inputSchema,
      })))
    .sort((left, right) => (left.serverName + "/" + left.toolName)
      .localeCompare(right.serverName + "/" + right.toolName));
}

export function conversationalMcpToolIsAvailable(
  call: ConversationalMcpToolCall,
  capabilities: ConversationalMcpToolCapability[],
) {
  return capabilities.some((capability) =>
    capability.serverName === call.serverName && capability.toolName === call.toolName);
}

export function parseConversationalMcpToolRequest(
  text: string,
): ParsedConversationalMcpToolRequest | null {
  const match = toolFencePattern.exec(text);
  if (match) {
    const parsed = parseJsonObject(match[1] ?? "");
    const call = parsed ? normalizeConversationalMcpToolCall(parsed) : null;
    return call ? { call, blockText: match[0] } : null;
  }
  return parseCompactConversationalMcpToolRequest(text);
}

const compactToolCallPattern =
  /^call:([a-zA-Z0-9_-]{1,80})\/([a-zA-Z0-9_.-]{1,120})\s*(\{[^\r\n]{0,2000}\})$/;

/**
 * Small local models occasionally preserve the tool identity but compress the
 * requested fenced JSON into `call:server/tool{key: value}`. Accept only that
 * exact, whole-response form. Availability, schema, approval, and execution
 * remain enforced by the native broker; prose containing a lookalike is never
 * promoted into a tool request.
 */
function parseCompactConversationalMcpToolRequest(
  text: string,
): ParsedConversationalMcpToolRequest | null {
  const blockText = text.trim();
  const match = compactToolCallPattern.exec(blockText);
  if (!match) return null;
  const argumentsValue = parseCompactArguments(match[3] ?? "");
  if (!argumentsValue) return null;
  return {
    call: {
      serverName: match[1],
      toolName: match[2],
      argumentsValue,
    },
    blockText,
  };
}

function parseCompactArguments(text: string): Record<string, unknown> | null {
  const json = parseJsonObject(text);
  if (json) return json;
  const body = text.slice(1, -1).trim();
  if (!body) return {};

  const fields = body.split(/,\s*(?=[a-zA-Z_][a-zA-Z0-9_]*\s*:)/);
  if (fields.length > 16) return null;
  const result: Record<string, unknown> = {};
  for (const field of fields) {
    const separator = field.indexOf(":");
    if (separator <= 0) return null;
    const key = field.slice(0, separator).trim();
    const rawValue = field.slice(separator + 1).trim();
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key) || key in result || !rawValue) {
      return null;
    }
    const value = parseCompactScalar(rawValue);
    if (value === undefined) return null;
    result[key] = value;
  }
  return result;
}

function parseCompactScalar(value: string): unknown | undefined {
  if (/^"(?:[^"\\]|\\.)*"$/.test(value)) {
    try {
      return JSON.parse(value);
    } catch {
      return undefined;
    }
  }
  if (value === "true") return true;
  if (value === "false") return false;
  if (value === "null") return null;
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value)) {
    const number = Number(value);
    return Number.isFinite(number) ? number : undefined;
  }
  if (/[{}\[\]`]/.test(value)) return undefined;
  return value;
}

function normalizeConversationalMcpToolCall(
  payload: Record<string, unknown>,
): ConversationalMcpToolCall | null {
  const record = firstRecord(payload.oomu_mcp_tool_call, payload.tool_call, payload.toolCall)
    ?? payload;
  const serverName = firstString(record.serverName, record.server_name, record.server);
  const toolName = firstString(record.toolName, record.tool_name, record.name);
  return serverName && toolName ? {
    serverName,
    toolName,
    argumentsValue: record.arguments ?? record.args ?? record.input ?? record.parameters ?? {},
  } : null;
}

function parseJsonObject(text: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(text.trim());
    return isPlainRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function firstRecord(...values: unknown[]) {
  return values.find(isPlainRecord) as Record<string, unknown> | undefined;
}

export function firstString(...values: unknown[]) {
  const value = values.find(
    (candidate) => typeof candidate === "string" && candidate.trim().length > 0,
  );
  return typeof value === "string" ? value.trim() : null;
}

export function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function assistantTextForMcpRequest(
  assistantText: string,
  request: ParsedConversationalMcpToolRequest,
  translate: Translate,
) {
  return assistantText.replace(request.blockText, "").trim() ||
    translate("chat.mcp.requesting_tool");
}

export function assistantControlProjection(
  text: string,
  translate: Translate,
  allowSearch = true,
) {
  const searchRequest = allowSearch ? parseSearchContinuationRequest(text) : null;
  const mcpRequest = parseConversationalMcpToolRequest(text);
  const displayText = canonicalAssistantDisplayText(searchRequest
    ? assistantTextForSearchContinuation(text, searchRequest)
    : mcpRequest
      ? assistantTextForMcpRequest(text, mcpRequest, translate)
      : text);
  return { searchRequest, mcpRequest, displayText };
}

export function sanitizeAssistantTranscriptText(content: string, translate: Translate) {
  const sanitized = sanitizeInferenceText(content);
  return assistantControlProjection(sanitized, translate).displayText;
}

export function mcpContinuationMessage(call: ConversationalMcpToolCall) {
  return [
    "A verified local result is attached for the user's original request.",
    `Source: ${call.serverName}/${call.toolName}`,
    "Use only the attached result. Do not request another local tool unless more local context is necessary.",
  ].join("\n");
}

export function mcpTerminalOutcomeMessage(call: ConversationalMcpToolCall) {
  return [
    "The native tool broker returned a terminal outcome for the user's original request.",
    `Source: ${call.serverName}/${call.toolName}`,
    "Explain the attached outcome plainly. Do not claim the tool succeeded and do not request another tool in this continuation.",
  ].join("\n");
}

export function mcpTerminalOutcomeText(
  call: ConversationalMcpToolCall,
  outcome: string,
  detail: string,
) {
  return [
    `Source: ${call.serverName}/${call.toolName}`,
    `Terminal outcome: ${outcome}`,
    `Detail: ${detail}`,
  ].join("\n");
}

export function mcpContinuationAttachment(
  call: ConversationalMcpToolCall,
  resultText: string,
): ChatAttachment {
  const safeName = call.toolName.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_");
  return {
    name: `connector_${safeName || "result"}.json`,
    mime_type: "application/json",
    byte_count: new TextEncoder().encode(resultText).byteLength,
    text: resultText,
  };
}

export function isSovereignMcpSearchCall(call: ConversationalMcpToolCall) {
  return call.serverName.trim().toLowerCase() === "local_search" &&
    call.toolName.trim().toLowerCase() === "search_web";
}

export function sovereignMcpSearchQuery(call: ConversationalMcpToolCall) {
  return isPlainRecord(call.argumentsValue)
    ? firstString(call.argumentsValue.query)
    : null;
}
