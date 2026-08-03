"use client";

import { useApproval } from "@/context/ApprovalContext";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import {
  redactSensitiveText,
  redactSensitiveValue,
  safeErrorMessage,
} from "@/lib/redaction";
import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";

interface McpServerConfig {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  transport?:
    | { type: "native" }
    | { type: "stdio" }
    | { type: "http"; url: string; localOriginGrant?: { exactLoopbackPort: number } }
    | { type: "sse"; url: string; localOriginGrant?: { exactLoopbackPort: number } };
}

interface McpTool {
  name: string;
  description: string;
  inputSchema: unknown;
  outputSchema?: unknown;
  annotations?: unknown;
  _meta?: unknown;
}

export interface McpServerState {
  name: string;
  status: "disconnected" | "connecting" | "connected" | "error";
  tools: McpTool[];
  protocolVersion?: string;
  serverInfo?: unknown;
  capabilities?: unknown;
}

export interface McpToolCallResult {
  content: unknown[];
  structuredContent?: unknown;
  isError: boolean;
  _meta?: unknown;
  raw?: unknown;
}

interface McpToolSearchResult {
  serverName: string;
  name: string;
  description: string;
  score: number;
}

export interface McpToolApprovalRequest {
  approvalToken: string;
  serverName: string;
  toolName: string;
  arguments: unknown;
  message: string;
  capabilityRiskTier?: string;
  capabilityReason?: string;
  expiresAtMs: number;
  argumentSummary?: string;
  sensitiveFields?: string[];
  canonicalOrigin?: string;
  transport?: string;
  resolvedDestinationClass?: string;
  destinationBinding?: string;
  serverIdentityBinding?: string;
  certificateBinding?: string;
  toolDefinitionBinding?: string;
  auditId?: string;
  responseByteLimit?: number;
  nativeShieldApproved?: boolean;
}

type McpToolApproval = {
  approvalToken: string;
};

type McpToolApprovalHandler = (
  request: McpToolApprovalRequest,
) => Promise<boolean>;

type McpExecutionTurnContext = {
  turnId: string;
  generationToken: string;
  sessionId: string;
  agentId: string;
  providerId: string;
  modelId: string;
  parentTurnId: string | null;
  rootTurnId: string;
  turnKind: string;
};

type McpToolExecutionOptions = {
  requestApproval?: McpToolApprovalHandler;
  isExecutionContextCurrent?: () => boolean;
  turnContext?: McpExecutionTurnContext;
};

interface McpLogEntry {
  id: string;
  serverName?: string;
  level: "info" | "error";
  message: string;
  timestamp: number;
}

type McpContextValue = {
  cancelRemoteOperations: (serverName?: string) => Promise<number>;
  clearLog: () => void;
  connectServer: (config: McpServerConfig) => Promise<McpServerState>;
  executeTool: (
    serverName: string,
    toolName: string,
    argumentsValue?: unknown,
    options?: McpToolExecutionOptions,
  ) => Promise<McpToolCallResult>;
  getServerState: (serverName: string) => McpServerState;
  getToolDetails: (serverName: string, toolName: string) => Promise<McpTool>;
  listTools: (serverName: string) => Promise<McpTool[]>;
  log: McpLogEntry[];
  searchTools: (query: string) => Promise<McpToolSearchResult[]>;
  servers: Record<string, McpServerState>;
};

const DISCONNECTED_STATE: McpServerState = {
  name: "",
  status: "disconnected",
  tools: [],
};

const MCP_WORKSPACE_AUTHORIZATION_MESSAGE =
  "Local tools need your explicit approval before they can cross the workspace boundary. Approve the Shield Gate request, then try again.";

const McpContext = createContext<McpContextValue | null>(null);

