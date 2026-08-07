import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import { agents, configuredProviders, dynamicSessions } from "./ChatScreen.execution-boundaries.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
    }
    if (command === "chat_turn" && response && typeof response === "object") {
      return {
        ...response,
        session_id: response.session_id ?? args?.request?.session_id,
        turn_id: response.turn_id ?? args?.request?.turn_id,
        generation_token: response.generation_token ?? args?.request?.generation_token,
      };
    }
    return response;
  },
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class TestChannel {} }));
vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: null,
    fallbackRoute: null,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

const projectId = "project_11111111-1111-4111-8111-111111111111";
const projectSessions = [
  { ...dynamicSessions[0], projectId },
  {
    ...dynamicSessions[0],
    id: "session-2",
    title: "Second chat",
    projectId,
    updatedAtMs: 2,
  },
];

function chatScreen(activeSessionId: string) {
  return <ChatScreen activeSessionId={activeSessionId} agents={agents} configuredProviders={configuredProviders} onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()} onSessionsChange={vi.fn()} privacySettings={null} projectId={projectId} sessions={projectSessions} />;
}

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

afterEach(() => cleanup());

it("keeps the Project document turn visible across chats and executes once", async () => {
  let resolveChatTurn: ((response: Record<string, unknown>) => void) | null = null;
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "get_local_generation_health") return "ready";
    if (command === "get_auto_route_session_readiness")
      return {
        status: "ready",
        sessionId: "session-1",
        dynamicBindingValid: true,
        classifierModelId: "model-1",
        classifierReady: true,
        localProviderId: "provider-1",
        localProviderType: "local_model",
        localModelId: "model-1",
        routeGeneration: 1,
        localModelReady: true,
        recommendedLocalProviderId: null,
        recommendedLocalModelId: null,
        contextBudgetValid: true,
        cloudTargetRequired: false,
        cloudTargetReady: true,
        storageReady: true,
        auditReady: true,
        readinessGeneration: 1,
        lastVerifiedAtMs: Date.now(),
        failureCode: null,
        failureBoundary: null,
      };
    if (command === "chat_turn")
      return new Promise((resolve) => {
        resolveChatTurn = resolve;
      });
    if (command === "create_project_chat_document") return { artifactId: "artifact-1", version: 1 };
    if (command === "list_chat_sessions") return projectSessions;
    return null;
  });
  const view = render(chatScreen("session-1"), { wrapper: I18nProvider });
  await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "get_auto_route_session_readiness")).toBe(true));
  fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
    target: {
      value: "Using only the files in this Project, prepare a two-page quarterly program update. Produce an editable Word document and a PDF.",
    },
  });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
  expect(await screen.findByText("OOMU is thinking…")).toBeInTheDocument();
  await waitFor(() => expect(resolveChatTurn).not.toBeNull());
  expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
  const request = invokeMock.mock.calls.find(([command]) => command === "chat_turn")?.[1]?.request;
  expect(request).toEqual(
    expect.objectContaining({
      provider_id: "provider-1",
      model_id: "model-1",
      dynamic_routing_override: true,
      auto_route_choice: "local",
      project_document_composition: true,
      mcp_tool_capabilities: [],
    }),
  );
  view.rerender(chatScreen("session-2"));
  expect(within(document.getElementById("oomu-chat-session-session-1")!).getByRole("status", { name: "OOMU is thinking…" })).toBeInTheDocument();
  const finishTurn = resolveChatTurn as ((response: Record<string, unknown>) => void) | null;
  finishTurn!({
    text: "# Quarterly Program Update\n\nEvidence-backed results.",
  });
  await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "create_project_chat_document")).toBe(true));
  await waitFor(() => expect(within(document.getElementById("oomu-chat-session-session-1")!).queryByRole("status", { name: "OOMU is thinking…" })).not.toBeInTheDocument());
});

