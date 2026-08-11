import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import {
  agents,
  cloudAgents,
  cloudConfiguredProviders,
  cloudSessions,
  configuredProviders,
  sessions,
} from "./ChatScreen.execution-boundaries.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const optionalMcpMock = vi.hoisted(() => ({
  value: null as null | {
    cancelRemoteOperations: () => Promise<number>;
    executeTool: ReturnType<typeof vi.fn>;
    servers: Record<string, unknown>;
  },
}));
vi.mock("@/hooks/useMcp", () => ({ useOptionalMcp: () => optionalMcpMock.value }));
vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
    }
    if (command === "chat_turn" && response && typeof response === "object") {
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
  isTauriRuntime: true,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class TestChannel {} }));
vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: null,
    fallbackRoute: null,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

function renderCloudChat() {
  return render(
    <ChatScreen
      activeSessionId="session-1"
      agents={cloudAgents}
      configuredProviders={cloudConfiguredProviders}
      onCreateSession={vi.fn()}
      onDeleteSession={vi.fn()}
      onSelectSession={vi.fn()}
      onSessionsChange={vi.fn()}
      privacySettings={null}
      sessions={cloudSessions}
    />,
    { wrapper: I18nProvider },
  );
}

function conversationalRoute() {
  return {
    route: "conversational_stream",
    requires_local_access: false,
    decision_source: "contextual_informational_topic_filter",
    reason: "Summarize private context.",
    matched_signals: [],
    status_label: "Thinking…",
  };
}

function autoRouteConsentImplementation(
  chatTurnRequests: Array<Record<string, unknown>>,
  dynamicSessions: typeof sessions,
) {
  return async (command: string, args?: { request?: Record<string, unknown> }) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return {
      localProviderConfigId: "provider-1",
      localProviderType: "local",
      modelId: "model-1",
      reasoningDepth: "medium",
      contextBudget: 4096,
      localRouteGeneration: 1,
    };
    if (command === "get_local_generation_health") return "ready";
    if (command === "get_auto_route_session_readiness") return null;
    if (command === "classify_chat_intent_route") return conversationalRoute();
    if (command === "chat_turn") {
      chatTurnRequests.push({ ...(args?.request ?? {}) });
      if (chatTurnRequests.length === 1) {
        throw { code: "private_egress_confirmation_required" };
      }
      return { text: "The approved cloud analysis is complete.", session_id: "session-1" };
    }
    if (command === "get_private_egress_confirmation") return {
      challengeId: "challenge-auto-route",
      destinationProviderId: "cloud-provider-1",
      destinationModelId: "gpt-5.5",
      sourceNames: ["supplier_proposals.json", "requirements.txt"],
    };
    if (command === "resolve_private_egress_confirmation") return { decision: "approved" };
    if (command === "list_chat_sessions") return dynamicSessions;
    return null;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
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
afterEach(cleanup);

describe("ChatScreen private attachment approval continuation", () => {
  it("shows a one-reply choice and resumes the exact cloud turn", async () => {
    const chatTurnRequests: Array<Record<string, unknown>> = [];
    let chatCompleted = false;
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return chatCompleted ? [{
        id: 2, sessionId: "session-1", role: "assistant",
        content: "The private plan is concise.", createdAtMs: 2,
      }] : [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "choose_local_context") return {
        results: [{ name: "private-plan.md", ok: true, grantId: "e".repeat(64),
          mimeType: "text/markdown", decodedByteCount: 14, encodedByteCount: 0,
          expiresAtMs: Date.now() + 60_000, errorCode: null }],
        countLimit: 5, decodedByteLimit: 20 * 1024 * 1024,
        encodedByteLimit: 28 * 1024 * 1024,
      };
      if (command === "read_local_context") return {
        name: "private-plan.md", mime_type: "text/markdown", byte_count: 14,
        text: "# Private plan", truncated: false,
      };
      if (command === "revoke_local_context_grants") return { revokedCount: 1 };
      if (command === "classify_chat_intent_route") return conversationalRoute();
      if (command === "chat_turn") {
        chatTurnRequests.push({ ...(args?.request ?? {}) });
        if (chatTurnRequests.length === 1) {
          throw { code: "private_egress_confirmation_required" };
        }
        chatCompleted = true;
        return { text: "The private plan is concise.", session_id: "session-1" };
      }
      if (command === "get_private_egress_confirmation") return {
        challengeId: "challenge-1", destinationProviderId: "openai",
        destinationModelId: "gpt-4o-mini", sourceNames: ["private-plan.md"],
        expiresAtMs: Date.now() + 30_000, decision: "pending",
      };
      if (command === "resolve_private_egress_confirmation") return { decision: "approved" };
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });

    const view = renderCloudChat();
    fireEvent.click(within(view.container).getByRole("button", { name: "Attach file" }));
    await within(view.container).findByText("private-plan.md");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Summarize the attached plan." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    const consent = await within(view.container).findByRole("alert");
    expect(within(consent).getByRole("heading")).toHaveTextContent("Send private information to");
    expect(consent).toHaveTextContent("private-plan.md");
    fireEvent.click(within(view.container).getByRole("button", { name: "Send once" }));

    await waitFor(() => expect(chatTurnRequests).toHaveLength(2));
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      session_id: chatTurnRequests[0]?.session_id,
      turn_id: chatTurnRequests[0]?.turn_id,
      generation_token: chatTurnRequests[0]?.generation_token,
    }));
    expect(invokeMock).toHaveBeenCalledWith("resolve_private_egress_confirmation", {
      request: expect.objectContaining({ challengeId: "challenge-1", approved: true }),
    });
    await waitFor(() => expect(view.container).toHaveTextContent("The private plan is concise."));
  });

  it("pins an approved Auto-route continuation to the same cloud turn", async () => {
    const chatTurnRequests: Array<Record<string, unknown>> = [];
    const dynamicSessions = [{
      ...sessions[0],
      providerId: "dynamic",
      modelId: "dynamic",
      dynamicRoutingOverride: true,
    }];
    const providers = [
      configuredProviders[0],
      { ...cloudConfiguredProviders[0], autoRouteTarget: true },
    ];
    invokeMock.mockImplementation(autoRouteConsentImplementation(
      chatTurnRequests,
      dynamicSessions,
    ));

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={dynamicSessions}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Reconcile the approved supplier evidence." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    fireEvent.click(await within(view.container).findByRole("button", { name: "Send once" }));

    await waitFor(() => expect(chatTurnRequests).toHaveLength(2));
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      turn_id: chatTurnRequests[0]?.turn_id,
      generation_token: chatTurnRequests[0]?.generation_token,
      session_id: chatTurnRequests[0]?.session_id,
      auto_route_choice: "cloud",
      auto_route_cloud_confirmed: true,
    }));
    expect(invokeMock.mock.calls.filter(([command]) =>
      command === "resolve_private_egress_confirmation"
    )).toHaveLength(1);
  });
});

