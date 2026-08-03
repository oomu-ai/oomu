import { act, cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { StoredChatMessage } from "@/lib/chatSessions";
import { ChatScreen } from "../ChatScreen";
import {
  cloudAgents,
  cloudConfiguredProviders,
  cloudSessions,
} from "./ChatScreen.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const tauriRuntimeMock = vi.hoisted(() => ({ value: true }));

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
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

vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: null,
    fallbackRoute: null,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

afterEach(() => {
  cleanup();
  invokeMock.mockReset();
  tauriRuntimeMock.value = true;
});

describe("sovereign search result and evidence presentation", () => {
  it("renders successful tool-result notices green while actual failures remain red", async () => {
    tauriRuntimeMock.value = false;
    const messages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "system",
        content: "Tool result ready.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-1",
        role: "system",
        content: "Web search isn't available right now. Try again.",
        createdAtMs: 2,
      },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return messages;
      if (command === "get_queued_messages" || command === "list_installed_mods") return [];
      if (command === "get_session_config") return null;
      return null;
    });

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
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    const success = await within(view.container).findByText("Tool result ready.");
    const failure = within(view.container).getByText(
      "Web search isn't available right now. Try again.",
    );
    expect(success.closest("div[class*='max-w-3xl']")).toHaveClass(
      "bg-[var(--success-background)]",
      "text-[var(--success)]",
    );
    expect(failure.closest("div[class*='max-w-3xl']")).toHaveClass(
      "bg-[var(--destructive-background)]",
      "text-[var(--destructive)]",
    );
  });

  it("excludes durable internal envelopes from the DOM and accessibility tree", async () => {
    tauriRuntimeMock.value = false;
    const messages: StoredChatMessage[] = [
      ...["sovereign_search_progress", "verified_sovereign_search"].map(
        (checkpointKind, index) => ({
          id: index + 1,
          sessionId: "session-1",
          role: "system" as const,
          content: checkpointKind,
          metadataJson: JSON.stringify({ checkpointKind, uiOnlyCheckpoint: true }),
          createdAtMs: index + 1,
        }),
      ),
      {
        id: 3,
        sessionId: "session-1",
        role: "assistant",
        content: "Verified answer with citations.",
        createdAtMs: 3,
      },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return messages;
      if (command === "get_queued_messages" || command === "list_installed_mods") return [];
      if (command === "get_session_config") return null;
      return null;
    });

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
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    expect(await within(view.container).findByText("Verified answer with citations."))
      .toBeInTheDocument();
    expect(view.container).not.toHaveTextContent("sovereign_search_progress");
    expect(view.container).not.toHaveTextContent("verified_sovereign_search");
  });
});

describe("sovereign search clarification presentation", () => {
  it("presents a missing public-search topic as one completed assistant clarification", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages" ||
        command === "list_installed_mods") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "finalize_accepted_chat_turn") return 1;
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
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
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Search the web" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    expect(await within(view.container).findByText("What would you like me to search for?"))
      .toBeVisible();
    expect(within(view.container).queryByText("System")).not.toBeInTheDocument();
    await waitFor(() => {
      const finalization = invokeMock.mock.calls.find(
        ([command]) => command === "finalize_accepted_chat_turn",
      )?.[1]?.request;
      expect(finalization).toEqual(expect.objectContaining({
        role: "assistant",
        status: "completed",
        content: "What would you like me to search for?",
      }));
    });
    for (const forbidden of [
      "sovereign_duckduckgo_search",
      "classify_chat_intent_route",
      "process_agent_objective",
      "chat_turn",
    ]) {
      expect(invokeMock.mock.calls.some(([command]) => command === forbidden)).toBe(false);
    }
  });
});

describe("explicit search terminal outcomes", () => {
  it("keeps localized host search progress visible on the primary composer surface", async () => {
    let rejectSearch: ((reason?: unknown) => void) | undefined;
    const pendingSearch = new Promise<never>((_resolve, reject) => {
      rejectSearch = reject;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages" ||
        command === "list_installed_mods") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "sovereign_duckduckgo_search") return pendingSearch;
      if (command === "finalize_accepted_chat_turn") return 1;
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
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
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Go online and research the new Kimi and Fable accusations" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      const status = view.container.querySelector("#oomu-chat-status");
      expect(status).toHaveAttribute("role", "status");
      expect(status).toHaveTextContent("Searching the web...");
    });
    await act(async () => {
      rejectSearch?.({ code: "search_no_results" });
      await pendingSearch.catch(() => undefined);
    });
    expect(await within(view.container).findByText(
      "OOMU couldn't find a reliable result for that search. Try a more specific topic.",
    )).toBeVisible();
  });

  it("presents an explicit search with no evidence as an honest assistant outcome", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages" ||
        command === "list_installed_mods") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "sovereign_duckduckgo_search") {
        throw { code: "search_no_results" };
      }
      if (command === "finalize_accepted_chat_turn") return 1;
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
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
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Go online and research the new Kimi and Fable accusations" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const outcome = "OOMU couldn't find a reliable result for that search. Try a more specific topic.";
    expect(await within(view.container).findByText(outcome)).toBeVisible();
    expect(within(view.container).queryByText("System")).not.toBeInTheDocument();
    await waitFor(() => {
      const finalization = invokeMock.mock.calls.find(
        ([command]) => command === "finalize_accepted_chat_turn",
      )?.[1]?.request;
      expect(finalization).toEqual(expect.objectContaining({
        role: "assistant",
        status: "completed",
        content: outcome,
      }));
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });
});