it("uses the Project attached to the selected chat when the screen has no Project prop", async () => {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "get_local_generation_health") return "ready";
    if (command === "get_auto_route_session_readiness")
      return {
        status: "ready",
        sessionId: "session-1",
        dynamicBindingValid: true,
        classifierModelId: "model-1",
        classifierReady: true,
        localProviderId: "provider-1",
        localProviderType: "local_model",
        localModelId: "model-1",
        routeGeneration: 1,
        localModelReady: true,
        recommendedLocalProviderId: null,
        recommendedLocalModelId: null,
        contextBudgetValid: true,
        cloudTargetRequired: false,
        cloudTargetReady: true,
        storageReady: true,
        auditReady: true,
        readinessGeneration: 1,
        lastVerifiedAtMs: Date.now(),
        failureCode: null,
        failureBoundary: null,
      };
    if (command === "chat_turn") return { text: "# Quarterly Program Update\n\nEvidence." };
    if (command === "create_project_chat_document") return { artifactId: "artifact-2", version: 1 };
    if (command === "list_chat_sessions") return projectSessions;
    return null;
  });
  const view = render(<ChatScreen activeSessionId="session-1" agents={agents} configuredProviders={configuredProviders} onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()} onSessionsChange={vi.fn()} privacySettings={null} sessions={projectSessions} />, { wrapper: I18nProvider });
  await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "get_auto_route_session_readiness")).toBe(true));
  fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
    target: {
      value: "Using only the files in this Project, prepare a quarterly update. Produce an editable Word document and a PDF.",
    },
  });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

  await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true));
  const request = invokeMock.mock.calls.find(([command]) => command === "chat_turn")?.[1]?.request;
  expect(request).toEqual(
    expect.objectContaining({
      provider_id: "provider-1",
      model_id: "model-1",
      dynamic_routing_override: true,
      auto_route_choice: "local",
      project_document_composition: true,
      mcp_tool_capabilities: [],
    }),
  );
});

it("does not present a persisted accepted marker as live work without an active owner", async () => {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages")
      return [
        {
          id: 1,
          sessionId: "session-1",
          role: "user",
          content: "Old request",
          metadataJson: JSON.stringify({
            turnId: "stale-turn",
            turnState: "accepted",
          }),
          createdAtMs: 1,
        },
      ];
    if (command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "get_local_generation_health") return "ready";
    if (command === "list_chat_sessions") return projectSessions;
    return null;
  });
  render(chatScreen("session-1"), { wrapper: I18nProvider });
  await screen.findByText("Old request");
  expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
  expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
});

it("keeps a document-creation failure visible after the transient draft is removed", async () => {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "get_local_generation_health") return "ready";
    if (command === "get_auto_route_session_readiness")
      return {
        status: "ready",
        sessionId: "session-1",
        dynamicBindingValid: true,
        classifierModelId: "model-1",
        classifierReady: true,
        localProviderId: "provider-1",
        localProviderType: "local_model",
        localModelId: "model-1",
        routeGeneration: 1,
        localModelReady: true,
        recommendedLocalProviderId: null,
        recommendedLocalModelId: null,
        contextBudgetValid: true,
        cloudTargetRequired: false,
        cloudTargetReady: true,
        storageReady: true,
        auditReady: true,
        readinessGeneration: 1,
        lastVerifiedAtMs: Date.now(),
        failureCode: null,
        failureBoundary: null,
      };
    if (command === "chat_turn") return { text: "# Quarterly Program Update\n\nDraft." };
    if (command === "create_project_chat_document") {
      throw {
        code: "file_creation_failed",
        message: "The document receipt could not be verified.",
      };
    }
    if (command === "list_chat_sessions") return projectSessions;
    return null;
  });
  const view = render(chatScreen("session-1"), { wrapper: I18nProvider });
  await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "get_auto_route_session_readiness")).toBe(true));
  fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
    target: {
      value: "Using only the files in this Project, prepare a two-page quarterly program update. Produce an editable Word document and a PDF.",
    },
  });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

  expect(await screen.findByText("OOMU couldn’t create and check this file. Nothing was changed. Try a different file name or format.")).toBeInTheDocument();
  expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
  expect(screen.queryByText("# Quarterly Program Update")).not.toBeInTheDocument();
});
