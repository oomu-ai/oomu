import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession, StoredChatMessage } from "@/lib/chatSessions";
import { scenarioOneDecisionPackPrompt, scenarioOneDecisionPackSteps } from "./fixtures/scenarioOneDecisionPack";
import { expectReadablePlanPreview } from "./ChatScreen.plan-test-runtime";
import { ChatScreen } from "../ChatScreen";
import {
  agents,
  cloudAgents,
  cloudConfiguredProviders,
  cloudSessions,
  configuredProviders,
  dynamicSessions,
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

function renderChatScreen(sessionFixtures: ChatSession[] = sessions, projectId: string | null = null) {
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

describe("ChatScreen execution boundaries", () => {
  it("resumes the same Project turn after explicit one-message cloud approval", async () => {
    const chatTurnRequests: Array<Record<string, unknown>> = [];
    let chatCompleted = false;
    invokeMock.mockImplementation(
      async (command: string, args?: { request?: Record<string, unknown> }) => {
        if (command === "list_chat_messages") {
          return chatCompleted
            ? [
                {
                  id: 1,
                  sessionId: "session-1",
                  role: "user",
                  content: "Summarize this Project.",
                  createdAtMs: 1,
                },
                {
                  id: 2,
                  sessionId: "session-1",
                  role: "assistant",
                  content: "The Project contains three notes.",
                  createdAtMs: 2,
                },
              ]
            : [];
        }
        if (command === "get_queued_messages") return [];
        if (command === "get_session_config") return null;
        if (command === "get_local_generation_health") return "ready";
        if (command === "classify_chat_intent_route") {
          return {
            route: "conversational_stream",
            requires_local_access: false,
            decision_source: "contextual_informational_topic_filter",
            reason: "Ordinary Project conversation.",
            matched_signals: [],
            status_label: "Thinking…",
          };
        }
        if (command === "chat_turn") {
          chatTurnRequests.push({ ...(args?.request ?? {}) });
          if (chatTurnRequests.length === 1) {
            throw {
              code: "project_provider_consent_required",
              message: "Project cloud approval is required.",
            };
          }
          chatCompleted = true;
          return {
            text: "The Project contains three notes.",
            session_id: "session-1",
          };
        }
        if (command === "list_chat_sessions") return cloudSessions;
        return null;
      },
    );

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={cloudAgents}
        configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        projectId="project_11111111-1111-4111-8111-111111111111"
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Summarize this Project." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByRole("heading", {
        name: "Use the cloud for this Project?",
      }),
    ).toBeVisible();
    expect(within(view.container).getByRole("alert")).toHaveTextContent(
      "OpenAI",
    );
    expect(chatTurnRequests).toHaveLength(1);
    expect(chatTurnRequests[0]).toEqual(
      expect.objectContaining({ project_cloud_confirmed: false }),
    );

    fireEvent.click(
      within(view.container).getByRole("button", { name: "Allow this message" }),
    );

    await waitFor(() => expect(chatTurnRequests).toHaveLength(2));
    await waitFor(() => {
      expect(view.container.textContent).toContain("The Project contains three notes.");
    });
    expect(chatTurnRequests[1]).toEqual(
      expect.objectContaining({
        session_id: chatTurnRequests[0]?.session_id,
        turn_id: chatTurnRequests[0]?.turn_id,
        generation_token: chatTurnRequests[0]?.generation_token,
        provider_id: chatTurnRequests[0]?.provider_id,
        model_id: chatTurnRequests[0]?.model_id,
        project_cloud_confirmed: true,
      }),
    );
    expect(
      invokeMock.mock.calls.some(([command]) => command === "set_project_policy"),
    ).toBe(false);
  });

  it("routes the literal Scenario 1 Test 3 prompt to the planner instead of the Mail-only shortcut", async () => {
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
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
      if (command === "triage_local_app_intent") return true;
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "deterministic_action_rules",
          reason: "This compound request requires files, web research, artifacts, Calendar, and Mail.",
          matched_signals: ["cross-surface task"],
          status_label: "Preparing the decision pack…",
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "scenario-one-plan",
          objective: scenarioOneDecisionPackPrompt,
          steps: scenarioOneDecisionPackSteps,
          exit_condition: "Every requested result is independently verified.",
          trusted_automatic_execution: false,
          model_route: {
            reason: "The request requires bounded local and native-app actions.",
            requires_principal_authorization: true,
          },
        };
      }
      if (command === "record_browser_chat_turn") {
        return {
          text: args?.request?.assistant_text,
          session_id: args?.request?.session_id,
          turn_id: args?.request?.turn_id,
          generation_token: args?.request?.generation_token,
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen(dynamicSessions, "project_11111111-1111-4111-8111-111111111111");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: scenarioOneDecisionPackPrompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "process_agent_objective"),
      ).toBe(true);
      expect(
        invokeMock.mock.calls.some(([command]) => command === "record_browser_chat_turn"),
      ).toBe(true);
    });
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "classify_chat_intent_route"),
    ).toHaveLength(1);
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "process_agent_objective"),
    ).toHaveLength(1);
    expectReadablePlanPreview(view.container, scenarioOneDecisionPackSteps.length);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "execute_system_apple_app_tool"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "prepare_system_apple_app_tool_approval"),
    ).toBe(false);
    const plannerRequest = invokeMock.mock.calls.find(
      ([command]) => command === "process_agent_objective",
    )?.[1]?.request;
    expect(plannerRequest).toEqual(expect.objectContaining({
      user_objective: scenarioOneDecisionPackPrompt,
      selected_provider_id: "dynamic",
      selected_model_id: "dynamic",
      dynamic_routing_enabled: true,
      project_id: "project_11111111-1111-4111-8111-111111111111",
    }));
  });

  it("reconciles a true already-running reply without saving or showing a red failure", async () => {
    let listMessagesCalls = 0;
    let duplicateTurnId = "";
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") {
        listMessagesCalls += 1;
        if (duplicateTurnId) {
          return [{
            id: 2,
            sessionId: "session-1",
            role: "assistant",
            content: "The original reply finished.",
            metadataJson: JSON.stringify({
              terminalResultForTurnId: duplicateTurnId,
              turnState: "completed",
            }),
            createdAtMs: 2,
          }];
        }
        return [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "Ordinary conversational follow-up.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        duplicateTurnId = String(args?.request?.turn_id ?? "");
        throw {
          code: "chat_turn_already_running",
          message: "This message is already being answered.",
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    await waitFor(() => expect(listMessagesCalls).toBeGreaterThan(0));
    const listMessagesCallsBeforeSubmit = listMessagesCalls;
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "What happened to the other requested materials?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
      expect(listMessagesCalls).toBeGreaterThan(listMessagesCallsBeforeSubmit);
    });
    expect(screen.queryByText("This message is already being answered.")).not.toBeInTheDocument();
    expect(
      await screen.findByText("The original reply finished."),
    ).toBeVisible();
    expect(
      invokeMock.mock.calls.some(([command]) => command === "finalize_accepted_chat_turn"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "abandon_accepted_chat_turn"),
    ).toBe(false);
  });

  it("ends a bounded duplicate wait with honest nonpending delayed guidance", async () => {
    reconciliationDelaysMock.value = [0, 0];
    let listMessagesCalls = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        listMessagesCalls += 1;
        return [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "Ordinary conversational follow-up.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        throw {
          code: "chat_turn_already_running",
          message: "This message is already being answered.",
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    await waitFor(() => expect(listMessagesCalls).toBeGreaterThan(0));
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Continue the original reply." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(await screen.findByText(
      "This reply is taking longer than expected. OOMU did not start a duplicate response. Reopen this chat to refresh the result.",
    )).toBeVisible();
    expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
    expect(
      invokeMock.mock.calls.some(([command]) => command === "finalize_accepted_chat_turn"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "abandon_accepted_chat_turn"),
    ).toBe(false);
  });

  it("durably accepts an attachment-only turn with its canonical visible receipt", async () => {
    let acceptedMessage = "";
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "choose_local_context") {
        return {
          results: [{
            name: "decision-brief.txt",
            ok: true,
            grantId: "a".repeat(64),
            mimeType: "text/plain",
            decodedByteCount: 12,
            encodedByteCount: 0,
            expiresAtMs: Date.now() + 60_000,
            errorCode: null,
          }],
          countLimit: 5,
          decodedByteLimit: 20 * 1024 * 1024,
          encodedByteLimit: 28 * 1024 * 1024,
        };
      }
      if (command === "read_local_context") {
        return {
          name: "decision-brief.txt",
          mime_type: "text/plain",
          byte_count: 12,
          text: "Decision data",
          truncated: false,
        };
      }
      if (command === "revoke_local_context_grants") return { revokedCount: 1 };
      if (command === "accept_chat_turn") {
        acceptedMessage = String(args?.request?.message ?? "");
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_local_context_filter",
          reason: "The selected file is already attached.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        return { text: "I reviewed the attached decision brief.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    fireEvent.click(within(view.container).getByRole("button", { name: "Attach file" }));
    await screen.findByText("decision-brief.txt");
    const sendButton = within(view.container).getByRole("button", { name: "Send" });
    await waitFor(() => expect(sendButton).toBeEnabled());
    fireEvent.click(sendButton);

    await waitFor(() => expect(acceptedMessage).toContain("Please review the attached file."));
    expect(acceptedMessage).toContain("decision-brief.txt (text/plain; 12 bytes)");
  });

  it("keeps questions about prior browser behavior in chat without replacing the reply", async () => {
    let recordedAssistantText = "";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return recordedAssistantText
          ? [
              {
                id: 1,
                sessionId: "session-1",
                role: "user",
                content: "Why did you open the browser panel?",
                createdAtMs: 1,
              },
              {
                id: 2,
                sessionId: "session-1",
                role: "assistant",
                content: recordedAssistantText,
                createdAtMs: 2,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_session_config") return null;
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          confidence: 1,
          reason: "No executable action was requested.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        recordedAssistantText = "I opened it because I incorrectly interpreted your earlier message.";
        return { text: recordedAssistantText, session_id: "session-1" };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health")).toBe(true);
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Why did you open the browser panel?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await within(view.container).findByText(
      "I opened it because I incorrectly interpreted your earlier message.",
    );
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
    expect(
      within(view.container).queryByText(
        "Action claim blocked: OOMU did not receive a verified native execution receipt.",
      ),
    ).not.toBeInTheDocument();
  });

});