describe("ChatScreen private attachment local-only outcome", () => {
  it("ends the reply cleanly when private information stays on the Mac", async () => {
    let chatTurnCalls = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return conversationalRoute();
      if (command === "chat_turn") {
        chatTurnCalls += 1;
        throw { code: "private_egress_confirmation_required" };
      }
      if (command === "get_private_egress_confirmation") return {
        challengeId: "challenge-decline", destinationProviderId: "openai",
        destinationModelId: "gpt-4o-mini", sourceNames: ["private-notes.md"],
        expiresAtMs: Date.now() + 30_000, decision: "pending",
      };
      if (command === "resolve_private_egress_confirmation") return { decision: "denied" };
      return null;
    });
    const view = renderCloudChat();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Summarize my private notes." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    fireEvent.click(await within(view.container).findByRole("button", { name: "Keep on this Mac" }));

    await waitFor(() => expect(view.container).toHaveTextContent(
      "Your private information stayed on this Mac. Nothing was sent.",
    ));
    expect(chatTurnCalls).toBe(1);
    expect(within(view.container).queryByText("OOMU is thinking…")).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("resolve_private_egress_confirmation", {
      request: expect.objectContaining({ challengeId: "challenge-decline", approved: false }),
    });
  });
});

describe("ChatScreen private MCP approval continuation", () => {
  it("resumes the exact steered turn after approving private MCP tool context", async () => {
    const executeTool = vi.fn(async () => ({
      content: [{ type: "text", text: "Private connected result" }],
      structuredContent: { content: "Private connected result" },
      isError: false,
    }));
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool,
      servers: {
        local_filesystem: {
          name: "local_filesystem",
          status: "connected",
          tools: [{ name: "read_file", description: "Read a local file", inputSchema: { type: "object" } }],
        },
      },
    };

    const chatTurnRequests: Array<Record<string, unknown>> = [];
    let persistedMessages: Array<{
      id: number;
      sessionId: string;
      role: string;
      content: string;
      createdAtMs: number;
    }> = [];
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return persistedMessages;
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return conversationalRoute();
      if (command === "chat_turn") {
        chatTurnRequests.push({ ...(args?.request ?? {}) });
        if (chatTurnRequests.length === 1) {
          return {
            text: [
              "```oomu_mcp_tool_call",
              JSON.stringify({
                serverName: "local_filesystem",
                toolName: "read_file",
                arguments: { path: "private-plan.md" },
              }),
              "```",
            ].join("\n"),
            session_id: "session-1",
          };
        }
        if (chatTurnRequests.length === 2) {
          throw { code: "private_egress_confirmation_required" };
        }
        persistedMessages = [{
          id: 2,
          sessionId: "session-1",
          role: "assistant",
          content: "The connected private result was summarized.",
          createdAtMs: 2,
        }];
        return { text: "The connected private result was summarized.", session_id: "session-1" };
      }
      if (command === "get_private_egress_confirmation") return {
        challengeId: "challenge-steered",
        destinationProviderId: "openai",
        destinationModelId: "gpt-4o-mini",
        sourceNames: ["connector_read_file.json"],
        expiresAtMs: Date.now() + 30_000,
        decision: "pending",
      };
      if (command === "resolve_private_egress_confirmation") return { decision: "approved" };
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });

    const view = renderCloudChat();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use my connected private source." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const consent = await within(view.container).findByRole("alert");
    expect(consent).toHaveTextContent("connector_read_file.json");
    fireEvent.click(within(view.container).getByRole("button", { name: "Send once" }));

    await waitFor(() => expect(chatTurnRequests).toHaveLength(3));
    expect(chatTurnRequests[2]).toEqual(expect.objectContaining({
      session_id: chatTurnRequests[1]?.session_id,
      turn_id: chatTurnRequests[1]?.turn_id,
      generation_token: chatTurnRequests[1]?.generation_token,
      steering_only: true,
    }));
    expect(invokeMock).toHaveBeenCalledWith("resolve_private_egress_confirmation", {
      request: expect.objectContaining({ challengeId: "challenge-steered", approved: true }),
    });
    expect(
      await within(view.container).findByText("The connected private result was summarized."),
    ).toBeVisible();
  });
});

