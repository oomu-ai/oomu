import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession } from "@/lib/chatSessions";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { ChatScreen, type ChatAgent } from "../ChatScreen";

const invokeMock = vi.hoisted(() => vi.fn());

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
  isTauriRuntime: true,
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

const agents: ChatAgent[] = [
  {
    id: "agent-1",
    name: "OOMU",
    description: "Test agent",
    endpoint: { provider: "provider-1", modelId: "model-1" },
  },
];

const configuredProviders: ConfiguredProvider[] = [
  {
    id: "provider-1",
    providerId: "local",
    providerName: "Local",
    authMethod: "api_key",
    baseUrl: "",
    apiKeyLabel: "",
    customModelIds: "model-1",
  },
];

const sessions: ChatSession[] = [
  {
    id: "session-1",
    agentId: "agent-1",
    title: "Execution boundaries",
    providerId: "provider-1",
    modelId: "model-1",
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
];

describe("ChatScreen verified write boundary", () => {
  beforeEach(() => {
    invokeMock.mockReset();
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

  it("does not claim direct host file creation when Shield cannot verify the write", async () => {
    let recordedAssistantText = "";
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        return recordedAssistantText
          ? [
              {
                id: 20,
                sessionId: "session-1",
                role: "user",
                content:
                  'Create a new markdown file in my Downloads directory called Hello World.md with the content "Hello World".',
                createdAtMs: 20,
              },
              {
                id: 21,
                sessionId: "session-1",
                role: "assistant",
                content: recordedAssistantText,
                createdAtMs: 21,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "execute_command") {
        return {
          operation: "file_write",
          status: "completed",
          message:
            "Shield Gate approved the file write request for /Users/example/Downloads/Hello World.md, but could not verify that the final file contents match the requested content.",
          claims: [],
          verified: false,
        };
      }
      if (command === "record_browser_chat_turn") {
        recordedAssistantText = (payload as { request: { assistant_text: string } }).request
          .assistant_text;
        return { text: recordedAssistantText, session_id: "session-1" };
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
      target: {
        value:
          'Create a new markdown file in my Downloads directory called Hello World.md with the content "Hello World".',
      },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(screen.getByText(/Local command failed/)).toBeInTheDocument();
    });

    expect(
      within(view.container).queryByText(/^Shield Gate approved and wrote/),
    ).not.toBeInTheDocument();
    expect(recordedAssistantText).toMatch(/^Local command failed\./);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });
});
