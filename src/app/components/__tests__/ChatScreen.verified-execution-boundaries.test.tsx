import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession, StoredChatMessage } from "@/lib/chatSessions";
import { ChatScreen } from "../ChatScreen";
import {
  agents,
  configuredProviders,
  sessions,
} from "./ChatScreen.execution-boundaries.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const tauriRuntimeMock = vi.hoisted(() => ({ value: true }));
const reconciliationDelaysMock = vi.hoisted(() => ({ value: null as number[] | null }));
const optionalMcpMock = vi.hoisted(() => ({
  value: null as null | {
    cancelRemoteOperations: () => Promise<number>;
    executeTool: ReturnType<typeof vi.fn>;
    servers: Record<string, unknown>;
  },
}));

vi.mock("../chat/turnReconciliation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../chat/turnReconciliation")>();
  return {
    ...actual,
    waitForTerminalChatTurnResult: (
      fetchMessages: () => Promise<StoredChatMessage[]>,
      turnId: string,
      options?: Parameters<typeof actual.waitForTerminalChatTurnResult>[2],
    ) => actual.waitForTerminalChatTurnResult(fetchMessages, turnId, {
      ...options,
      delaysMs: reconciliationDelaysMock.value ?? options?.delaysMs,
    }),
  };
});

vi.mock("@/hooks/useMcp", () => ({
  useOptionalMcp: () => optionalMcpMock.value,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) {
      return true;
    }
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
    }
    if (
      (command === "chat_turn" || command === "record_browser_chat_turn") &&
      response &&
      typeof response === "object"
    ) {
      const request = args?.request ?? {};
      return {
        ...response,
        session_id: response.session_id ?? request.session_id,
        turn_id: response.turn_id ?? request.turn_id,
        generation_token: response.generation_token ?? request.generation_token,
      };
    }
    return response;
  },
  get isTauriRuntime() {
    return tauriRuntimeMock.value;
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class TestChannel {
    constructor() {}
  },
}));

vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: null,
    fallbackRoute: null,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

function renderChatScreen(
  sessionFixtures: ChatSession[] = sessions,
  projectId: string | null = null,
) {
  return render(
    <ChatScreen
      activeSessionId="session-1"
      agents={agents}
      configuredProviders={configuredProviders}
      onCreateSession={vi.fn()}
      onDeleteSession={vi.fn()}
      onSelectSession={vi.fn()}
      onSessionsChange={vi.fn()}
      privacySettings={null}
      projectId={projectId}
      sessions={sessionFixtures}
    />,
    { wrapper: I18nProvider },
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  tauriRuntimeMock.value = true;
  reconciliationDelaysMock.value = null;
  optionalMcpMock.value = null;
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      clear: vi.fn(),
      getItem: vi.fn(() => null),
      removeItem: vi.fn(),
      setItem: vi.fn(),
    },
  });
});

afterEach(() => {
  cleanup();
});