function useBuiltinMcpServers(
  pushLog: (entry: Omit<McpLogEntry, "id" | "timestamp">) => void,
  setServers: Dispatch<SetStateAction<Record<string, McpServerState>>>,
) {
  useEffect(() => {
    if (!isTauriRuntime) return;

    let didCancel = false;
    void (async () => {
      try {
        const configs = await invoke<McpServerConfig[]>("mcp_builtin_server_configs");
        if (didCancel) return;
        setServers((current) => {
          const next = { ...current };
          configs.forEach((config) => {
            const normalizedConfig = normalizeConfig(config);
            next[normalizedConfig.name] = current[normalizedConfig.name] ?? {
              name: normalizedConfig.name,
              status: "disconnected",
              tools: [],
            };
          });
          return next;
        });

        // Built-in descriptors originate at the native trust boundary. The
        // trusted path avoids duplicating Shield approval for stdio startup.
        await Promise.allSettled(configs.map(async (config) => {
          if (didCancel) return;
          const serverName = config.name.trim();
          if (!serverName) return;
          try {
            setServers((current) => ({
              ...current,
              [serverName]: {
                name: serverName,
                status: "connecting",
                tools: current[serverName]?.tools ?? [],
              },
            }));
            const state = await invoke<McpServerState>("mcp_connect_builtin_server", {
              serverName,
            });
            if (didCancel) return;
            setServers((current) => ({ ...current, [state.name]: state }));
            pushLog({
              level: "info",
              message: `Connected built-in MCP server "${state.name}".`,
              serverName: state.name,
            });
          } catch (error) {
            if (didCancel) return;
            const message = getErrorMessage(error);
            setServers((current) => ({
              ...current,
              [serverName]: {
                name: serverName,
                status: "error",
                tools: current[serverName]?.tools ?? [],
              },
            }));
            pushLog({ level: "error", message, serverName });
            // One optional runtime must not hide the remaining built-in tools.
          }
        }));
      } catch (error) {
        if (!didCancel) pushLog({ level: "error", message: getErrorMessage(error) });
      }
    })();

    return () => {
      didCancel = true;
    };
  }, [pushLog, setServers]);
}

