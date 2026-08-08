import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import { agents, configuredProviders, sessions } from "./ChatScreen.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
    }
    return response;
  },
  isTauriRuntime: false,
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

describe("ChatScreen Project scope", () => {
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

  it("makes Project chat scope explicit and offers a global-chat escape", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_project") return { name: "OOMU Test Project" };
      return null;
    });
    const onStartGlobalChat = vi.fn();
    render(
      <ChatScreen
        activeSessionId="project-session"
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        onStartGlobalChat={onStartGlobalChat}
        privacySettings={null}
        projectId="project-1"
        sessions={[
          { ...sessions[0], id: "project-session", title: "Project work", projectId: "project-1" },
          { ...sessions[0], id: "global-session", title: "Global work", projectId: null },
          { ...sessions[0], id: "other-project-session", title: "Other Project", projectId: "project-2" },
        ]}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("button", { name: "New Project chat" })).toBeVisible();
    const projectSession = document.getElementById("oomu-chat-session-project-session");
    expect(projectSession).toBeVisible();
    expect(await screen.findByText(/Project: OOMU Test Project/)).toBeVisible();
    expect(screen.queryByText("Global work")).not.toBeInTheDocument();
    expect(screen.queryByText("Other Project")).not.toBeInTheDocument();
    const scope = screen.getByRole("region", { name: "Chat scope" });
    expect(scope).toHaveTextContent("New chats stay connected to this Project.");
    fireEvent.click(within(scope).getByRole("button", { name: "Start global chat" }));
    expect(onStartGlobalChat).toHaveBeenCalledTimes(1);
  });

  it("explains how to recover instead of silently running a Project-file request globally", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "list_chat_sessions") return sessions;
      return null;
    });
    const view = render(
      <ChatScreen
        activeSessionId="global-session"
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[{ ...sessions[0], id: "global-session", projectId: null }]}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: {
        value: "Funder_Questions.pdf\nCohort_Outcomes.xlsx\nProgram_Notes.docx\nProduce an editable Word document and a PDF.",
      },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(await screen.findByText(/This chat isn’t connected to a Project/)).toBeVisible();
    expect(document.getElementById("oomu-chat-session-global-session")).toHaveTextContent("Global chat");
    await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "accept_chat_turn")).toBe(true));
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });
});