it("retries ordinary chat once when an escalated planner route is rejected", async () => {
    tauriRuntimeMock.value = false;
    let chatTurnCount = 0;
    let chatCompleted = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return chatCompleted
          ? [{
              id: 92,
              sessionId: "session-1",
              role: "assistant",
              content: "Most people will discover capabilities as they need them.",
              createdAtMs: 92,
            }]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No executable action is present.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        if (chatTurnCount === 1) {
          return {
            text: "Pivoting to Agentic Planner.",
            session_id: "session-1",
            route_escalation: {
              route: "agentic_planner",
              requires_local_access: true,
              decision_source: "stale_server_preflight",
              reason: "A stale classifier requested planning.",
              matched_signals: ["false positive"],
              status_label: "Planning...",
            },
          };
        }
        chatCompleted = true;
        return {
          text: "Most people will discover capabilities as they need them.",
          session_id: "session-1",
        };
      }
      if (command === "process_agent_objective") {
        throw Object.assign(new Error("This request is conversational."), {
          code: "agent_objective_not_executable",
        });
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "99.9% of users will discover that later." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => expect(chatTurnCount).toBe(2));
    expect(
      await screen.findByText("Most people will discover capabilities as they need them."),
    ).toBeVisible();
    const chatCalls = invokeMock.mock.calls.filter(([command]) => command === "chat_turn");
    expect(chatCalls).toHaveLength(2);
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "process_agent_objective"),
    ).toHaveLength(1);
    const firstRequest = chatCalls[0]?.[1]?.request;
    const retryRequest = chatCalls[1]?.[1]?.request;
    expect(retryRequest).toEqual(expect.objectContaining({
      turn_id: firstRequest?.turn_id,
      generation_token: firstRequest?.generation_token,
      parent_turn_id: firstRequest?.parent_turn_id,
      root_turn_id: firstRequest?.root_turn_id,
      turn_kind: firstRequest?.turn_kind,
      mcp_tool_capabilities: [],
    }));
    expect(screen.queryByText(/agent_object_not_executable/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/ActionPlan/i)).not.toBeInTheDocument();
  });

  it("keeps a verified MCP mutation receipt through its natural-language continuation", async () => {
    const executeTool = vi.fn(async () => ({
      content: [{ type: "text", text: "Wrote notes.txt and verified the saved contents." }],
      structuredContent: { path: "notes.txt", relativePath: "notes.txt", bytesWritten: 7 },
      isError: false,
      _meta: {
        oomuNativeExecutionReceipt: {
          schema: "oomu.native-mcp-execution.v1", receiptId: "apple-operation-write-1",
          outcome: "succeeded", verified: true,
          postcondition: { nativeResultCode: "verified" },
        },
      },
    }));
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_filesystem: {
          name: "local_filesystem",
          status: "connected",
          tools: [
            { name: "read_file", description: "Read a local file", inputSchema: { type: "object" } },
            { name: "write_file", description: "Write a local file", inputSchema: { type: "object" } },
          ],
        },
      },
    };

    let chatTurnCount = 0;
    let persistedMessages: StoredChatMessage[] = [];
    let hydratedVerifiedCompletion = false;
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        hydratedVerifiedCompletion ||= persistedMessages.some(
          (message) => message.metadataJson?.includes("verifiedNativeExecutionReceipt"),
        );
        return persistedMessages;
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_system_hardware_profile") {
        return {
          physicalMemoryGb: 16,
          processorTier: "Mid (Metal, 16K local context)",
          cpuArch: "aarch64",
          cpuCores: 8,
          osName: "macos",
          metalSupported: true,
          maxLocalContextBudget: 16_384,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "model_classifier",
          reason: "The request needs bounded local file context.",
          matched_signals: ["local file"],
          status_label: "Preparing local context…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        if (chatTurnCount === 1) {
          return {
            text: [
              "I need to apply the verified file update.",
              "```oomu_mcp_tool_call",
              JSON.stringify({
                serverName: "local_filesystem",
                toolName: "write_file",
                arguments: { path: "notes.txt", content: "Updated" },
              }),
              "```",
            ].join("\n"),
            session_id: "session-1",
          };
        }

        const request = (payload as {
          request: {
            native_execution_receipt_id?: string;
            verified_native_execution_receipt?: boolean;
          };
        }).request;
        expect(request.verified_native_execution_receipt).toBe(true);
        expect(request.native_execution_receipt_id).toBe("apple-operation-write-1");
        const completion = "I've written the requested update to notes.txt.";
        persistedMessages = [
          {
            id: 1,
            sessionId: "session-1",
            role: "user",
            content: "Review my local changelog file.",
            createdAtMs: 1,
          },
          {
            id: 2,
            sessionId: "session-1",
            role: "assistant",
            content: completion,
            providerId: "provider-1",
            modelId: "model-1",
            metadataJson: JSON.stringify({
              verifiedNativeExecutionReceipt: true,
              executingProviderId: "provider-1",
              executingModelId: "model-1",
            }),
            createdAtMs: 2,
          },
        ];
        return {
          text: completion,
          session_id: "session-1",
          metadata: {
            verifiedNativeExecutionReceipt: true,
            executingProviderId: "provider-1",
            executingModelId: "model-1",
          },
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Review my local changelog file." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(
      () => {
        expect(executeTool).toHaveBeenCalledWith(
          "local_filesystem",
          "write_file",
          { path: "notes.txt", content: "Updated" },
          expect.any(Object),
        );
        expect(
          within(view.container).getByText("I've written the requested update to notes.txt."),
        ).toBeInTheDocument();
        expect(hydratedVerifiedCompletion).toBe(true);
      },
      { timeout: 3_000 },
    );
    expect(chatTurnCount).toBe(2);
    expect(
      within(view.container).queryByText(/did not receive a verified native execution receipt/i),
    ).not.toBeInTheDocument();
  });

  it("continues from a genuine broker error instead of leaving the tool request as the final answer", async () => {
    const executeTool = vi.fn(async () => {
      throw Object.assign(new Error("Calendar permission denied."), {
        code: "mcp_permission_required",
      });
    });
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_filesystem: {
          name: "local_filesystem",
          status: "connected",
          tools: [{
            name: "read_file",
            description: "Read a local file",
            inputSchema: { type: "object" },
          }],
        },
      },
    };

    let chatTurnCount = 0;
    let persistedMessages: StoredChatMessage[] = [];
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") return persistedMessages;
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_system_hardware_profile") {
        return {
          physicalMemoryGb: 16,
          processorTier: "Mid (Metal, 16K local context)",
          cpuArch: "aarch64",
          cpuCores: 8,
          osName: "macos",
          metalSupported: true,
          maxLocalContextBudget: 16_384,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "model_classifier",
          reason: "The request may need a connected local tool.",
          matched_signals: ["connected tool"],
          status_label: "Preparing local context…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        if (chatTurnCount === 1) {
          return {
            text: [
              "```oomu_mcp_tool_call",
              JSON.stringify({
                serverName: "local_filesystem",
                toolName: "read_file",
                arguments: { path: "notes.txt" },
              }),
              "```",
            ].join("\n"),
            session_id: "session-1",
          };
        }
        const request = (payload as {
          request: { attachments?: Array<{ text?: string }>; mcp_tool_capabilities?: unknown[] };
        }).request;
        expect(request.attachments?.[0]?.text).toContain("Terminal outcome: permission");
        expect(request.mcp_tool_capabilities).toEqual([]);
        const completion = "I couldn't read that because access was denied.";
        persistedMessages = [{ id: 1, sessionId: "session-1", role: "assistant",
          content: completion, providerId: "provider-1", modelId: "model-1", createdAtMs: 1 }];
        return {
          text: completion,
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Review the connected workspace source." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByText("I couldn't read that because access was denied."),
    ).toBeVisible();
    expect(executeTool).toHaveBeenCalledTimes(1);
    expect(chatTurnCount).toBe(2);
  });

  it("keeps a verified sovereign-search answer terminal when it also contains another tool directive", async () => {
    const query = "Writing AI Prompts for Dummies latest edition";
    const digest = "a".repeat(64);
    const contextJson = JSON.stringify({
      accessedAtUtc: "2026-08-01T20:30:00.000Z",
      pages: [{
        title: "Writing AI Prompts For Dummies",
        url: "https://www.wiley.com/en-us/Writing+AI+Prompts+For+Dummies-p-9781394283126",
        text: "The publication page identifies the first edition.",
      }],
    });
    const executeTool = vi.fn(async (serverName: string, toolName: string) => {
      expect(`${serverName}/${toolName}`).toBe("local_search/search_web");
      return {
        content: [{ type: "text", text: "Verified public search context." }],
        structuredContent: {
          sovereignSearch: {
            query,
            engine: "duckduckgo_lite_static",
            resultCount: 1,
            contextJson,
            degraded: false,
            receiptDigest: digest,
            invocationIndex: 1,
          },
        },
        isError: false,
        _meta: {
          oomuSovereignSearchReceipt: {
            schema: "oomu.sovereign-mcp-search.v1",
            verified: true,
            query,
            engine: "duckduckgo_lite_static",
            resultCount: 1,
            receiptDigest: digest,
            invocationIndex: 1,
          },
        },
      };
    });
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_search: {
          name: "local_search",
          status: "connected",
          tools: [{
            name: "search_web",
            description: "Search public web sources",
            inputSchema: {
              type: "object",
              properties: { query: { type: "string" } },
              required: ["query"],
            },
          }],
        },
        local_filesystem: {
          name: "local_filesystem",
          status: "connected",
          tools: [{
            name: "read_file",
            description: "Read a local file",
            inputSchema: { type: "object" },
          }],
        },
      },
    };

    let chatTurnCount = 0;
    let persistedMessages: StoredChatMessage[] = [];
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") return persistedMessages;
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The model may select a connected public search tool.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        if (chatTurnCount === 1) {
          return {
            text: [
              "I need verified public evidence.",
              "```oomu_mcp_tool_call",
              JSON.stringify({
                serverName: "local_search",
                toolName: "search_web",
                arguments: { query },
              }),
              "```",
            ].join("\n"),
            session_id: "session-1",
          };
        }
        const request = (payload as {
          request: {
            attachments: Array<{
              name: string;
              mime_type: string;
              byte_count: number;
              text: string;
              data_base64?: string;
              approved_file_receipt?: unknown;
              private_data_provenance?: unknown;
            }>;
            steering_only: boolean;
            verified_native_execution_receipt: boolean;
            mcp_tool_capabilities: unknown[];
          };
        }).request;
        expect(request.steering_only).toBe(true);
        expect(request.verified_native_execution_receipt).toBe(false);
        expect(request.mcp_tool_capabilities).toEqual([]);
        expect(request.attachments).toHaveLength(1);
        expect(request.attachments[0]?.name).toBe("local_web_search.md");
        expect(request.attachments[0]?.mime_type).toBe("text/markdown");
        expect(request.attachments[0]?.byte_count).toBe(
          new TextEncoder().encode(request.attachments[0]?.text).byteLength,
        );
        expect(request.attachments[0]?.text).toContain(`Query: ${query}`);
        expect(request.attachments[0]?.text).toContain("Engine: duckduckgo_lite_static");
        expect(request.attachments[0]?.text).toContain(`Native-Receipt: ${digest}`);
        expect(request.attachments[0]?.text).toContain("Invocation-Index: 1");
        expect(request.attachments[0]?.text).toContain("Result-Count: 1");
        expect(request.attachments[0]?.text).toContain(
          "https://www.wiley.com/en-us/Writing+AI+Prompts+For+Dummies-p-9781394283126",
        );
        expect(request.attachments[0]?.private_data_provenance).toBeUndefined();
        expect(request.attachments[0]?.approved_file_receipt).toBeUndefined();
        expect(request.attachments[0]?.data_base64).toBeUndefined();
        expect(request.attachments.map((attachment) => attachment.name)).not.toContain(
          "connector_search_web.json",
        );
        const completion = "The verified publisher evidence identifies the first edition. Source: Wiley.";
        persistedMessages = [{
          id: 2,
          sessionId: "session-1",
          role: "assistant",
          content: completion,
          providerId: "provider-1",
          modelId: "model-1",
          createdAtMs: 2,
        }];
        return {
          text: [
            completion,
            "```oomu_mcp_tool_call",
            JSON.stringify({
              serverName: "local_filesystem",
              toolName: "read_file",
              arguments: { path: "private-notes.md" },
            }),
            "```",
          ].join("\n"),
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Help me answer a factual publishing question." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByText(
        "The verified publisher evidence identifies the first edition. Source: Wiley.",
      ),
    ).toBeVisible();
    expect(executeTool).toHaveBeenCalledTimes(1);
    expect(executeTool).toHaveBeenCalledWith(
      "local_search",
      "search_web",
      { query },
      expect.any(Object),
    );
    expect(chatTurnCount).toBe(2);
    expect(view.container).not.toHaveTextContent("Running read_file");
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "sovereign_duckduckgo_search"),
    ).toHaveLength(0);
    expect(
      invokeMock.mock.calls.filter(([command]) =>
        command === "get_private_egress_confirmation" ||
        command === "resolve_private_egress_confirmation"),
    ).toHaveLength(0);
  });

  it("never executes a conversational MCP request outside the turn's frozen catalog", async () => {
    const executeTool = vi.fn();
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_search: {
          name: "local_search",
          status: "connected",
          tools: [{ name: "search_web", description: "Search public sources", inputSchema: { type: "object" } }],
        },
      },
    };

    let chatTurnCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The request may use the frozen connected-tool catalog.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        return {
          text: [
            "```oomu_mcp_tool_call",
            JSON.stringify({
              serverName: "local_filesystem",
              toolName: "read_file",
              arguments: { path: "private-notes.md" },
            }),
            "```",
          ].join("\n"),
          session_id: "session-1",
        };
      }
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use only the connected tools available to this turn." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(await within(view.container).findByText(
      "Blocked local tool request: local_filesystem/read_file was not available to this turn.",
    )).toBeVisible();
    expect(executeTool).not.toHaveBeenCalled();
    expect(chatTurnCount).toBe(1);
  });

  it("fails a nominal MCP web search closed when its sovereign marker is missing", async () => {
    const query = "Writing AI Prompts for Dummies latest edition";
    const executeTool = vi.fn(async () => ({
      content: [{ type: "text", text: "I found a result." }],
      structuredContent: {
        sovereignSearch: {
          query,
          engine: "duckduckgo_lite_static",
          resultCount: 1,
          contextJson: JSON.stringify({
            accessedAtUtc: "2026-08-01T20:30:00.000Z",
            pages: [{ url: "https://example.com/unverified" }],
          }),
          degraded: false,
          receiptDigest: "a".repeat(64),
          invocationIndex: 1,
        },
      },
      isError: false,
    }));
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_search: {
          name: "local_search",
          status: "connected",
          tools: [{ name: "search_web", description: "Search public web sources", inputSchema: { type: "object" } }],
        },
      },
    };

    let chatTurnCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The model may select a connected public search tool.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        return {
          text: [
            "```oomu_mcp_tool_call",
            JSON.stringify({
              serverName: "local_search",
              toolName: "search_web",
              arguments: { query },
            }),
            "```",
          ].join("\n"),
          session_id: "session-1",
        };
      }
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Help me answer a factual publishing question." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByText("Web search isn't available right now. Try again."),
    ).toBeVisible();
    expect(executeTool).toHaveBeenCalledTimes(1);
    expect(chatTurnCount).toBe(1);
    expect(view.container).not.toHaveTextContent("connector_search_web.json");
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "sovereign_duckduckgo_search"),
    ).toHaveLength(0);
  });

  it("surfaces a native MCP search error without creating a private connector continuation", async () => {
    const query = "Writing AI Prompts for Dummies latest edition";
    const executeTool = vi.fn(async () => {
      throw Object.assign(new Error("Sovereign public search failed."), {
        code: "search_provider_unavailable",
      });
    });
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_search: {
          name: "local_search",
          status: "connected",
          tools: [{ name: "search_web", description: "Search public web sources", inputSchema: { type: "object" } }],
        },
      },
    };

    let chatTurnCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The model may select a connected public search tool.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        return {
          text: [
            "```oomu_mcp_tool_call",
            JSON.stringify({
              serverName: "local_search",
              toolName: "search_web",
              arguments: { query },
            }),
            "```",
          ].join("\n"),
          session_id: "session-1",
        };
      }
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Help me answer a factual publishing question." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByText("Web search isn't available right now. Try again."),
    ).toBeVisible();
    expect(executeTool).toHaveBeenCalledTimes(1);
    expect(chatTurnCount).toBe(1);
    expect(view.container).not.toHaveTextContent("connector_search_web.json");
    expect(
      invokeMock.mock.calls.filter(([command]) =>
        command === "get_private_egress_confirmation" ||
        command === "resolve_private_egress_confirmation"),
    ).toHaveLength(0);
  });

  it("pauses a result-level Mail permission failure for recovery", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_system_hardware_profile") {
        return {
          physicalMemoryGb: 16,
          processorTier: "Mid (Metal, 16K local context)",
          cpuArch: "aarch64",
          cpuCores: 8,
          osName: "macos",
          metalSupported: true,
          maxLocalContextBudget: 16_384,
        };
      }
      if (command === "read_system_emails") {
        return {
          content: [{ type: "text", text: "MCP tool returned a typed error." }],
          structuredContent: { status: "permission_blocked_or_timed_out" },
          isError: true,
        };
      }
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Check my email for anything unread" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "read_system_emails"),
      ).toHaveLength(1);
    });
    expect(
      await within(view.container).findByRole("heading", { name: "Mail access needed" }),
    ).toBeVisible();
    expect(within(view.container).getByRole("button", { name: "Check again" })).toBeEnabled();
    expect(view.container).not.toHaveTextContent("Mail context blocked.");
  });