export function McpProvider({ children }: { children: ReactNode }) {
  const approvals = useApproval();
  const [servers, setServers] = useState<Record<string, McpServerState>>({});
  const [log, setLog] = useState<McpLogEntry[]>([]);
  const [connectInFlight] = useState(
    () => new Map<string, Promise<McpServerState>>(),
  );

  const pushLog = useCallback(
    (entry: Omit<McpLogEntry, "id" | "timestamp">) => {
      setLog((current) => [
        ...current,
        {
          ...entry,
          message: redactSensitiveText(entry.message),
          serverName: entry.serverName
            ? redactSensitiveText(entry.serverName)
            : undefined,
          id: `mcp-log-${Date.now()}-${current.length}`,
          timestamp: Date.now(),
        },
      ]);
    },
    [],
  );

  const connectServer = useCallback(
    async (config: McpServerConfig) => {
      const normalizedConfig = normalizeConfig(config);
      const existing = connectInFlight.get(normalizedConfig.name);

      if (existing) {
        return existing;
      }

      setServers((current) => ({
        ...current,
        [normalizedConfig.name]: {
          name: normalizedConfig.name,
          status: "connecting",
          tools: current[normalizedConfig.name]?.tools ?? [],
        },
      }));

      const request = invoke<McpServerState>("mcp_connect_server", {
        config: normalizedConfig,
      })
        .then((state) => {
          setServers((current) => ({
            ...current,
            [state.name]: state,
          }));
          pushLog({
            level: "info",
            message: `Connected MCP server "${state.name}".`,
            serverName: state.name,
          });
          return state;
        })
        .catch((error: unknown) => {
          const message = getErrorMessage(error);
          setServers((current) => ({
            ...current,
            [normalizedConfig.name]: {
              name: normalizedConfig.name,
              status: "error",
              tools: current[normalizedConfig.name]?.tools ?? [],
            },
          }));
          pushLog({
            level: "error",
            message,
            serverName: normalizedConfig.name,
          });
          throw normalizedMcpError(error, message);
        })
        .finally(() => {
          connectInFlight.delete(normalizedConfig.name);
        });

      connectInFlight.set(normalizedConfig.name, request);
      return request;
    },
    [connectInFlight, pushLog],
  );

  useBuiltinMcpServers(pushLog, setServers);

  const listTools = useCallback(
    async (serverName: string) => {
      try {
        const tools = await invoke<McpTool[]>("mcp_list_tools", { serverName });
        setServers((current) => ({
          ...current,
          [serverName]: {
            name: serverName,
            status: "connected",
            tools,
          },
        }));
        return tools;
      } catch (error) {
        const message = getErrorMessage(error);
        setServers((current) => ({
          ...current,
          [serverName]: {
            name: serverName,
            status: "error",
            tools: current[serverName]?.tools ?? [],
          },
        }));
        pushLog({ level: "error", message, serverName });
        throw normalizedMcpError(error, message);
      }
    },
    [pushLog],
  );

  const searchTools = useCallback(
    async (query: string) => {
      try {
        return await invoke<McpToolSearchResult[]>("mcp_search_tools", { query });
      } catch (error) {
        const message = getErrorMessage(error);
        pushLog({ level: "error", message });
        throw normalizedMcpError(error, message);
      }
    },
    [pushLog],
  );

  const getToolDetails = useCallback(
    async (serverName: string, toolName: string) => {
      try {
        return await invoke<McpTool>("mcp_get_tool_details", {
          serverName,
          toolName,
        });
      } catch (error) {
        const message = getErrorMessage(error);
        pushLog({ level: "error", message, serverName });
        throw normalizedMcpError(error, message);
      }
    },
    [pushLog],
  );

  const requestToolApproval = useCallback(
    async (request: McpToolApprovalRequest) => {
      const result = await approvals.requestApproval({
        approvalToken: request.approvalToken,
        actionType: "mcp_tool_call",
        actionLabel: `${request.serverName}/${request.toolName}`,
        targetPath: approvalTargetPath(request.arguments),
        principal: request.serverName,
        riskTier: request.capabilityRiskTier ?? "unknown",
        reason: request.capabilityReason ?? request.message,
        requestedAtMs: Date.now(),
        preview: "",
        approvalScopeKinds: ["once"],
      });
      return result.decision === "approve";
    },
    [approvals],
  );

  const executeTool = useCallback(
    async (
      serverName: string,
      toolName: string,
      argumentsValue: unknown = {},
      options?: McpToolExecutionOptions,
    ) => {
      try {
        const approvalRequest = await invoke<McpToolApprovalRequest | null>(
          "mcp_prepare_tool_approval",
          {
            arguments: argumentsValue,
            serverName,
            toolName,
          },
        );
        let approval: McpToolApproval | undefined;
        if (approvalRequest) {
          // Remote calls have already passed the app-level native permission
          // sheet. Reusing that exact one-use token avoids asking twice for the
          // same operation; local tools still receive this confirmation.
          const approved = approvalRequest.nativeShieldApproved
            ? true
            : await (options?.requestApproval ?? requestToolApproval)(
                approvalRequest,
              );
          if (!approved) {
            await rejectToolApproval(approvalRequest.approvalToken);
            throw mcpError(
              "shield_approval_denied",
              `MCP tool "${toolName}" was not approved.`,
            );
          }
          approval = { approvalToken: approvalRequest.approvalToken };
        }

        if (options?.isExecutionContextCurrent?.() === false) {
          if (approvalRequest) {
            await rejectToolApproval(approvalRequest.approvalToken);
          }
          throw new Error(
            `MCP tool "${toolName}" was cancelled because its originating chat turn is no longer active.`,
          );
        }

        const executeArgs: Record<string, unknown> = {
          arguments: argumentsValue,
          serverName,
          toolName,
        };
        if (options?.turnContext) {
          executeArgs.turnContext = options.turnContext;
        }
        if (approval) {
          executeArgs.approval = approval;
        }
        const result = await invoke<McpToolCallResult>("mcp_execute_tool", executeArgs);
        if (result.isError) {
          const message = getToolResultErrorMessage(toolName, result);
          throw new Error(message);
        }
        return result;
      } catch (error) {
        const message = getErrorMessage(error);
        pushLog({ level: "error", message, serverName });
        throw normalizedMcpError(error, message);
      }
    },
    [pushLog, requestToolApproval],
  );

  const getServerState = useCallback(
    (serverName: string) =>
      servers[serverName] ?? {
        ...DISCONNECTED_STATE,
        name: serverName,
      },
    [servers],
  );

  const clearLog = useCallback(() => {
    setLog([]);
  }, []);

  const cancelRemoteOperations = useCallback(
    async (serverName?: string) =>
      invoke<number>("mcp_cancel_remote_operations", {
        serverName: serverName?.trim() || null,
      }),
    [],
  );

  const value = useMemo<McpContextValue>(
    () => ({
      cancelRemoteOperations,
      clearLog,
      connectServer,
      executeTool,
      getServerState,
      getToolDetails,
      listTools,
      log,
      searchTools,
      servers,
    }),
    [
      cancelRemoteOperations,
      clearLog,
      connectServer,
      executeTool,
      getServerState,
      getToolDetails,
      listTools,
      log,
      searchTools,
      servers,
    ],
  );

  return createElement(McpContext.Provider, { value }, children);
}

