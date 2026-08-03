import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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

  it("makes Project chat scope explicit and offers a global-chat escape", () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      return null;
    });
    const onStartGlobalChat = vi.fn();
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        onStartGlobalChat={onStartGlobalChat}
        privacySettings={null}
        projectId="project-1"
        sessions={[{ ...sessions[0], projectId: "project-1" }]}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("button", { name: "New Project chat" })).toBeVisible();
    const scope = screen.getByRole("region", { name: "Chat scope" });
    expect(scope).toHaveTextContent("New chats stay connected to this Project.");
    fireEvent.click(within(scope).getByRole("button", { name: "Start global chat" }));
    expect(onStartGlobalChat).toHaveBeenCalledTimes(1);
  });
});