describe("ChatScreen local execution boundaries", () => {
  it("keeps a Calendar read local when Search is off", async () => {
    const prompt = "Check my calendar and let me know what I have going on today";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_session_config") return null;
      if (command === "read_system_calendar") {
        return {
          content: [{ type: "text", text: "No calendar events were found for today." }],
          structuredContent: { events: [] },
          isError: false,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: true,
          decision_source: "deterministic_action_rules",
          confidence: 1,
          reason: "The request uses the native Calendar reader.",
          matched_signals: ["calendar"],
          status_label: "Reading Calendar…",
        };
      }
      if (command === "chat_turn") {
        return { text: "You have no events on your calendar today.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "read_system_calendar")).toBe(
        true,
      );
    });
    expect(
      invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
    ).toBe(false);
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
  });

  it("honors a user-authored public search with ambient Search disabled and private attachments present", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_session_config") return null;
      if (command === "choose_local_context") return {
        results: [{
          name: "private-plan.txt", ok: true, grantId: "a".repeat(64),
          mimeType: "text/plain", decodedByteCount: 12, encodedByteCount: 0,
          expiresAtMs: Date.now() + 60_000, errorCode: null,
        }],
        countLimit: 5,
        decodedByteLimit: 20 * 1024 * 1024,
        encodedByteLimit: 28 * 1024 * 1024,
      };
      if (command === "read_local_context") return {
        name: "private-plan.txt", mime_type: "text/plain", byte_count: 12,
        text: "Private data", truncated: false,
      };
      if (command === "revoke_local_context_grants") return { revokedCount: 1 };
      if (command === "sovereign_duckduckgo_search") {
        return {
          query: "the next time the Red Sox are playing in Boston",
          engine: "duckduckgo_lite_static",
          resultCount: 1,
          results: [{
            title: "Red Sox schedule",
            url: "https://example.com/red-sox-schedule",
            snippet: "Observed public search context.",
          }],
          contextJson: "[]",
          retrievalElapsedMs: 1,
          degraded: false,
          security: {
            apiKeyRequired: false,
            cookiesEnabled: false,
            browserAutomationEnabled: false,
            proxyEnvironmentEnabled: false,
            endpointAllowlist: ["lite.duckduckgo.com"],
          },
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "No enabled external context source.",
          matched_signals: [],
          status_label: "Thinking…",
        };
      }
      if (command === "chat_turn") {
        return { text: "Search is not enabled for this chat.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health"),
      ).toBe(true);
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Attach file" }));
    await within(view.container).findByText("private-plan.txt");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Take a look online and find out the next time the Red Sox are playing in Boston" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
      ).toBe(true);
    });
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
  });

  it("sends the immutable originating utterance with the stripped sovereign-search query", async () => {
    const prompt = "Search the web for lunar eclipses";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_session_config") return null;
      if (command === "sovereign_duckduckgo_search") {
        return {
          query: "lunar eclipses",
          engine: "duckduckgo_lite_static",
          resultCount: 1,
          results: [{
            title: "Lunar eclipses",
            url: "https://example.com/lunar-eclipses",
            snippet: "Observed public search context.",
          }],
          contextJson: "[]",
          retrievalElapsedMs: 1,
          degraded: false,
          security: {
            apiKeyRequired: false,
            cookiesEnabled: false,
            browserAutomationEnabled: false,
            proxyEnvironmentEnabled: false,
            endpointAllowlist: ["lite.duckduckgo.com"],
          },
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_web_grounding_filter",
          reason: "Verified local search context is attached.",
          matched_signals: ["hydrated web grounding"],
          status_label: "Reading sources…",
        };
      }
      if (command === "chat_turn") {
        return { text: "A lunar eclipse occurs when Earth blocks sunlight from the Moon.", session_id: "session-1" };
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
        privacySettings={{
          automatedWebGroundingEnabled: true,
          licenseAccepted: true,
          licenseState: "accepted",
          licenseVersion: "test",
          licenseEffectiveDate: "2026-01-01",
          licenseText: "Test license",
        }}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health"),
      ).toBe(true);
    });
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
      ).toBe(true);
    });
    const searchCall = invokeMock.mock.calls.find(
      ([command]) => command === "sovereign_duckduckgo_search",
    );
    expect(searchCall?.[1]).toEqual({
      request: {
        query: "lunar eclipses",
        originatingUtterance: prompt,
        maxResults: 5,
        sessionId: "session-1",
        modId: undefined,
        originTurnId: expect.any(String),
        originGenerationToken: expect.any(String),
      },
    });
  });

  it("keeps an exact Contacts search native when backend triage is unavailable", async () => {
    tauriRuntimeMock.value = true;
    const requestText = "Search my contacts and see if you can find Maya Allan";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return [];
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
      if (command === "triage_local_app_intent") {
        throw new Error("triage command unavailable");
      }
      if (command === "prepare_system_apple_app_tool_approval") return null;
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{
            type: "text",
            text: JSON.stringify([{
              displayName: "Maya Allan",
              emails: ["maya@example.com"],
              phones: [],
            }]),
          }],
          structuredContent: {
            backend: "contacts",
            code: "contacts_read_ok",
            authorization: "authorized",
            searchText: "Maya Allan",
            contacts: [{
              displayName: "Maya Allan",
              emails: ["maya@example.com"],
              phones: [],
            }],
            returnedCount: 1,
            truncated: false,
          },
          isError: false,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The protected Contacts result is already attached.",
          matched_signals: [],
          status_label: "OOMU is typing...",
        };
      }
      if (command === "chat_turn") {
        return {
          text: "I found Maya Allan in your contacts.",
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });

    const contactsScreen = () => (
      <StrictMode>
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
        />
      </StrictMode>
    );
    const view = render(
      contactsScreen(),
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: requestText },
    });
    const sendButton = within(view.container).getByRole("button", { name: "Send" });
    fireEvent.click(sendButton);
    view.rerender(contactsScreen());
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "chat_turn"),
      ).toHaveLength(1);
    });
    const executeCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "execute_system_apple_app_tool",
    );
    expect(executeCalls).toHaveLength(1);
    expect(executeCalls[0]?.[1]).toEqual(expect.objectContaining({
      toolName: "read_system_contacts",
      arguments: {
        max_contacts: 20,
        search_text: "Maya Allan",
      },
    }));
    const chatTurnCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "chat_turn",
    );
    const nativeTurnContext = executeCalls[0]?.[1]?.turnContext;
    const chatRequest = chatTurnCalls[0]?.[1]?.request;
    expect(chatRequest).toEqual(expect.objectContaining({
      turn_id: nativeTurnContext?.turnId,
      generation_token: nativeTurnContext?.generationToken,
      session_id: nativeTurnContext?.sessionId,
      agent_id: nativeTurnContext?.agentId,
    }));
    expect(chatTurnCalls).toHaveLength(1);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "process_agent_objective"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
    ).toBe(false);
  });

});