export function useOptionalMcp() {
  return useContext(McpContext);
}

function normalizeConfig(config: McpServerConfig): McpServerConfig {
  return {
    args: Array.isArray(config.args) ? config.args : [],
    command: config.command,
    env: config.env ?? {},
    name: config.name,
    transport: config.transport ?? { type: "stdio" },
  };
}

function approvalTargetPath(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  for (const key of ["path", "folderPath", "directoryPath", "targetPath"]) {
    if (typeof record[key] === "string" && record[key].trim()) {
      return record[key].trim();
    }
  }
  return null;
}

function mcpError(code: string, message: string) {
  return Object.assign(new Error(message), { code });
}

function mcpErrorCode(error: unknown) {
  if (typeof error === "string") {
    try {
      return mcpErrorCode(JSON.parse(error));
    } catch {
      return "";
    }
  }
  return error &&
    typeof error === "object" &&
    "code" in error &&
    typeof error.code === "string"
    ? error.code
    : "";
}

function normalizedMcpError(error: unknown, message: string) {
  const code = mcpErrorCode(error);
  return code ? mcpError(code, message) : new Error(message);
}

function getErrorMessage(error: unknown) {
  if (isMcpAuthorizationError(error)) {
    return MCP_WORKSPACE_AUTHORIZATION_MESSAGE;
  }

  return safeErrorMessage(error, "MCP request failed.");
}

function isMcpAuthorizationError(error: unknown) {
  const code =
    error &&
    typeof error === "object" &&
    "code" in error &&
    typeof error.code === "string"
      ? error.code
      : "";
  if (
    code === "mcp_permission_required" ||
    code === "shield_approval_denied" ||
    code === "shield_approval_not_found"
  ) {
    return true;
  }

  const message =
    typeof error === "string"
      ? error
      : error &&
          typeof error === "object" &&
          "message" in error &&
          typeof error.message === "string"
        ? error.message
        : "";
  return /MCP workspace boundary|MCP Permission Gateway|MCP stdio server|Shield Gate approval|MCP approval token|approval token|mcp_(?:connect_server|execute_tool).*not allowed|Command "?mcp_(?:connect_server|execute_tool)"?/i.test(
    message,
  );
}

const MAX_MCP_ERROR_CONTENT_ITEMS = 32;

export function getToolResultErrorMessage(
  toolName: string,
  result: McpToolCallResult,
) {
  const contentText = result.content
    .slice(0, MAX_MCP_ERROR_CONTENT_ITEMS)
    .map((item) => {
      if (
        item &&
        typeof item === "object" &&
        "text" in item &&
        typeof item.text === "string"
      ) {
        return redactSensitiveText(item.text);
      }
      return safeStringify(item);
    })
    .filter((item) => item.trim().length > 0)
    .join(" ");

  const safeToolName = redactSensitiveText(toolName);
  return redactSensitiveText(
    contentText
      ? `MCP tool "${safeToolName}" returned an error: ${contentText}`
      : `MCP tool "${safeToolName}" returned an error result.`,
  );
}

async function rejectToolApproval(approvalToken: string) {
  try {
    await invoke<void>("mcp_reject_tool_approval", { approvalToken });
  } catch {
    // Approval tokens expire server-side; rejection is best-effort cleanup.
  }
}

function safeStringify(value: unknown) {
  const redacted = redactSensitiveValue(value);
  if (typeof redacted === "string") {
    return redactSensitiveText(redacted);
  }

  try {
    return redactSensitiveText(JSON.stringify(redacted));
  } catch {
    return "";
  }
}