describe("ChatScreen private MCP durable cancellation", () => {
  it("durably cancels a steered private continuation when the user keeps it on the Mac", async () => {
    optionalMcpMock.value = {
      cancelRemoteOperations: vi.fn(async () => 0),
      executeTool: vi.fn(async () => ({
        content: [{ type: "text", text: "Private connected result" }],
        structuredContent: { content: "Private connected result" },
        isError: false,
      })),
      servers: {
        local_filesystem: {
          name: "local_filesystem",
          status: "connected",
          tools: [{ name: "read_file", description: "Read a local file", inputSchema: { type: "object" } }],
        },
      },
    };

    let chatTurnCalls = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return conversationalRoute();
      if (command === "chat_turn") {
        chatTurnCalls += 1;
        if (chatTurnCalls === 1) {
          return {
            text: [
              "```oomu_mcp_tool_call",
              JSON.stringify({
                serverName: "local_filesystem",
                toolName: "read_file",
                arguments: { path: "private-plan.md" },
              }),
              "```",
            ].join("\n"),
            session_id: "session-1",
          };
        }
        throw { code: "private_egress_confirmation_required" };
      }
      if (command === "get_private_egress_confirmation") return {
        challengeId: "challenge-steered-decline",
        destinationProviderId: "openai",
        destinationModelId: "gpt-4o-mini",
        sourceNames: ["connector_read_file.json"],
        expiresAtMs: Date.now() + 30_000,
        decision: "pending",
      };
      if (command === "resolve_private_egress_confirmation") return { decision: "denied" };
      if (command === "finalize_accepted_chat_turn") return 2;
      return null;
    });

    const view = renderCloudChat();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use my connected private source." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    fireEvent.click(
      await within(view.container).findByRole("button", { name: "Keep on this Mac" }),
    );

    await waitFor(() => expect(view.container).toHaveTextContent(
      "Your private information stayed on this Mac. Nothing was sent.",
    ));
    expect(chatTurnCalls).toBe(2);
    expect(invokeMock).toHaveBeenCalledWith("finalize_accepted_chat_turn", {
      request: expect.objectContaining({ status: "cancelled", role: "system" }),
    });
    expect(invokeMock).toHaveBeenCalledWith("resolve_private_egress_confirmation", {
      request: expect.objectContaining({ challengeId: "challenge-steered-decline", approved: false }),
    });
  });
});