describe("ChatScreen continuation execution boundaries", () => {
  it("summarizes a verified Contacts result without planning while the local model hydrates", async () => {
    tauriRuntimeMock.value = true;
    const requestText = "Search my contacts and see if you can find Maya Allan";
    let chatCompleted = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return chatCompleted
          ? [{
              id: 1,
              sessionId: "session-1",
              role: "assistant",
              content: "I found Maya Allan in your contacts.",
              createdAtMs: 1,
            }]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "loading";
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
      if (command === "triage_local_app_intent") return true;
      if (command === "prepare_system_apple_app_tool_approval") return null;
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{
            type: "text",
            text: JSON.stringify([{
              displayName: "Maya Allan",
              emails: ["maya@example.com"],
              phones: [],
            }]),
          }],
          structuredContent: {
            backend: "contacts",
            code: "contacts_read_ok",
            authorization: "authorized",
            searchText: "Maya Allan",
            contacts: [{
              displayName: "Maya Allan",
              emails: ["maya@example.com"],
              phones: [],
            }],
            returnedCount: 1,
            truncated: false,
          },
          isError: false,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "contextual_informational_topic_filter",
          reason: "The protected Contacts result is already attached.",
          matched_signals: [],
          status_label: "OOMU is typing...",
        };
      }
      if (command === "process_agent_objective") {
        throw new Error("A verified Contacts result must not enter the planner.");
      }
      if (command === "chat_turn") {
        chatCompleted = true;
        return {
          text: "I found Maya Allan in your contacts.",
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = renderChatScreen();
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health"),
      ).toBe(true);
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: requestText },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await within(view.container).findByText("I found Maya Allan in your contacts."),
    ).toBeVisible();
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "execute_system_apple_app_tool"),
    ).toHaveLength(1);
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "classify_chat_intent_route"),
    ).toHaveLength(1);
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "chat_turn"),
    ).toHaveLength(1);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "process_agent_objective"),
    ).toBe(false);
  });

  it("keeps ambiguous technical Contacts wording conversational during a triage outage", async () => {
    tauriRuntimeMock.value = true;
    const requestText = "Search contacts in the mesh graph";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return [];
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
      if (command === "triage_local_app_intent") {
        throw new Error("triage command unavailable");
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "unexpected_planner_route",
          reason: "This route must be ignored by the frontend outage guard.",
          matched_signals: ["search"],
          status_label: "Planning",
        };
      }
      if (command === "chat_turn") {
        return {
          text: "That sounds like a technical graph topic.",
          session_id: "session-1",
          route_escalation: {
            route: "agentic_planner",
            requires_local_access: true,
            decision_source: "unexpected_stream_escalation",
            reason: "This escalation must also remain suppressed.",
            matched_signals: ["search"],
            status_label: "Planning",
          },
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "unexpected-plan",
          objective: requestText,
          steps: [],
        };
      }
      if (command === "sovereign_duckduckgo_search") {
        return { ok: true, query: requestText, results: [] };
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
      target: { value: requestText },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "chat_turn"),
      ).toHaveLength(1);
    });
    const chatTurnCall = invokeMock.mock.calls.find(([command]) => command === "chat_turn");
    expect(chatTurnCall?.[1]?.request).toEqual(expect.objectContaining({
      automated_web_grounding_enabled: false,
      mcp_tool_capabilities: [],
    }));
    for (const forbiddenCommand of [
      "classify_chat_intent_route",
      "execute_system_apple_app_tool",
      "prepare_system_apple_app_tool_approval",
      "process_agent_objective",
      "sovereign_duckduckgo_search",
    ]) {
      expect(
        invokeMock.mock.calls.some(([command]) => command === forbiddenCommand),
        forbiddenCommand,
      ).toBe(false);
    }
  });

  it("does not turn a rejected private-app plan into an ungrounded chat answer", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "triage_local_app_intent") return false;
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "private_app_data_filter",
          reason: "Contacts require a protected native read.",
          matched_signals: ["private contacts request"],
          status_label: "Checking Contacts...",
        };
      }
      if (command === "process_agent_objective") {
        throw Object.assign(new Error("Private reads do not use ActionPlan."), {
          code: "agent_objective_not_executable",
        });
      }
      if (command === "chat_turn") {
        return { text: "Maya Allan is in your contacts.", session_id: "session-1" };
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
      target: { value: "Search my contacts for Maya Allan" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(
      await screen.findByText(
        "OOMU couldn't prepare this request safely on your Mac. Nothing was changed. Try again.",
      ),
    ).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "chat_turn")).toHaveLength(0);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "execute_system_apple_app_tool"),
    ).toBe(false);
    expect(screen.queryByText("Maya Allan is in your contacts.")).not.toBeInTheDocument();
    expect(screen.queryByText(/agent_objective_not_executable/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/ActionPlan/i)).not.toBeInTheDocument();
  });

});
