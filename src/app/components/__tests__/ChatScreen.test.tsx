import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  BrowserModPanel,
  ChatScreen,
  type ChatAgent,
} from "../ChatScreen";
import { I18nProvider } from "@/context/I18nContext";
import { ApprovalProvider } from "@/context/ApprovalContext";
import type { ChatSession, StoredChatMessage } from "@/lib/chatSessions";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import {
  agents,
  approvedFilePreparation,
  cloudAgents,
  cloudConfiguredProviders,
  cloudSessions,
  configuredProviders,
  geminiConfiguredProviders,
  rejectDeferred,
  resolveDeferred,
  searchEnabledSessions,
  sessions,
  storedMessages,
  testBypassNotice,
  terminal,
  token,
} from "./ChatScreen.fixtures";
import { createPlanPersistenceMock } from "./ChatScreen.plan-test-runtime";
import { MAIL_READ_FAILURE_RESULT, TERMINAL_DOWNLOADS_LIST_PROMPT } from "./ChatScreen.native-tool-fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const tauriRuntimeMock = vi.hoisted(() => ({ value: false }));
const modelRoutingPreferencesMock = vi.hoisted(() => ({
  primaryRoute: null as null | {
    providerConfigId: string;
    providerId: string;
    modelId: string;
    label: string;
    updatedAt: number;
  },
  fallbackRoute: null as null | {
    providerConfigId: string;
    providerId: string;
    modelId: string;
    label: string;
    updatedAt: number;
  },
}));
const tauriEventListeners = vi.hoisted(
  () => new Map<string, Set<(event: { payload: unknown }) => void>>(),
);
const executionChannelCallbacks = vi.hoisted(
  () => new Set<(batch: Record<string, unknown>) => void>(),
);

function ApprovalTestProvider({ children }: { children: ReactNode }) {
  return (
    <I18nProvider>
      <ApprovalProvider>{children}</ApprovalProvider>
    </I18nProvider>
  );
}
vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) {
      return true;
    }
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1e6, accepted: true };
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
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    const listeners = tauriEventListeners.get(event) ?? new Set();
    listeners.add(handler);
    tauriEventListeners.set(event, listeners);
    return () => {
      listeners.delete(handler);
      if (listeners.size === 0 && tauriEventListeners.get(event) === listeners) {
        tauriEventListeners.delete(event);
      }
    };
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class TestChannel {
    constructor(callback: (batch: Record<string, unknown>) => void) {
      executionChannelCallbacks.add(callback);
    }
  },
}));

function emitTauriEvent(event: string, payload: unknown) {
  if (event === "chat://token") {
    payload = { delivery_state: "validated", ...(payload as object) };
  }
  for (const listener of tauriEventListeners.get(event) ?? []) {
    listener({ payload });
  }
}

vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: modelRoutingPreferencesMock.primaryRoute,
    fallbackRoute: modelRoutingPreferencesMock.fallbackRoute,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

describe("ChatScreen", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    tauriRuntimeMock.value = false;
    modelRoutingPreferencesMock.primaryRoute = null;
    modelRoutingPreferencesMock.fallbackRoute = null;
    tauriEventListeners.clear();
    executionChannelCallbacks.clear();
    delete (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__;
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
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

  it("keeps Tasks and Documents available as quiet Chat utilities", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_system_hardware_profile") {
        return {
          physicalMemoryGb: 16,
          processorTier: "Mid",
          cpuArch: "aarch64",
          cpuCores: 8,
          osName: "macos",
          metalSupported: true,
          maxLocalContextBudget: 16_384,
        };
      }
      return null;
    });
    const onOpenTasks = vi.fn();
    const onOpenDocuments = vi.fn();
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={cloudAgents}
        configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onOpenDocuments={onOpenDocuments}
        onOpenTasks={onOpenTasks}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    const results = screen.getByRole("navigation", { name: "Results" });
    fireEvent.click(within(results).getByRole("button", { name: "All tasks" }));
    fireEvent.click(within(results).getByRole("button", { name: "Documents" }));
    expect(onOpenTasks).toHaveBeenCalledTimes(1);
    expect(onOpenDocuments).toHaveBeenCalledTimes(1);
  });

  it("keeps the loading state active until the native auto-turn finishes", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (
        command === "list_chat_messages" ||
        command === "get_queued_messages" ||
        command === "list_installed_mods"
      ) {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_system_hardware_profile") {
        return {
          physicalMemoryGb: 16,
          processorTier: "Mid",
          cpuArch: "aarch64",
          cpuCores: 8,
          osName: "macos",
          metalSupported: true,
          maxLocalContextBudget: 16_384,
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
      return null;
    });
    render(
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
    await waitFor(() => {
      expect(tauriEventListeners.has("gateway://auto-turn")).toBe(true);
      expect(
        invokeMock.mock.calls.some(([command]) => command === "list_chat_messages"),
      ).toBe(true);
    });
    fireEvent.click(screen.getByRole("button", { name: "Tuning" }));

    act(() => {
      emitTauriEvent("gateway://auto-turn", {
        sessionId: "session-1",
        taskId: "data-verification",
        turnId: "turn-guarded",
        status: "data_retrying",
      });
    });
    expect(
      await screen.findByText(
        "We encountered an issue verifying the live data. Retrying...",
      ),
    ).toBeInTheDocument();

    act(() => {
      emitTauriEvent("gateway://auto-turn", {
        sessionId: "session-1",
        taskId: "task-1",
        status: "retrieving",
      });
    });
    expect(await screen.findByText("Working in the background…")).toBeInTheDocument();

    act(() => {
      emitTauriEvent("gateway://auto-turn", {
        sessionId: "session-1",
        taskId: "task-1",
        status: "processing",
      });
    });
    expect(await screen.findByText("Compiling the completed work…")).toBeInTheDocument();

    act(() => {
      emitTauriEvent("gateway://auto-turn", {
        sessionId: "session-1",
        taskId: "task-1",
        status: "completed",
        turnId: "turn-auto",
      });
    });
    expect(await screen.findByText("Ready.")).toBeInTheDocument();
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "list_chat_sessions"),
      ).toBe(true);
    });
  });

  it("renders routing and assistant execution badges from stored metadata", async () => {
    const assistantMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "assistant",
        content: "Escalated answer.",
        providerId: "openai",
        modelId: "gpt-5.5",
        metadataJson: JSON.stringify({
          routingMode: "dynamic",
          eventKind: "dynamic_routing",
          executingProviderId: "openai",
          executingModelId: "gpt-5.5",
          secureMemoryStatus: "claim_rejected",
          matchedComplexityRules: ["phrase:architecture review"],
        }),
        createdAtMs: 1,
      },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return assistantMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
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

    await waitFor(() => {
      expect(within(view.container).getByText("Cloud (GPT 5.5)")).toBeInTheDocument();
      expect(within(view.container).getByTitle("Processed by GPT 5.5")).toBeInTheDocument();
      expect(within(view.container).getByText("That memory was not saved.")).toBeInTheDocument();
      expect(
        within(view.container).getByText(/reply above did not change your saved memory/i),
      ).toBeInTheDocument();
    });
  });

  it("toggles active split panel content from the header", async () => {
    const verticalMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "assistant",
        content: [
          "### CLIENT PROFILE STATE",
          "* State: Confused",
          "",
          "### RECOMMENDED RESOLUTION PATHS",
          "1. Verify the account.",
          "",
          "### EXPERIENCE ENHANCEMENT CHECKS",
          "* Avoid unsupported promises.",
        ].join("\n"),
        createdAtMs: 1,
      },
    ];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return verticalMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    render(
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

    await screen.findByLabelText("Operation control panel");

    const splitToggle = screen.getByRole("button", { name: "Split" });
    expect(splitToggle).toBeEnabled();
    expect(splitToggle).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(splitToggle);

    expect(screen.queryByLabelText("Operation control panel")).not.toBeInTheDocument();
    expect(splitToggle).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(splitToggle);

    expect(screen.getByLabelText("Operation control panel")).toBeInTheDocument();
    expect(splitToggle).toHaveAttribute("aria-pressed", "true");
  });

  it("keeps a closed browser split panel closed after leaving and returning to Chat", async () => {
    const browserMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "user",
        content: "Open google.com in the browser.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-1",
        role: "assistant",
        content: [
          "<OomuSplitView>",
          "<mod_id>ai.eldris.mods.browser</mod_id>",
          "<action>NAVIGATE</action>",
          "<url>https://www.google.com</url>",
          "<reason>Open the requested site.</reason>",
          "</OomuSplitView>",
        ].join(" "),
        createdAtMs: 2,
      },
    ];
    const storedValues = new Map<string, string>();
    vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
      storedValues.get(key) ?? null,
    );
    vi.mocked(window.localStorage.setItem).mockImplementation((key: string, value: string) => {
      storedValues.set(key, value);
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return browserMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    function ChatNavigationHarness() {
      const [showChat, setShowChat] = useState(true);
      return (
        <>
          <button onClick={() => setShowChat(false)} type="button">Leave Chat</button>
          <button onClick={() => setShowChat(true)} type="button">Return to Chat</button>
          {showChat ? (
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
            />
          ) : (
            <div>Agents view</div>
          )}
        </>
      );
    }

    render(<ChatNavigationHarness />, { wrapper: I18nProvider });

    await screen.findByLabelText("Browser mod");
    fireEvent.click(screen.getByRole("button", { name: "Split" }));
    expect(screen.queryByLabelText("Browser mod")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(
        JSON.parse(storedValues.get("oomu.chat.dismissedSplitRoutes") ?? "[]"),
      ).toHaveLength(1);
    });

    fireEvent.click(screen.getByRole("button", { name: "Leave Chat" }));
    expect(screen.getByText("Agents view")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Return to Chat" }));

    await waitFor(() => {
      const splitToggle = screen.getByRole("button", { name: "Split" });
      expect(splitToggle).toBeEnabled();
      expect(splitToggle).toHaveAttribute("aria-pressed", "false");
    });
    expect(screen.queryByLabelText("Browser mod")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Split" }));
    await screen.findByLabelText("Browser mod");
    await waitFor(() => {
      expect(
        JSON.parse(storedValues.get("oomu.chat.dismissedSplitRoutes") ?? "[]"),
      ).toEqual([]);
    });
  });

  it("reopens a persisted-closed split panel for a new browser route", async () => {
    const dismissedRouteIdentity = JSON.stringify([
      "session-1",
      "ai.eldris.mods.browser",
      2,
    ]);
    const storedValues = new Map<string, string>([
      ["oomu.chat.dismissedSplitRoutes", JSON.stringify([dismissedRouteIdentity])],
    ]);
    const dismissedBrowserMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "user",
        content: "Open google.com in the browser.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-1",
        role: "assistant",
        content:
          "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://www.google.com</url></OomuSplitView>",
        createdAtMs: 2,
      },
    ];
    vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
      storedValues.get(key) ?? null,
    );
    vi.mocked(window.localStorage.setItem).mockImplementation((key: string, value: string) => {
      storedValues.set(key, value);
    });
    invokeMock.mockImplementation(async (
      command: string,
      args?: { request?: Record<string, unknown> },
    ) => {
      if (command === "list_chat_messages") {
        return dismissedBrowserMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "accept_chat_turn") {
        return {
          turnId: args?.request?.turn_id,
          messageId: 3,
          accepted: true,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Opening the page.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    await waitFor(() => {
      expect(window.localStorage.setItem).toHaveBeenCalledWith(
        "oomu.chat.dismissedSplitRoutes",
        JSON.stringify([dismissedRouteIdentity]),
      );
    });
    await waitFor(() => {
      const splitToggle = within(view.container).getByRole("button", { name: "Split" });
      expect(splitToggle).toBeEnabled();
      expect(splitToggle).toHaveAttribute("aria-pressed", "false");
    });
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Visit example.com in the browser" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const browserPanel = await within(view.container).findByLabelText("Browser mod");
    expect(within(browserPanel).getByText("example.com")).toBeInTheDocument();
    expect(within(view.container).getByRole("button", { name: "Split" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await waitFor(() => {
      expect(
        JSON.parse(storedValues.get("oomu.chat.dismissedSplitRoutes") ?? "[]"),
      ).toEqual([dismissedRouteIdentity]);
    });
  });

  it("scopes browser dismissals by session and provider without persisting URL secrets", async () => {
    const rawUrl =
      "https://user-canary:password-canary@example.com/path?token=query-secret-canary";
    let hydratedMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "user",
        content: "Open https://example.com in the browser.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-1",
        role: "assistant",
        content: `<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>${rawUrl}</url></OomuSplitView>`,
        createdAtMs: 2,
      },
    ];
    const storedValues = new Map<string, string>();
    vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
      storedValues.get(key) ?? null,
    );
    vi.mocked(window.localStorage.setItem).mockImplementation((key: string, value: string) => {
      storedValues.set(key, value);
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return hydratedMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    const firstView = render(
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
    await within(firstView.container).findByLabelText("Browser mod");
    fireEvent.click(within(firstView.container).getByRole("button", { name: "Split" }));
    await waitFor(() => {
      const storedDismissals = storedValues.get("oomu.chat.dismissedSplitRoutes") ?? "";
      expect(JSON.parse(storedDismissals)).toHaveLength(1);
      for (const canary of ["user-canary", "password-canary", "query-secret-canary"]) {
        expect(storedDismissals).not.toContain(canary);
      }
    });
    firstView.unmount();

    hydratedMessages = [
      {
        id: 1,
        sessionId: "session-2",
        role: "user",
        content: "Open https://example.com in the browser.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-2",
        role: "assistant",
        content: `<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>${rawUrl}</url></OomuSplitView>`,
        createdAtMs: 2,
      },
    ];
    const sessionTwo = { ...sessions[0], id: "session-2" };
    const secondView = render(
      <ChatScreen
        activeSessionId="session-2"
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[sessionTwo]}
      />,
      { wrapper: I18nProvider },
    );
    await within(secondView.container).findByLabelText("Browser mod");
    await waitFor(() => {
      expect(within(secondView.container).getByRole("button", { name: "Split" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });
    secondView.unmount();

    hydratedMessages = [
      {
        id: 1,
        sessionId: "session-1",
        role: "assistant",
        content: [
          "### CLIENT PROFILE STATE",
          "* State: Confused",
          "",
          "### RECOMMENDED RESOLUTION PATHS",
          "1. Verify the account.",
          "",
          "### EXPERIENCE ENHANCEMENT CHECKS",
          "* Avoid unsupported promises.",
        ].join("\n"),
        createdAtMs: 1,
      },
    ];
    const thirdView = render(
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
    await within(thirdView.container).findByLabelText("Operation control panel");
    expect(within(thirdView.container).getByRole("button", { name: "Split" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("keeps a dismissed browser route closed across duplicate stream activation", async () => {
    const enabledSessions = searchEnabledSessions;
    const directive =
      "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://www.google.com/search?q=oomu</url><reason>Searching Google for oomu.</reason></OomuSplitView>";
    let streamRequest: Record<string, string> | null = null;
    let resolveTurn: ((value: Record<string, unknown>) => void) | null = null;
    invokeMock.mockImplementation((command: string, payload?: Record<string, unknown>) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        streamRequest = (payload as { request: Record<string, string> }).request;
        emitTauriEvent("chat://token", {
          stream_id: streamRequest.stream_id,
          session_id: "session-1",
          turn_id: streamRequest.turn_id,
          generation_token: streamRequest.generation_token,
          sequence: 1,
          token: directive,
          elapsed_ms: 1,
        });
        return new Promise<Record<string, unknown>>((resolve) => {
          resolveTurn = resolve;
        });
      }
      if (command === "list_chat_sessions") {
        return enabledSessions;
      }
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
        sessions={enabledSessions}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use the browser to research oomu" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const streamedBrowserPanel = await within(view.container).findByLabelText("Browser mod");
    await within(streamedBrowserPanel).findByText("Suggested by OOMU");
    expect(within(streamedBrowserPanel).queryByText("Searching Google for oomu.")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(within(view.container).getByRole("button", { name: "Split" })).toHaveAttribute("aria-pressed", "true");
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Split" }));
    await waitFor(() => {
      expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
    });

    const request = streamRequest as Record<string, string> | null;
    expect(request).not.toBeNull();
    emitTauriEvent("chat://token", {
      stream_id: request?.stream_id,
      session_id: "session-1",
      turn_id: request?.turn_id,
      generation_token: request?.generation_token,
      sequence: 2,
      token: directive,
      elapsed_ms: 2,
    });
    resolveDeferred(resolveTurn, { text: directive, session_id: "session-1" });

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "list_chat_sessions")).toBe(true);
      expect(within(view.container).getByRole("button", { name: "Split" })).toHaveAttribute(
        "aria-pressed",
        "false",
      );
    });
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
  });

  it("keeps assistant browser directives renderer-only until the native runtime is available", async () => {
    const browserMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "user",
        content: "Open google.com in the browser.",
        createdAtMs: 1,
      },
      {
        id: 2,
        sessionId: "session-1",
        role: "assistant",
        content:
          "<OomuSplitView> <mod_id>ai.eldris.mods.browser</mod_id> <action>NAVIGATE</action> <url>https://www.google.com</url> <reason>User requested to open google.com using browser capabilities.</reason> </OomuSplitView>",
        createdAtMs: 2,
      },
    ];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return browserMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
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

    const browserPanel = await within(view.container).findByLabelText("Browser mod");
    expect(within(browserPanel).getByText("www.google.com")).toBeInTheDocument();
    expect(within(browserPanel).queryByText("https://www.google.com/")).not.toBeInTheDocument();
    expect(within(browserPanel).getByText("Suggested by OOMU")).toBeInTheDocument();
    expect(within(browserPanel).queryByText("User requested to open google.com using browser capabilities.")).not.toBeInTheDocument();
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    fireEvent.click(within(browserPanel).getByRole("button", { name: "Open secure browser" }));
    await within(browserPanel).findByText("Couldn't open the page");
    expect(within(browserPanel).getByText("Open OOMU on your Mac to use this page.")).toBeInTheDocument();
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain(
      "authorize_native_browser_navigation",
    );
    expect(within(view.container).queryByText(/OomuSplitView/)).not.toBeInTheDocument();

    const splitToggle = within(view.container).getByRole("button", { name: "Split" });
    expect(splitToggle).toBeEnabled();
    expect(splitToggle).toHaveAttribute("aria-pressed", "true");
  });

  it("fails a fresh browser request closed outside the native runtime", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Opening the page.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      target: { value: "Visit example.com in the browser" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const browserPanel = await within(view.container).findByLabelText("Browser mod");
    expect(within(browserPanel).getByText("example.com")).toBeInTheDocument();
    expect(within(browserPanel).queryByText("https://example.com/")).not.toBeInTheDocument();
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    fireEvent.click(within(browserPanel).getByRole("button", { name: "Open secure browser" }));
    await within(browserPanel).findByText("Couldn't open the page");
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    expect(within(view.container).getByRole("button", { name: "Split" })).toHaveAttribute("aria-pressed", "true");
  });

  it("never falls back to an iframe when native browser policy blocks a model directive", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [
          {
            id: 1,
            sessionId: "session-1",
            role: "user",
            content: "Open https://127.0.0.1 in the browser.",
            createdAtMs: 1,
          },
          {
            id: 2,
            sessionId: "session-1",
            role: "assistant",
            content:
              "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://127.0.0.1/</url></OomuSplitView>",
            createdAtMs: 2,
          },
        ];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "authorize_native_browser_navigation") {
        throw new Error("BACKEND CANARY: native_browser_loopback_address_class");
      }
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

    const browserPanel = await within(view.container).findByLabelText("Browser mod");
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    fireEvent.click(within(browserPanel).getByRole("button", { name: "Open secure browser" }));
    await within(browserPanel).findByText("Couldn't open the page");
    expect(within(browserPanel).getByText(
      "OOMU couldn't open the secure browser. Try again.",
    )).toBeInTheDocument();
    expect(within(browserPanel).queryByText(/BACKEND CANARY|native_browser_loopback/i)).not.toBeInTheDocument();
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain(
      "open_authorized_native_browser",
    );
  });

  it("redacts model URL credentials in browser consent while authorizing the exact original URL", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const rawUrl =
      "https://user-canary:password-canary@example.com/bot123456:telegram-secret-canary/path?token=query-secret-canary";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") {
        throw new Error("Native destination policy rejected this URL.");
      }
      return null;
    });
    const route = {
      messageId: 1,
      sessionId: "session-1",
      modId: "ai.eldris.mods.browser",
      action: "NAVIGATE",
      url: rawUrl,
      reason: null,
      rawDirective: "",
    };
    const view = render(<BrowserModPanel route={route} />, {
      wrapper: I18nProvider,
    });

    for (const canary of [
      "user-canary",
      "password-canary",
      "telegram-secret-canary",
      "query-secret-canary",
    ]) {
      expect(view.container.textContent).not.toContain(canary);
    }
    expect(view.container.textContent).toContain("example.com");
    fireEvent.click(
      within(view.container).getByRole("button", { name: "Open secure browser" }),
    );
    await within(view.container).findByText("Couldn't open the page");
    expect(invokeMock).toHaveBeenCalledWith("authorize_native_browser_navigation", {
      url: rawUrl,
    });
    expect(within(view.container).queryByTitle("Preview")).not.toBeInTheDocument();
  });

  it("shows only the site name in browser consent while keeping the exact URL internal", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const rawUrl = "https://www.google.com/search?q=private+calendar+request&account=personal";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") {
        throw new Error("Stopped for this test.");
      }
      return null;
    });
    const view = render(<BrowserModPanel route={{
      messageId: 3,
      sessionId: "session-1",
      modId: "ai.eldris.mods.browser",
      action: "NAVIGATE",
      url: rawUrl,
      reason: null,
      rawDirective: "",
    }} />, { wrapper: I18nProvider });

    expect(view.container.textContent).toContain("google.com");
    expect(view.container.textContent).not.toContain("private calendar request");
    expect(view.container.textContent).not.toContain("private+calendar+request");
    expect(view.container.textContent).not.toContain("account=personal");

    fireEvent.click(within(view.container).getByRole("button", { name: "Open secure browser" }));
    await within(view.container).findByText("Couldn't open the page");
    expect(invokeMock).toHaveBeenCalledWith("authorize_native_browser_navigation", {
      url: rawUrl,
    });
  });

  it("revokes a pending native browser approval when the active route changes", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    let resolveAuthorization: ((value: {
      approvalToken: string;
      canonicalUrl: string;
      canonicalOrigin: string;
      destinationBinding: string;
      expiresAtMs: number;
    }) => void) | null = null;
    const authorization = new Promise<{
      approvalToken: string;
      canonicalUrl: string;
      canonicalOrigin: string;
      destinationBinding: string;
      expiresAtMs: number;
    }>((resolve) => {
      resolveAuthorization = resolve;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") {
        return authorization;
      }
      return null;
    });

    const firstRoute = {
      messageId: 1,
      sessionId: "session-1",
      modId: "ai.eldris.mods.browser",
      action: "NAVIGATE",
      url: "https://example.com/a",
      reason: null,
      rawDirective: "",
    };
    const secondRoute = {
      ...firstRoute,
      messageId: 2,
      url: "https://example.org/b",
    };
    const view = render(<BrowserModPanel route={firstRoute} />, {
      wrapper: I18nProvider,
    });
    fireEvent.click(
      within(view.container).getByRole("button", { name: "Open secure browser" }),
    );
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("authorize_native_browser_navigation", {
        url: "https://example.com/a",
      });
    });

    view.rerender(<BrowserModPanel route={secondRoute} />);
    expect(within(view.container).getAllByText("example.org").length).toBeGreaterThan(0);
    resolveDeferred(resolveAuthorization, {
      approvalToken: "stale-token",
      canonicalUrl: "https://example.com/a",
      canonicalOrigin: "https://example.com",
      destinationBinding: "stale-binding",
      expiresAtMs: Date.now() + 60_000,
    });
    await authorization;

    await waitFor(() => {
      expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain(
        "open_authorized_native_browser",
      );
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).toContain("close_native_browser");
    expect(
      within(view.container).getByRole("button", { name: "Open secure browser" }),
    ).toBeEnabled();
  });

  it("retargets the live browser panel from streamed split-view directives", async () => {
    const enabledSessions = searchEnabledSessions;
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        const streamId = (payload as { request: { stream_id: string } }).request.stream_id;
        emitTauriEvent("chat://token", {
          stream_id: streamId,
          session_id: "session-1",
          turn_id: (payload as { request: { turn_id: string } }).request.turn_id,
          generation_token: (payload as { request: { generation_token: string } }).request.generation_token,
          sequence: 1,
          token:
            "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://www.google.com/search?q=oomu</url><reason>Searching Google for oomu.</reason></OomuSplitView>",
          elapsed_ms: 1,
        });
        return {
          text:
            "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://www.google.com/search?q=oomu</url><reason>Searching Google for oomu.</reason></OomuSplitView>",
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
        sessions={enabledSessions}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use the browser to research oomu" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const browserPanel = await within(view.container).findByLabelText("Browser mod");
    await waitFor(() => {
      expect(within(browserPanel).getByText("www.google.com")).toBeInTheDocument();
      expect(within(browserPanel).queryByText(/search\?q=oomu/)).not.toBeInTheDocument();
    });
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    fireEvent.click(within(browserPanel).getByRole("button", { name: "Open secure browser" }));
    await within(browserPanel).findByText("Couldn't open the page");
    expect(within(browserPanel).queryByTitle("Preview")).not.toBeInTheDocument();
    expect(within(view.container).queryByText(/OomuSplitView/)).not.toBeInTheDocument();
  });

  it("runs an explicit active network mod headlessly while automatic grounding is off", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const travelDirective =
      "<OomuSplitView><mod_id>ai.eldris.mods.travel_companion</mod_id><action>NAVIGATE</action><url>https://www.google.com/flights?q=ROC+to+SIN</url><reason>Checking live flight options.</reason></OomuSplitView>";
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "list_installed_mods") {
        return [
          {
            id: "ai.eldris.mods.browser",
            name: "Sovereign Web Browser",
            isActive: true,
            endpoints: ["*"],
            commands: [],
          },
          {
            id: "ai.eldris.mods.travel_companion",
            name: "Travel Companion",
            isActive: true,
            endpoints: ["google.com", "*.google.com"],
            commands: [{
              trigger: "/travel",
              description: { "en-US": "Search live flights and accommodations." },
              public_network: true,
            }],
          },
        ];
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "sovereign_duckduckgo_search") {
        return {
          query: "travel ROC to SIN",
          engine: "duckduckgo_lite_static",
          resultCount: 1,
          results: [{
            title: "Flights from Rochester to Singapore",
            url: "https://www.google.com/travel/flights",
            snippet: "Live public flight options.",
          }],
          contextJson: JSON.stringify({
            results: [{
              title: "Flights from Rochester to Singapore",
              url: "https://www.google.com/travel/flights",
              snippet: "Live public flight options.",
            }],
            pages: [{
              url: "https://www.google.com/travel/flights",
              title: "Google Flights",
              visibleText: "ROC to SIN from $1,120 round trip",
              inputs: [],
              buttons: [],
              links: [],
              tables: [],
              extractionMethod: "headless_browser",
            }],
          }),
          retrievalElapsedMs: 35,
          domPageCount: 1,
          headlessFallbackCount: 1,
          degraded: false,
          security: {
            apiKeyRequired: false,
            cookiesEnabled: false,
            browserAutomationEnabled: true,
            visibleBrowserOpened: false,
            proxyEnvironmentEnabled: false,
            endpointAllowlist: ["lite.duckduckgo.com"],
          },
        };
      }
      if (command === "chat_turn") {
        const request = (payload as { request: Record<string, string> }).request;
        emitTauriEvent("chat://token", {
          stream_id: request.stream_id,
          session_id: request.session_id,
          turn_id: request.turn_id,
          generation_token: request.generation_token,
          sequence: 1,
          token: travelDirective,
          elapsed_ms: 1,
        });
        return {
          text: travelDirective,
          session_id: request.session_id,
          turn_id: request.turn_id,
          generation_token: request.generation_token,
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
          automatedWebGroundingEnabled: false,
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
      expect(invokeMock.mock.calls.some(([command]) => command === "list_installed_mods")).toBe(true);
    });
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "/travel ROC to SIN" },
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
        query: "travel ROC to SIN",
        originatingUtterance: "/travel ROC to SIN",
        maxResults: 5,
        sessionId: "session-1",
        originTurnId: expect.any(String),
        originGenerationToken: expect.any(String),
        modId: "ai.eldris.mods.travel_companion",
      },
    });
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
    expect(within(view.container).queryByText(/OomuSplitView/)).not.toBeInTheDocument();
  });

  it("shows a headless mod turn immediately and enriches the same bubble after search completes", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const prompt = "/travel ROC to SIN";
    let resolveSearch: ((value: Record<string, unknown>) => void) | null = null;
    const pendingSearch = new Promise<Record<string, unknown>>((resolve) => {
      resolveSearch = resolve;
    });
    let resolveChatTurn: ((value: Record<string, unknown>) => void) | null = null;
    const pendingChatTurn = new Promise<Record<string, unknown>>((resolve) => {
      resolveChatTurn = resolve;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "list_installed_mods") {
        return [
          {
            id: "ai.eldris.mods.travel_companion",
            name: "Travel Companion",
            isActive: true,
            endpoints: ["kayak.com", "*.kayak.com"],
            commands: [{
              trigger: "/travel",
              description: { "en-US": "Search live flights and accommodations." },
              public_network: true,
            }],
          },
        ];
      }
      if (command === "sovereign_duckduckgo_search") {
        return pendingSearch;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_web_grounding_filter",
          confidence: 1,
          reason: "Verified local search context is attached.",
          matched_signals: ["hydrated web grounding"],
          status_label: "Reading sources…",
        };
      }
      if (command === "chat_turn") {
        return pendingChatTurn;
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
          automatedWebGroundingEnabled: false,
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
      expect(invokeMock.mock.calls.some(([command]) => command === "list_installed_mods")).toBe(true);
    });
    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: prompt } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
      ).toBe(true);
      expect(within(view.container).getByText(prompt)).toBeInTheDocument();
      expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    });
    expect(within(view.container).queryByText(/local_web_search\.md/)).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);

    await act(async () => {
      resolveSearch?.({
        query: "travel ROC to SIN",
        engine: "duckduckgo_lite_static",
        resultCount: 1,
        contextJson: JSON.stringify({
          results: [{
            title: "Flights from Rochester to Singapore",
            url: "https://www.kayak.com/flights/ROC-SIN",
            snippet: "Verified public flight options.",
          }],
        }),
        retrievalElapsedMs: 35,
        domPageCount: 1,
        headlessFallbackCount: 0,
        degraded: false,
      });
      await pendingSearch;
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
      expect(within(view.container).getByText(/local_web_search\.md/)).toBeInTheDocument();
    });
    const userAuthors = within(view.container).getAllByText("You");
    expect(userAuthors).toHaveLength(1);
    expect(userAuthors[0].parentElement).toHaveTextContent(prompt);
    expect(userAuthors[0].parentElement).toHaveTextContent("local_web_search.md");
    const chatTurnCalls = invokeMock.mock.calls.filter(([command]) => command === "chat_turn");
    expect(chatTurnCalls).toHaveLength(1);
    const chatTurnCall = chatTurnCalls[0];
    const chatTurnRequest = (chatTurnCall?.[1] as {
      request: {
        attachments: Array<{ name: string }>;
        display_message: string;
      };
    }).request;
    expect(chatTurnRequest.attachments.map((attachment) => attachment.name)).toContain(
      "local_web_search.md",
    );
    expect(chatTurnRequest.display_message).toContain("local_web_search.md");

    await act(async () => {
      resolveChatTurn?.({ text: "Verified flight context received." });
      await pendingChatTurn;
    });
  });

  it("binds first-turn headless grounding to the newly created session", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const createdSession = { ...sessions[0], id: "session-new" };
    const onCreateSession = vi.fn(async () => createdSession);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "ready";
      }
      if (command === "list_installed_mods") {
        return [{
          id: "ai.eldris.mods.travel_companion",
          name: "Travel Companion",
          isActive: true,
          endpoints: ["kayak.com", "*.kayak.com"],
          commands: [{
            trigger: "/travel",
            description: "Search live flights and accommodations.",
            public_network: true,
          }],
        }];
      }
      if (command === "sovereign_duckduckgo_search") {
        return {
          query: "travel ROC to SIN",
          engine: "duckduckgo_lite_static",
          resultCount: 1,
          contextJson: JSON.stringify({
            results: [{
              title: "Flights from Rochester to Singapore",
              url: "https://www.kayak.com/flights/ROC-SIN",
              snippet: "Verified public flight options.",
            }],
          }),
          retrievalElapsedMs: 10,
          domPageCount: 1,
          headlessFallbackCount: 0,
          degraded: false,
        };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_web_grounding_filter",
          confidence: 1,
          reason: "Verified local search context is attached.",
          matched_signals: ["hydrated web grounding"],
          status_label: "Reading sources…",
        };
      }
      if (command === "chat_turn") {
        return { text: "Verified context received.", session_id: "session-new" };
      }
      if (command === "list_chat_sessions") {
        return [createdSession];
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId=""
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={onCreateSession}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={{
          automatedWebGroundingEnabled: false,
          licenseAccepted: true,
          licenseState: "accepted",
          licenseVersion: "test",
          licenseEffectiveDate: "2026-01-01",
          licenseText: "Test license",
        }}
        sessions={[]}
      />,
      { wrapper: I18nProvider },
    );

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "list_installed_mods")).toBe(true);
    });
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "/travel ROC to SIN" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(onCreateSession).toHaveBeenCalledTimes(1);
      expect(
        invokeMock.mock.calls.some(([command]) => command === "sovereign_duckduckgo_search"),
      ).toBe(true);
    });
    const searchCall = invokeMock.mock.calls.find(
      ([command]) => command === "sovereign_duckduckgo_search",
    );
    expect(searchCall?.[1]).toEqual({
      request: expect.objectContaining({
        originatingUtterance: "/travel ROC to SIN",
        sessionId: "session-new",
        modId: "ai.eldris.mods.travel_companion",
      }),
    });
  });

  it("keeps concurrent session streams isolated when tokens and completions arrive out of order", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    const sessionList: ChatSession[] = [
      cloudSessions[0],
      { ...cloudSessions[0], id: "session-2", title: "Second chat", updatedAtMs: 2 },
    ];
    const pendingTurns = new Map<
      string,
      {
        request: Record<string, string>;
        resolve: (response: Record<string, unknown>) => void;
      }
    >();
    const completed = new Map<string, string>();
    const onSelectSession = vi.fn();

    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      const request = (args?.request as Record<string, string> | undefined) ?? {};
      const sessionId = request.session_id ?? String(args?.session_id ?? args?.sessionId ?? "");
      if (command === "list_chat_messages") {
        const seed: StoredChatMessage = {
          id: sessionId === "session-1" ? 50 : 1,
          sessionId,
          role: "assistant",
          content: sessionId === "session-1" ? "A history" : "B history",
          createdAtMs: 1,
        };
        const finalText = completed.get(sessionId);
        return finalText
          ? [
              seed,
              {
                id: seed.id + 1,
                sessionId,
                role: "assistant",
                content: finalText,
                createdAtMs: 2,
              },
            ]
          : [seed];
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
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return new Promise((resolve) => {
          pendingTurns.set(sessionId, {
            request,
            resolve,
          });
        });
      }
      if (command === "list_chat_sessions") return sessionList;
      return null;
    });

    const screenProps = {
      agents: cloudAgents,
      configuredProviders: cloudConfiguredProviders,
      onCreateSession: vi.fn(),
      onDeleteSession: vi.fn(),
      onSelectSession,
      onSessionsChange: vi.fn(),
      privacySettings: null,
      sessions: sessionList,
    };
    const view = render(
      <ChatScreen activeSessionId="session-1" {...screenProps} />,
      { wrapper: I18nProvider },
    );

    await screen.findByText("A history");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "request A" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(pendingTurns.has("session-1")).toBe(true));

    view.rerender(<ChatScreen activeSessionId="session-2" {...screenProps} />);
    await screen.findByText("B history");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "request B" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(pendingTurns.has("session-2")).toBe(true));
    const turnA = pendingTurns.get("session-1")!;
    const turnB = pendingTurns.get("session-2")!;
    emitTauriEvent("chat://token", token(turnB.request, "session-2", 1, "final-"));
    emitTauriEvent("chat://token", token(turnA.request, "session-1", 1, "final-"));
    emitTauriEvent("chat://token", token(turnB.request, "session-2", 2, "B"));
    emitTauriEvent("chat://token", token(turnA.request, "session-1", 2, "A"));
    await screen.findByText("final-B");
    expect(screen.queryByText("final-A")).not.toBeInTheDocument();
    completed.set("session-2", "final-B");
    emitTauriEvent("chat://validated-stream-complete", await terminal(turnB.request, "session-2", "final-B", 2));
    turnB.resolve({ text: "final-B", session_id: "session-2" });
    await screen.findByText("final-B");

    completed.set("session-1", "final-A");
    emitTauriEvent("chat://validated-stream-complete", await terminal(turnA.request, "session-1", "final-A", 2));
    turnA.resolve({ text: "final-A", session_id: "session-1" });
    await waitFor(() => expect(tauriEventListeners.has("chat://token")).toBe(false));

    view.rerender(<ChatScreen activeSessionId="session-1" {...screenProps} />);
    await screen.findByText("final-A");
    expect(screen.queryByText("final-B")).not.toBeInTheDocument();

    emitTauriEvent("chat://token", token(turnA.request, "session-1", 3, "stale-A"));
    expect(screen.queryByText("stale-A")).not.toBeInTheDocument();
    view.rerender(<ChatScreen activeSessionId="session-2" {...screenProps} />);
    await screen.findByText("final-B");
    expect(screen.queryByText("final-A")).not.toBeInTheDocument();
    expect(onSelectSession).not.toHaveBeenCalled();
  });

  it("keeps unsent composer drafts isolated while switching sessions", async () => {
    const sessionList: ChatSession[] = [
      cloudSessions[0],
      { ...cloudSessions[0], id: "session-2", title: "Second chat", updatedAtMs: 2 },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "list_chat_sessions") return sessionList;
      return null;
    });
    const screenProps = {
      agents: cloudAgents,
      configuredProviders: cloudConfiguredProviders,
      onCreateSession: vi.fn(),
      onDeleteSession: vi.fn(),
      onSelectSession: vi.fn(),
      onSessionsChange: vi.fn(),
      privacySettings: null,
      sessions: sessionList,
    };
    const view = render(<ChatScreen activeSessionId="session-1" {...screenProps} />, {
      wrapper: I18nProvider,
    });
    const composer = await within(view.container).findByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "draft for A" } });
    expect(composer).toHaveValue("draft for A");

    view.rerender(<ChatScreen activeSessionId="session-2" {...screenProps} />);
    expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "draft for B" },
    });

    view.rerender(<ChatScreen activeSessionId="session-1" {...screenProps} />);
    expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("draft for A");
    view.rerender(<ChatScreen activeSessionId="session-2" {...screenProps} />);
    expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("draft for B");
  });

  it("uses a nonempty native grant scope during the startup session transition", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "list_chat_sessions") return sessions;
      if (command === "revoke_local_context_grants") return { revokedCount: 0 };
      return null;
    });
    const screenProps = {
      agents,
      configuredProviders,
      onCreateSession: vi.fn(),
      onDeleteSession: vi.fn(),
      onSelectSession: vi.fn(),
      onSessionsChange: vi.fn(),
      privacySettings: null,
    };
    const view = render(
      <ChatScreen activeSessionId="" sessions={[]} {...screenProps} />,
      { wrapper: I18nProvider },
    );

    view.rerender(
      <ChatScreen activeSessionId="session-1" sessions={sessions} {...screenProps} />,
    );
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "list_chat_messages")).toBe(true);
    });
    expect(invokeMock).toHaveBeenCalledWith("revoke_local_context_grants", {
      request: { sessionId: "__new_chat_session__" },
    });

    view.unmount();
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("revoke_local_context_grants", {
        request: { sessionId: "session-1" },
      });
    });
  });

  it("uses the new-chat scope for local context before a persisted session exists", async () => {
    tauriRuntimeMock.value = true;
    let pickerRequest: Record<string, unknown> | null = null;
    let readRequest: Record<string, unknown> | null = null;
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "choose_local_context") {
        pickerRequest = args?.request as Record<string, unknown>;
        return {
          results: [{
            name: "startup.txt",
            ok: true,
            grantId: "b".repeat(64),
            mimeType: "text/plain",
            decodedByteCount: 7,
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
        readRequest = args?.request as Record<string, unknown>;
        return {
          name: "startup.txt",
          mime_type: "text/plain",
          byte_count: 7,
          text: "startup",
          truncated: false,
        };
      }
      if (command === "revoke_local_context_grants") return { revokedCount: 0 };
      return null;
    });
    const view = render(
      <ChatScreen
        activeSessionId=""
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[]}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Attach file" }));
    await within(view.container).findByText("startup.txt");

    expect(pickerRequest).toMatchObject({
      sessionId: "__new_chat_session__",
      operation: "read",
      turnId: expect.stringMatching(/^attachment-/),
    });
    const pickerTurnId = (pickerRequest as Record<string, unknown> | null)?.turnId;
    expect(readRequest).toMatchObject({
      grantId: "b".repeat(64),
      sessionId: "__new_chat_session__",
      turnId: pickerTurnId,
    });
    expect(invokeMock).toHaveBeenCalledWith("revoke_local_context_grants", {
      request: {
        sessionId: "__new_chat_session__",
        turnId: pickerTurnId,
      },
    });
  });

  it("claims a native Finder drop privately", async () => {
    tauriRuntimeMock.value = true;
    let claimRequest: Record<string, unknown> | null = null;
    let readRequest: Record<string, unknown> | null = null;
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "claim_dropped_local_context") {
        claimRequest = args?.request as Record<string, unknown>;
        return {
          results: [{
            name: "finder-note.txt",
            ok: true,
            grantId: "c".repeat(64),
            mimeType: "text/plain",
            decodedByteCount: 11,
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
        readRequest = args?.request as Record<string, unknown>;
        return {
          name: "finder-note.txt",
          mime_type: "text/plain",
          byte_count: 11,
          text: "Finder note",
          truncated: false,
        };
      }
      if (command === "revoke_local_context_grants") return { revokedCount: 0 };
      return null;
    });
    const view = render(
      <ChatScreen
        activeSessionId=""
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[]}
      />,
      { wrapper: I18nProvider },
    );
    const dropTarget = view.container.querySelector<HTMLElement>("[data-chat-drop-target]");
    expect(dropTarget).not.toBeNull();
    vi.spyOn(dropTarget!, "getBoundingClientRect").mockReturnValue(
      { bottom: 300, left: 100, right: 700, top: 100 } as DOMRect,
    );
    await waitFor(() => expect(
      tauriEventListeners.get("oomu://local-context-drag")?.size,
    ).toBe(1));

    act(() => emitTauriEvent("oomu://local-context-drag", {
      type: "drop", dropId: "d".repeat(64),
      position: { x: 240, y: 180 },
    }));
    await within(view.container).findByText("finder-note.txt");

    expect(claimRequest).toMatchObject({
      dropId: "d".repeat(64), sessionId: "__new_chat_session__",
      turnId: expect.stringMatching(/^attachment-/),
    });
    const dropTurnId = (claimRequest as Record<string, unknown> | null)?.turnId;
    expect(readRequest).toMatchObject({
      grantId: "c".repeat(64),
      sessionId: "__new_chat_session__",
      turnId: dropTurnId,
    });
    expect(invokeMock).toHaveBeenCalledWith("revoke_local_context_grants", {
      request: {
        sessionId: "__new_chat_session__",
        turnId: dropTurnId,
      },
    });

  });

  it("ignores late stream chunks and completion after cancelling a turn", async () => {
    let pendingRequest: Record<string, string> | null = null;
    let resolveTurn: ((response: Record<string, unknown>) => void) | null = null;
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, string> }) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        pendingRequest = args?.request ?? {};
        return new Promise((resolve) => {
          resolveTurn = resolve;
        });
      }
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
      target: { value: "cancel this turn" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(pendingRequest).not.toBeNull());
    const request = pendingRequest!;
    emitTauriEvent("chat://token", {
      stream_id: request.stream_id,
      session_id: "session-1",
      turn_id: request.turn_id,
      generation_token: request.generation_token,
      sequence: 1,
      token: "before-cancel",
      elapsed_ms: 1,
    });
    await screen.findByText("before-cancel");
    fireEvent.click(within(view.container).getByRole("button", { name: "Stop" }));
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "cancel_chat_stream")).toBe(true);
    });
    emitTauriEvent("chat://token", {
      stream_id: request.stream_id,
      session_id: "session-1",
      turn_id: request.turn_id,
      generation_token: request.generation_token,
      sequence: 2,
      token: "after-cancel",
      elapsed_ms: 2,
    });
    resolveDeferred(resolveTurn, { text: "late completion", session_id: "session-1" });
    await waitFor(() => {
      expect(within(view.container).getByRole("button", { name: "Send" })).toBeInTheDocument();
    });
    expect(screen.queryByText("after-cancel")).not.toBeInTheDocument();
    expect(screen.queryByText("late completion")).not.toBeInTheDocument();
  });

  it("clears an accepted steer draft and immediately renders it as a user message", async () => {
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    const chatTurnRequests: Record<string, unknown>[] = [];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        chatTurnRequests.push(args?.request ?? {});
        if (chatTurnCount === 1) {
          return new Promise((_, reject) => {
            rejectOriginalTurn = reject;
          });
        }
        return new Promise(() => undefined);
      }
      if (command === "cancel_chat_stream") {
        rejectOriginalTurn?.({ code: "local_inference_cancelled" });
        return true;
      }
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

    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Write the initial answer" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));

    const liveComposer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(liveComposer, { target: { value: "Use Markdown headings" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));

    await waitFor(() => {
      expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    });
    expect(screen.getByText("Use Markdown headings")).toBeInTheDocument();
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "cancel_chat_stream"),
      ).toBe(true);
    });
    await waitFor(() => expect(chatTurnCount).toBe(2));
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      message: "Use Markdown headings",
      steering: "Use Markdown headings",
      steering_only: true,
      persist_steering_message: true,
      turn_kind: "steer",
    }));
  });

  it("approves and hydrates an explicit file before steering a live reply", async () => {
    tauriRuntimeMock.value = true;
    const prompt = "Can you view this file? file:///Users/example/Desktop/Private%20Forecast.png";
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    const chatTurnRequests: Record<string, unknown>[] = [];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false,
        decision_source: "heuristic_filter", reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "prepare_approved_chat_file") return approvedFilePreparation(
        "Private Forecast.png",
        "Visual analysis for Private Forecast.png\nDetected text:\n- Revenue forecast",
        "image/png",
        2048,
      );
      if (command === "chat_turn") {
        chatTurnCount += 1;
        chatTurnRequests.push(args?.request ?? {});
        if (chatTurnCount === 1) return new Promise((_, reject) => { rejectOriginalTurn = reject; });
        return new Promise(() => undefined);
      }
      if (command === "cancel_chat_stream") {
        rejectOriginalTurn?.({ code: "local_inference_cancelled" });
        return true;
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Write the initial answer" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));
    await waitFor(() => expect(chatTurnCount).toBe(2));
    const commands = invokeMock.mock.calls.map(([command]) => command);
    expect(commands.indexOf("prepare_approved_chat_file")).toBeLessThan(commands.lastIndexOf("chat_turn"));
    expect(chatTurnRequests[1]?.message).toContain("[approved file]");
    expect(chatTurnRequests[1]?.message).not.toContain("/Users/example/Desktop");
    expect(chatTurnRequests[1]?.message).not.toContain("file://");
    expect(chatTurnRequests[1]?.attachments).toEqual([
      expect.objectContaining({
        mime_type: "image/png",
        approved_file_receipt: expect.objectContaining({ payload: "signed-approved-file-payload" }),
      }),
    ]);
  });

  it("keeps a live reply running when an explicit steer file is denied", async () => {
    tauriRuntimeMock.value = true;
    let chatTurnCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false,
        decision_source: "heuristic_filter", reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "prepare_approved_chat_file") throw { code: "shield_approval_denied" };
      if (command === "chat_turn") {
        chatTurnCount += 1;
        return new Promise(() => undefined);
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Write the initial answer" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Open '/Users/example/Desktop/Private Forecast.png'" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));
    await screen.findByText("Permission wasn’t granted. Nothing was changed.");
    expect(chatTurnCount).toBe(1);
    expect(invokeMock.mock.calls.some(([command]) => command === "cancel_chat_stream")).toBe(false);
  });

  it("suppresses a superseded turn error after accepting a steer", async () => {
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    const chatTurnRequests: Record<string, unknown>[] = [];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        chatTurnRequests.push(args?.request ?? {});
        if (chatTurnCount === 1) {
          return new Promise((_, reject) => {
            rejectOriginalTurn = reject;
          });
        }
        return new Promise(() => undefined);
      }
      if (command === "cancel_chat_stream") return true;
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
      target: { value: "Write the initial answer" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Summarize it for me when you’re done" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));

    await waitFor(() => {
      expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    });
    expect(screen.getByText("Summarize it for me when you’re done")).toBeInTheDocument();
    await act(async () => {
      emitTauriEvent("chat://token", {
        stream_id: String(chatTurnRequests[0]?.stream_id),
        session_id: "session-1",
        turn_id: String(chatTurnRequests[0]?.turn_id),
        generation_token: String(chatTurnRequests[0]?.generation_token),
        sequence: 1,
        token: "late superseded output",
        elapsed_ms: 1,
      });
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    });
    expect(screen.queryByText("late superseded output")).not.toBeInTheDocument();
    rejectDeferred(rejectOriginalTurn, {
      code: "local_model_repetition_collapse",
      message: "The superseded local generation entered a repetition loop.",
    });

    await waitFor(() => expect(chatTurnCount).toBe(2));
    expect(view.container).not.toHaveTextContent("local_model_repetition_collapse");
    expect(screen.getAllByText("Summarize it for me when you’re done")).toHaveLength(1);
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      message: "Summarize it for me when you’re done",
      steering_only: true,
      persist_steering_message: true,
      turn_kind: "steer",
    }));
  });

  it("retains an accepted steer when the steered continuation fails", async () => {
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    let rejectSteeredTurn: ((reason?: unknown) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        return new Promise((_, reject) => {
          if (chatTurnCount === 1) {
            rejectOriginalTurn = reject;
          } else {
            rejectSteeredTurn = reject;
          }
        });
      }
      if (command === "cancel_chat_stream") {
        rejectOriginalTurn?.({ code: "local_inference_cancelled" });
        return true;
      }
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
      target: { value: "Write the initial answer" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use Markdown headings" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));
    await waitFor(() => expect(chatTurnCount).toBe(2));

    rejectDeferred(rejectSteeredTurn, {
      code: "local_model_repetition_collapse",
      message: "The steered local generation entered a repetition loop.",
    });

    await waitFor(() => {
      expect(view.container).toHaveTextContent("local_model_repetition_collapse");
    });
    expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    expect(screen.getAllByText("Use Markdown headings")).toHaveLength(1);
    expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
    expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
  });

  it("keeps an accepted steer deliverable while the user views another session", async () => {
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    const chatTurnRequests: Record<string, unknown>[] = [];
    const allSessions: ChatSession[] = [
      ...cloudSessions,
      { ...cloudSessions[0], id: "session-2", title: "Other chat" },
    ];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        chatTurnRequests.push(args?.request ?? {});
        if (chatTurnCount === 1) {
          return new Promise((_, reject) => {
            rejectOriginalTurn = reject;
          });
        }
        return new Promise(() => undefined);
      }
      if (command === "cancel_chat_stream") return true;
      if (command === "list_chat_sessions") return allSessions;
      return null;
    });

    const sharedProps = {
      agents: cloudAgents,
      configuredProviders: cloudConfiguredProviders,
      onCreateSession: vi.fn(),
      onDeleteSession: vi.fn(),
      onSelectSession: vi.fn(),
      onSessionsChange: vi.fn(),
      privacySettings: null,
      sessions: allSessions,
    };
    const view = render(
      <ChatScreen activeSessionId="session-1" {...sharedProps} />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Write the initial answer" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Use Markdown headings" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
    fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "cancel_chat_stream")).toBe(true);
    });

    view.rerender(<ChatScreen activeSessionId="session-2" {...sharedProps} />);
    rejectDeferred(rejectOriginalTurn, { code: "local_inference_cancelled" });

    await waitFor(() => expect(chatTurnCount).toBe(2));
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      session_id: "session-1",
      message: "Use Markdown headings",
      steering: "Use Markdown headings",
      persist_steering_message: true,
    }));
  });

  it("coalesces rapid steer submissions so one accepted steer is delivered", async () => {
    let chatTurnCount = 0;
    let rejectOriginalTurn: ((reason?: unknown) => void) | null = null;
    const chatTurnRequests: Record<string, unknown>[] = [];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        chatTurnCount += 1;
        chatTurnRequests.push(args?.request ?? {});
        if (chatTurnCount === 1) {
          return new Promise((_, reject) => {
            rejectOriginalTurn = reject;
          });
        }
        return new Promise(() => undefined);
      }
      if (command === "cancel_chat_stream") return true;
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
      target: { value: "Write the initial answer" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatTurnCount).toBe(1));

    for (const steer of ["Use Markdown headings", "Use a concise table instead"]) {
      fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
        target: { value: steer },
      });
      fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
      fireEvent.click(within(view.container).getByRole("menuitem", { name: "Steer reply" }));
    }

    await waitFor(() => {
      expect(screen.getByText("Use Markdown headings")).toBeInTheDocument();
      expect(screen.queryByText("Use a concise table instead")).not.toBeInTheDocument();
    });
    rejectDeferred(rejectOriginalTurn, { code: "local_inference_cancelled" });

    await waitFor(() => expect(chatTurnCount).toBe(2));
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      message: "Use Markdown headings",
      steering: "Use Markdown headings",
      persist_steering_message: true,
    }));
  });

  it("restarts queue draining when enqueue finishes after turn cleanup", async () => {
    let resolveTurn: ((response: Record<string, unknown>) => void) | null = null;
    let resolveEnqueue: ((record: unknown) => void) | null = null;
    let queued = false;
    let executions = 0;
    const queuedRecord = {
      id: 1, sessionId: "session-1", agentId: "agent-1", message: "Queued follow-up",
      attachments: [], status: "queued", createdAtMs: 1, updatedAtMs: 1,
    };
    const persisted = ["Cloud request", "Cloud answer", "Queued follow-up", "Queued answer"]
      .map<StoredChatMessage>((content, index) => ({
        id: index + 1, sessionId: "session-1", role: index % 2 === 0 ? "user" : "assistant",
        content, createdAtMs: index + 1,
      }));
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") return executions ? persisted : [];
      if (command === "get_queued_messages") return queued ? [queuedRecord] : [];
      if (command === "get_session_config") return null;
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false, decision_source: "heuristic_filter",
        reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "chat_turn") return new Promise((resolve) => { resolveTurn = resolve; });
      if (command === "queue_message") return new Promise((resolve) => { resolveEnqueue = resolve; });
      if (command === "execute_queued_messages") {
        executions += 1;
        queued = false;
        return [{ status: "completed", session_id: "session-1", text: "Queued answer" }];
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Cloud request" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(resolveTurn).not.toBeNull());
    await screen.findByText("OOMU is thinking…");
    const queueComposer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(queueComposer, { target: { value: "Queued follow-up" } });
    fireEvent.keyDown(queueComposer, { key: "Enter", code: "Enter" });
    await waitFor(() => expect(resolveEnqueue).not.toBeNull());
    resolveDeferred(resolveTurn, { text: "Cloud answer", session_id: "session-1" });
    await screen.findByText("Cloud answer");
    expect(executions).toBe(0);
    queued = true;
    resolveDeferred(resolveEnqueue, queuedRecord);
    await screen.findByText("Queued answer");
    expect(executions).toBe(1);
  });

  it("approves an explicit file before queueing it and never queues the host path", async () => {
    tauriRuntimeMock.value = true;
    const prompt = "Can you view this file? </Users/example/Desktop/Private Forecast.png>";
    const queueRequests: Record<string, unknown>[] = [];
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false,
        decision_source: "heuristic_filter", reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "chat_turn") return new Promise(() => undefined);
      if (command === "prepare_approved_chat_file") return approvedFilePreparation(
        "Private Forecast.png",
        "Visual analysis for Private Forecast.png\nDetected text:\n- Revenue forecast",
      );
      if (command === "queue_message") {
        const request = args?.request ?? {};
        const requestAttachments = Array.isArray(request.attachments) ? request.attachments : [];
        queueRequests.push({
          ...request,
          attachments: requestAttachments.map((attachment) => ({ ...attachment })),
        });
        return { id: 1, sessionId: "session-1", agentId: "agent-1", message: "queued",
          attachments: [], status: "queued", createdAtMs: 1, updatedAtMs: 1 };
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Write the initial answer" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await screen.findByText("OOMU is thinking…");
    const queueComposer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(queueComposer, { target: { value: prompt } });
    fireEvent.keyDown(queueComposer, { key: "Enter", code: "Enter" });
    await waitFor(() => expect(queueRequests).toHaveLength(1));
    const queueRequest = queueRequests[0];
    const commands = invokeMock.mock.calls.map(([command]) => command);
    expect(commands.indexOf("prepare_approved_chat_file")).toBeLessThan(commands.indexOf("queue_message"));
    expect(queueRequest?.message).toContain("[approved file]");
    expect(queueRequest?.message).not.toContain("/Users/example/Desktop");
    expect(queueRequest?.attachments).toEqual([
      expect.objectContaining({
        mime_type: "text/plain",
        approved_file_receipt: expect.objectContaining({ payload: "signed-approved-file-payload" }),
      }),
    ]);
  });

  it("discards an imperative queue refresh that resolves after its session is deleted", async () => {
    const onDeleteSession = vi.fn().mockResolvedValue(true);
    let queueReads = 0;
    let resolveLateQueueRefresh: ((queued: unknown[]) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "get_queued_messages") {
        queueReads += 1;
        if (queueReads === 1) {
          return [
            {
              id: 1,
              sessionId: "session-1",
              agentId: "agent-1",
              message: "queued before deletion",
              attachments: [],
              status: "queued",
              createdAtMs: 1,
              updatedAtMs: 1,
            },
          ];
        }
        return new Promise((resolve) => {
          resolveLateQueueRefresh = resolve;
        });
      }
      if (command === "execute_queued_messages") return [];
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={cloudAgents}
        configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={onDeleteSession}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(await within(view.container).findByRole("button", { name: "Run queue" }));
    await waitFor(() => expect(resolveLateQueueRefresh).not.toBeNull());
    fireEvent.click(within(view.container).getByRole("button", { name: "Delete Debug chat" }));
    await waitFor(() => expect(onDeleteSession).toHaveBeenCalledWith("session-1"));

    resolveDeferred(resolveLateQueueRefresh, [
      {
        id: 2,
        sessionId: "session-1",
        agentId: "agent-1",
        message: "orphaned late queue entry",
        attachments: [],
        status: "queued",
        createdAtMs: 2,
        updatedAtMs: 2,
      },
    ]);
    await waitFor(() => {
      expect(within(view.container).queryByRole("button", { name: "Run queue" })).not.toBeInTheDocument();
    });
    expect(screen.queryByText("orphaned late queue entry")).not.toBeInTheDocument();
  });

  it("gives queued execution hydration ownership over terminal reconciliation", async () => {
    let resolveTurn: ((response: Record<string, unknown>) => void) | null = null;
    let resolveExecution: ((results: unknown[]) => void) | null = null;
    let resolveStaleHydration: ((messages: StoredChatMessage[]) => void) | null = null;
    let messageReads = 0;
    let queued = false;
    const queuedRecord = {
      id: 1, sessionId: "session-1", agentId: "agent-1", message: "Queued follow-up",
      attachments: [], status: "queued", createdAtMs: 1, updatedAtMs: 1,
    };
    const persisted = ["Cloud request", "Cloud answer", "Queued follow-up", "Queued answer"]
      .map<StoredChatMessage>((content, index) => ({
        id: index + 1, sessionId: "session-1", role: index % 2 === 0 ? "user" : "assistant",
        content, createdAtMs: index + 1,
      }));
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") {
        messageReads += 1;
        if (messageReads === 1) return [];
        if (messageReads === 2) return new Promise((resolve) => { resolveStaleHydration = resolve; });
        return persisted;
      }
      if (command === "get_queued_messages") return queued ? [queuedRecord] : [];
      if (command === "get_session_config") return null;
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false, decision_source: "heuristic_filter",
        reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "chat_turn") return new Promise((resolve) => { resolveTurn = resolve; });
      if (command === "queue_message") {
        queued = true;
        return queuedRecord;
      }
      if (command === "execute_queued_messages") {
        return new Promise((resolve) => { resolveExecution = resolve; });
      }
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });
    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    const composer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Cloud request" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(resolveTurn).not.toBeNull());
    await screen.findByText("OOMU is thinking…");
    const queueComposer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(queueComposer, { target: { value: "Queued follow-up" } });
    fireEvent.keyDown(queueComposer, { key: "Enter", code: "Enter" });
    await within(view.container).findByRole("button", { name: "Run queue" });
    resolveDeferred(resolveTurn, { text: "Cloud answer", session_id: "session-1" });
    await waitFor(() => {
      expect(resolveStaleHydration).not.toBeNull();
      expect(resolveExecution).not.toBeNull();
    });
    await within(view.container).findByRole("button", { name: "Running" });
    expect(invokeMock.mock.calls.filter(([command]) => command === "compact_session_history")).toHaveLength(0);
    const blockedComposer = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(blockedComposer, { target: { value: "must wait" } });
    fireEvent.keyDown(blockedComposer, { key: "Enter", code: "Enter" });
    expect(blockedComposer).toHaveValue("must wait");
    expect(invokeMock.mock.calls.filter(([command]) => command === "chat_turn")).toHaveLength(1);
    queued = false;
    resolveDeferred(resolveExecution, [{ status: "completed", session_id: "session-1", text: "Queued answer" }]);
    await screen.findByText("Queued answer");
    await waitFor(() => expect(invokeMock.mock.calls.filter(([command]) => command === "compact_session_history")).toHaveLength(1));
    await act(async () => resolveDeferred(resolveStaleHydration, persisted.slice(0, 2)));
    expect(screen.getByText("Queued answer")).toBeInTheDocument();
    expect(invokeMock.mock.calls.filter(([command]) => command === "compact_session_history")).toHaveLength(1);
  });

  it("discards a deferred background execution refresh after a newer turn starts", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
      key === "oomu.chat.activeAgentExecution:session-1"
        ? JSON.stringify({
            executionId: "execution-1",
            planId: "plan-1",
            sessionId: "session-1",
            status: "running",
            startedAtMs: 1,
          })
        : null,
    );
    let messageReads = 0;
    let resolveBackgroundRefresh: ((messages: StoredChatMessage[]) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") {
        messageReads += 1;
        if (messageReads === 1) {
          return [
            {
              id: 1,
              sessionId: "session-1",
              role: "assistant",
              content: "initial history",
              createdAtMs: 1,
            },
          ];
        }
        if (messageReads === 2) {
          return new Promise((resolve) => {
            resolveBackgroundRefresh = resolve;
          });
        }
        return [];
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
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") return new Promise(() => undefined);
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
    await screen.findByText("initial history");
    await waitFor(() => expect(executionChannelCallbacks.size).toBe(1));
    for (const callback of executionChannelCallbacks) {
      callback({
        executionId: "execution-1",
        terminal: true,
        logs: [
          {
            id: 1,
            executionId: "execution-1",
            planId: "plan-1",
            sessionId: "session-1",
            level: "info",
            phase: "completed",
            message: "done",
            createdAtMs: 2,
          },
        ],
      });
    }
    await waitFor(() => expect(resolveBackgroundRefresh).not.toBeNull());

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "new optimistic request" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await screen.findByText("new optimistic request");
    resolveDeferred(resolveBackgroundRefresh, [
      {
        id: 2,
        sessionId: "session-1",
        role: "assistant",
        content: "stale background refresh",
        createdAtMs: 2,
      },
    ]);

    await waitFor(() => {
      expect(screen.queryByText("stale background refresh")).not.toBeInTheDocument();
    });
    expect(screen.getByText("new optimistic request")).toBeInTheDocument();
  });

  it("dismisses preflight bypass banners manually", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "ready";
      }
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
        initialBypassNotice={testBypassNotice()}
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    await waitFor(() => {
      expect(screen.getByText("Security preflight")).toBeInTheDocument();
    });

    fireEvent.click(within(view.container).getByRole("button", { name: "Dismiss" }));

    expect(screen.queryByText("Security preflight")).not.toBeInTheDocument();
  });

  it("clears preflight bypass banners when submitting a new message", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "ready";
      }
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
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Done.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return cloudSessions;
      }
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
        initialBypassNotice={testBypassNotice()}
        sessions={cloudSessions}
      />,
      { wrapper: I18nProvider },
    );

    await waitFor(() => {
      expect(screen.getByText("Security preflight")).toBeInTheDocument();
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Try again with the same request" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(screen.queryByText("Security preflight")).not.toBeInTheDocument();
    });
  });

  it("clears preflight bypass banners when switching sessions", async () => {
    const alternateSession: ChatSession = {
      ...cloudSessions[0],
      id: "session-2",
      title: "Follow-up chat",
      updatedAtMs: 2,
    };
    const sessionList = [...cloudSessions, alternateSession];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "ready";
      }
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
        initialBypassNotice={testBypassNotice()}
        sessions={sessionList}
      />,
      { wrapper: I18nProvider },
    );

    await waitFor(() => {
      expect(screen.getByText("Security preflight")).toBeInTheDocument();
    });

    view.rerender(
      <ChatScreen
        activeSessionId="session-2"
        agents={cloudAgents}
        configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessionList}
      />,
    );

    await waitFor(() => {
      expect(screen.queryByText("Security preflight")).not.toBeInTheDocument();
    });
  });

  it("pauses blocked Apple UI turns for permission recovery", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "ready";
      }
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
      if (command === "prepare_system_apple_app_tool_approval") {
        return null;
      }
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{ type: "text", text: "[\"missing value\"]" }],
          isError: false,
        };
      }
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
      target: { value: "Summarize my Messages app UI." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => expect(screen.getByRole("heading", { name: "Accessibility access needed" })).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Check again" })).toBeEnabled();
    expect(invokeMock.mock.calls.some(([command]) => command === "execute_system_apple_app_tool")).toBe(true);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });

  it("uses the native Photos reader for the exact singular library question", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
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
      if (command === "triage_local_app_intent") return true;
      if (command === "prepare_system_apple_app_tool_approval") return null;
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{ type: "text", text: "[]" }],
          structuredContent: {
            backend: "photokit",
            code: "photos_read_ok",
            authorization: "authorized",
            photos: [{
              originalFilename: "IMG_0042.HEIC",
              creationDate: "2026-07-12T20:05:00.000Z",
              pixelWidth: 4032,
              pixelHeight: 3024,
              favorite: false,
            }],
            returnedCount: 1,
            truncated: false,
          },
          isError: false,
        };
      }
      if (command === "classify_chat_intent_route") {
        const request = args?.request as { attachments?: Array<{ name?: string }> } | undefined;
        if (request?.attachments?.some((attachment) => attachment.name === "local_photos.json")) {
          return {
            route: "conversational_stream",
            requires_local_access: false,
            decision_source: "contextual_informational_topic_filter",
            reason: "The protected Photos result is already attached.",
            matched_signals: [],
            status_label: "OOMU is typing...",
          };
        }
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "protected_apple_library_read_filter",
          reason: "Protected Photos read.",
          matched_signals: ["protected Apple library read request"],
          status_label: "OOMU is checking Photos...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Your newest photo is IMG_0042.HEIC.", session_id: "session-1" };
      }
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
      target: { value: "What is the newest photo in my photo albums?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          ([command, args]) =>
            command === "execute_system_apple_app_tool" &&
            args?.toolName === "read_system_photos" &&
            args?.arguments?.max_photos === 1,
        ),
      ).toBe(true);
    });
    expect(
      invokeMock.mock.calls.some(
        ([command]) => command === "prepare_system_apple_app_tool_approval",
      ),
    ).toBe(true);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "process_agent_objective"),
    ).toBe(false);
  });

  it("routes internal name-memory requests through chat without Apple Notes approval", async () => {
    tauriRuntimeMock.value = true;
    const requestText = "Yes, call me Alex and make a note of that in your memories";
    let completed = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return completed
          ? [
              {
                id: 1,
                sessionId: "session-1",
                role: "user",
                content: requestText,
                createdAtMs: 1,
              },
              {
                id: 2,
                sessionId: "session-1",
                role: "assistant",
                content: "Got it, Alex.",
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
          decision_source: "internal_memory_profile_filter",
          reason: "Internal signed memory update.",
          matched_signals: ["internal_memory_profile"],
          status_label: "OOMU is updating conversation context...",
        };
      }
      if (command === "chat_turn") {
        completed = true;
        return { text: "Got it, Alex.", session_id: "session-1" };
      }
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
      target: { value: requestText },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await screen.findByText("Got it, Alex.");
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    const chatTurnCall = invokeMock.mock.calls.find(([command]) => command === "chat_turn");
    expect(chatTurnCall?.[1]?.request).toEqual(expect.objectContaining({
      message: requestText,
      mcp_tool_capabilities: [],
    }));
    expect(
      invokeMock.mock.calls.some(([command]) => command === "prepare_system_apple_app_tool_approval"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "execute_system_apple_app_tool"),
    ).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(false);
  });

  it("finishes an approved Apple Notes write without invoking the planner again", async () => {
    tauriRuntimeMock.value = true;
    let recorded = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return recorded
          ? [
              {
                id: 1,
                sessionId: "session-1",
                role: "user",
                content: "Write in Apple Notes: hello.",
                createdAtMs: 1,
              },
              {
                id: 2,
                sessionId: "session-1",
                role: "assistant",
                content: "Created note in Apple Notes.",
                createdAtMs: 2,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_system_apple_app_tool_approval") {
        return {
          approvalToken: "approval-note-write",
          serverName: "macos_applescript",
          toolName: "create_system_note",
          arguments: { title: "OOMU note", body: "hello." },
          message: "Create an Apple Notes note",
          capabilityRiskTier: "HIGH",
          capabilityReason: "Writes to Notes",
        };
      }
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{ type: "text", text: "Created note in Apple Notes." }],
          isError: false,
        };
      }
      if (command === "record_browser_chat_turn") {
        recorded = true;
        return {
          text: "Created note in Apple Notes.",
          session_id: "session-1",
        };
      }
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
      { wrapper: ApprovalTestProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Write in Apple Notes: hello." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const approvalDialog = await within(view.container).findByRole("dialog");
    fireEvent.click(within(approvalDialog).getByRole("button", { name: "Approve" }));

    await screen.findByText("Created note in Apple Notes.");
    const executeCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "execute_system_apple_app_tool",
    );
    expect(executeCalls).toHaveLength(1);
    expect(executeCalls[0]?.[1]).toEqual(expect.objectContaining({
      toolName: "create_system_note",
      approval: { approvalToken: "approval-note-write" },
    }));
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "record_browser_chat_turn"),
    ).toHaveLength(1);
    for (const forbiddenCommand of [
      "classify_chat_intent_route",
      "process_agent_objective",
      "chat_turn",
    ]) {
      expect(invokeMock.mock.calls.some(([command]) => command === forbiddenCommand)).toBe(false);
    }
  });

  it("surfaces a localized warning when an Apple action succeeds but its chat receipt fails", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_system_apple_app_tool_approval") {
        return {
          approvalToken: "approval-note-write",
          serverName: "macos_applescript",
          toolName: "create_system_note",
          arguments: { title: "OOMU note", body: "hello." },
          message: "Create an Apple Notes note",
          capabilityRiskTier: "HIGH",
          capabilityReason: "Writes to Notes",
        };
      }
      if (command === "execute_system_apple_app_tool") {
        return {
          content: [{ type: "text", text: "Created note in Apple Notes." }],
          isError: false,
        };
      }
      if (command === "record_browser_chat_turn") {
        throw new Error("database unavailable");
      }
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
      { wrapper: ApprovalTestProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Write in Apple Notes: hello." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    const approvalDialog = await within(view.container).findByRole("dialog");
    fireEvent.click(within(approvalDialog).getByRole("button", { name: "Approve" }));

    await screen.findByText(
      "The Notes action completed, but OOMU could not save its chat receipt.",
    );
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "execute_system_apple_app_tool"),
    ).toHaveLength(1);
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(false);
  });

  it("denies a session-bound Apple write approval when its session is deleted", async () => {
    tauriRuntimeMock.value = true;
    const onDeleteSession = vi.fn().mockResolvedValue(true);
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
      if (command === "prepare_system_apple_app_tool_approval") {
        return {
          approvalToken: "approval-session-1",
          serverName: "macos_applescript",
          toolName: "add_system_reminder",
          arguments: { title: "buy milk" },
          message: "Create a reminder",
          capabilityRiskTier: "HIGH",
          capabilityReason: "Writes to Reminders",
        };
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={cloudAgents}
        configuredProviders={cloudConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={onDeleteSession}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={cloudSessions}
      />,
      { wrapper: ApprovalTestProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Create a reminder to buy milk" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await within(view.container).findByRole("dialog");

    fireEvent.click(within(view.container).getByRole("button", { name: "Delete Debug chat" }));
    await waitFor(() => expect(onDeleteSession).toHaveBeenCalledWith("session-1"));
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "mcp_reject_tool_approval"),
      ).toBe(true);
    });
    expect(
      invokeMock.mock.calls.some(([command]) => command === "execute_system_apple_app_tool"),
    ).toBe(false);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("executes the explicit Terminal listing while the local model hydrates", async () => {
    tauriRuntimeMock.value = true;
    let reply = "";
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        return reply
          ? [
              {
                id: 20,
                sessionId: "session-1",
                role: "assistant",
                content: reply,
                createdAtMs: 20,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "loading";
      }
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
      if (command === "execute_command") {
        return {
          operation: "shell_command",
          status: "completed",
          message: "Local command completed.",
          claims: ["CLAIM command_exit status=0"],
          verified: true,
        };
      }
      if (command === "record_browser_chat_turn") {
        reply = (payload as { request: { assistant_text: string } }).request.assistant_text;
        return {
          text: reply,
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health")).toBe(true);
    });
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: TERMINAL_DOWNLOADS_LIST_PROMPT },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const calls = () => invokeMock.mock.calls.map(([name]) => name);
    await waitFor(() => expect(calls()).toContain("execute_command"));
    expect(calls()).not.toContain("execute_native_file_access");
    expect(calls()).not.toContain("chat_turn");
    const execution = invokeMock.mock.calls.find(([name]) => name === "execute_command");
    const args = execution?.[1] as { request: { action: { kind: string; content: string } } };
    expect(args.request.action).toEqual({
      kind: "shell_command",
      content: "ls ~/Downloads",
    });
  });

  it("sends an explicit host folder list through native Shield permission", async () => {
    tauriRuntimeMock.value = true;
    let recordedAssistantText = "";
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        return recordedAssistantText
          ? [{
              id: 20,
              sessionId: "session-1",
              role: "assistant",
              content: recordedAssistantText,
              createdAtMs: 20,
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
      if (command === "execute_native_file_access") {
        return {
          operation: "file_list",
          status: "completed",
          message: "report.pdf\nnotes.txt",
          claims: ["CLAIM file_list count=2"],
          verified: true,
        };
      }
      if (command === "record_browser_chat_turn") {
        recordedAssistantText = (payload as { request: { assistant_text: string } }).request.assistant_text;
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
      target: { value: "List the files in /Users/example/PrivateReports" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "execute_native_file_access")).toBe(true);
    });
    const executeCall = invokeMock.mock.calls.find(([command]) => command === "execute_native_file_access");
    const executeArgs = executeCall?.[1] as {
      request: { action: { kind: string; path: string } };
    };
    expect(executeArgs.request).toEqual({
      action: {
        kind: "file_list",
        path: "/Users/example/PrivateReports",
      },
      sessionId: "session-1",
      turnId: expect.any(String),
      generationToken: expect.any(String),
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "sign_logical_certificate")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "mcp_execute_tool")).toBe(false);
    expect(within(view.container).queryByLabelText("Browser mod")).not.toBeInTheDocument();
  });

  it("sends an explicit host file view through native Shield without planner dispatch", async () => {
    tauriRuntimeMock.value = true;
    const prompt = String.raw`Tell me what you see in this image: /Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU/oomu-profile.jpeg`;
    const approvedPrompt = "Tell me what you see in this image: [approved file]";
    let completedChatTurn = false;
    const capturedChatTurnRequests: Array<{
      message: string;
      display_message?: string;
      attachments: Array<{ name: string; approved_file_receipt?: { payload: string } }>;
    }> = [];
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        return completedChatTurn
          ? [
              {
                id: 19,
                sessionId: "session-1",
                role: "user",
                content: prompt,
                createdAtMs: 19,
              },
              {
                id: 20,
                sessionId: "session-1",
                role: "assistant",
                content: "Yes. I can view the profile image.",
                createdAtMs: 20,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_approved_chat_file") {
        return approvedFilePreparation(
          "oomu-profile.jpeg",
          "Visual analysis for oomu-profile.jpeg\nThe image contains a profile portrait.",
          "image/jpeg",
          4096,
        );
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "incorrect_runtime_fallback",
          reason: "This must never be consulted after a verified direct file read.",
          matched_signals: ["incorrect fallback"],
          status_label: "Planning…",
        };
      }
      if (command === "chat_turn") {
        const request = (payload as {
          request: {
            message: string;
            display_message?: string;
            attachments: Array<{ name: string; approved_file_receipt?: { payload: string } }>;
          };
        }).request;
        capturedChatTurnRequests.push({
          message: request.message,
          display_message: request.display_message,
          attachments: request.attachments.map((attachment) => ({ ...attachment })),
        });
        completedChatTurn = true;
        return {
          text: "Yes. I can view the profile image.",
          session_id: "session-1",
        };
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
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "prepare_approved_chat_file")).toBe(true);
    });
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });
    const executeCall = invokeMock.mock.calls.find(([command]) => command === "prepare_approved_chat_file");
    const commandOrder = invokeMock.mock.calls.map(([command]) => command);
    expect(commandOrder.indexOf("prepare_approved_chat_file")).toBeLessThan(
      commandOrder.indexOf("chat_turn"),
    );
    expect(executeCall?.[1]).toEqual({
      request: {
        access: {
          action: {
            kind: "file_read",
            path: "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU/oomu-profile.jpeg",
          },
          sessionId: "session-1",
          turnId: expect.any(String),
          generationToken: expect.any(String),
        },
        displayMessage: prompt,
      },
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "sign_logical_certificate")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
    const chatTurnRequest = capturedChatTurnRequests[0];
    expect(chatTurnRequest).toBeDefined();
    expect(chatTurnRequest.message).toBe(approvedPrompt);
    expect(chatTurnRequest.message).not.toContain("/Users/example/Library");
    expect(chatTurnRequest.message).not.toContain("file://");
    expect(chatTurnRequest.display_message).toBe(prompt);
    expect(chatTurnRequest.attachments).toHaveLength(1);
    expect(chatTurnRequest.attachments).toEqual([
      expect.objectContaining({
        name: "oomu-profile.jpeg",
        mime_type: "image/jpeg",
        approved_file_receipt: expect.objectContaining({ payload: "signed-approved-file-payload" }),
      }),
    ]);
    await screen.findByText(prompt);
    expect(screen.queryByText(approvedPrompt)).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([command]) => command === "read_local_context")).toBe(false);
  });

  it("persists an accurate terminal result when accepted file preflight is denied", async () => {
    tauriRuntimeMock.value = true;
    const prompt = "View '/Users/example/Desktop/Private Forecast.png'";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_approved_chat_file") {
        throw { code: "shield_approval_denied" };
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
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const failureText = "Permission wasn’t granted. Nothing was changed.";
    await screen.findByText(failureText);
    expect(screen.getByText(prompt)).toBeInTheDocument();
    const accepted = invokeMock.mock.calls.find(([command]) => command === "accept_chat_turn")?.[1] as {
      request: { turn_id: string };
    };
    const finalized = invokeMock.mock.calls.find(
      ([command]) => command === "finalize_accepted_chat_turn",
    )?.[1] as { request: { turn_id: string; content: string; status: string } };
    expect(finalized.request).toEqual(expect.objectContaining({
      turn_id: accepted.request.turn_id,
      content: failureText,
      status: "failed",
    }));
    expect(invokeMock.mock.calls.some(([command]) => command === "sign_logical_certificate")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });

  it("reports a native Mail error envelope without falling through to planner or model routing", async () => {
    tauriRuntimeMock.value = true;
    let recordedAssistantText = "";
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages") {
        return recordedAssistantText
          ? [
              {
                id: 20,
                sessionId: "session-1",
                role: "assistant",
                content: recordedAssistantText,
                createdAtMs: 20,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "loading";
      }
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
        return MAIL_READ_FAILURE_RESULT;
      }
      if (command === "record_browser_chat_turn") {
        recordedAssistantText = (payload as { request: { assistant_text: string } }).request.assistant_text;
        return {
          text: recordedAssistantText,
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health")).toBe(true);
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Do I have any unread emails?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(recordedAssistantText).toContain("Mail did not return usable results");
    });
    expect(recordedAssistantText).toContain("Mail could not read the inbox.");
    expect(recordedAssistantText).toContain("not because your inbox is clear");
    expect(recordedAssistantText).not.toContain("Mail context blocked");
    expect(invokeMock).toHaveBeenCalledWith("read_system_emails", { maxMessages: 20, unreadOnly: true, turnContext: expect.objectContaining({ sessionId: "session-1", turnId: expect.any(String), generationToken: expect.any(String) }) });
    expect(invokeMock.mock.calls.some(([command]) => command === "prepare_system_apple_app_tool_approval")).toBe(false);
    expect(within(view.container).queryByRole("dialog")).toBeNull();
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });

  it("routes broad local Mac tasks to the planner while the local model is hydrating", async () => {
    tauriRuntimeMock.value = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "get_local_generation_health") {
        return "loading";
      }
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
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "No explicit action rule matched.",
          matched_signals: [],
          status_label: "OOMU is contacting the selected model...",
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "plan-1",
          objective: "Take a screenshot of my screen.",
          steps: [
            {
              step: "Evaluate the requested local Mac screen-capture task.",
              tool: {
                kind: "unsupported",
                requested: "screen capture",
              },
              risk_level: "medium",
            },
          ],
          exit_condition: "Return an approval-gated plan or unsupported capability notice.",
          trusted_automatic_execution: false,
          model_route: {
            reason: "Planner forced by broad local Mac task intent.",
            requires_principal_authorization: true,
          },
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "get_local_generation_health")).toBe(true);
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Take a screenshot of my screen." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(true);
    });
    const nativePlannerCall = invokeMock.mock.calls.find(
      ([command]) => command === "process_agent_objective",
    );
    expect(
      (nativePlannerCall?.[1] as { request: Record<string, unknown> }).request.user_objective,
    ).toBe("Take a screenshot of my screen.");
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
  });

  it("executes direct host file creation through Shield without model dispatch", async () => {
    let recorded = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return recorded
          ? [
              {
                id: 10,
                sessionId: "session-1",
                role: "user",
                content:
                  'Create a new markdown file in my Downloads directory called Hello World.md with the content "Hello World".',
                createdAtMs: 10,
              },
              {
                id: 11,
                sessionId: "session-1",
                role: "assistant",
                content:
                  "Shield Gate approved and wrote 11 byte(s) to /Users/example/Downloads/Hello World.md.",
                createdAtMs: 11,
              },
            ]
          : [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "execute_command") {
        return {
          operation: "file_write",
          status: "completed",
          message:
            "Shield Gate approved and wrote 11 byte(s) to /Users/example/Downloads/Hello World.md.",
          claims: [],
          verified: true,
        };
      }
      if (command === "record_browser_chat_turn") {
        recorded = true;
        return {
          text: "Shield Gate approved and wrote 11 byte(s) to /Users/example/Downloads/Hello World.md.",
          session_id: "session-1",
        };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      expect(screen.getByText(/Shield Gate approved and wrote 11 byte/)).toBeInTheDocument();
    });

    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
    const executeCall = invokeMock.mock.calls.find(([command]) => command === "execute_command");
    expect(executeCall?.[1]).toEqual({
      request: expect.objectContaining({
        action: {
          kind: "file_write",
          path: "~/Downloads/Hello World.md",
          content: "Hello World",
        },
        logical_certificate: null,
        session_id: "session-1",
        turn_id: expect.any(String),
        generation_token: expect.any(String),
      }),
    });
  });

  it("routes /compact to semantic compaction without sending a chat turn", async () => {
    let compacted = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return compacted
          ? [
              {
                id: 99,
                sessionId: "session-1",
                role: "system",
                content:
                  "Compacted conversation excerpts. Every entry below is a bounded extract from a persisted source message.\n[source message_id=42 role=assistant sha256=abc123] Keep the current implementation moving.",
                compactionType: "summary_anchor",
                createdAtMs: 99,
              },
            ]
          : storedMessages;
      }
      if (command === "get_session_context_status") {
        return {
          estimatedTokensUsed: compacted ? 24 : 240,
          tokensTotal: 8192,
          estimatedPercentageUsed: compacted ? 0.006 : 0.058,
          activeModelId: "model-1",
          isCloudModel: false,
        };
      }
      if (command === "compact_chat_session") {
        compacted = true;
        return { compactedMessageCount: 1 };
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    const input = within(view.container).getByPlaceholderText("Message OOMU…");
    fireEvent.change(input, { target: { value: "/compact" } });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "compact_chat_session")).toBe(true);
    });

    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(false);
    const summary = await screen.findByText("Compaction summary");
    const details = summary.closest("details");

    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");
    expect(details?.textContent).toContain("source message_id=42");
    expect(details?.textContent).toContain("sha256=abc123");

    fireEvent.click(summary);
    expect(details).toHaveAttribute("open");

    fireEvent.click(summary);
    expect(details).not.toHaveAttribute("open");

    const checkpointBubble = screen.getByText("Memory checkpoint").closest("div");
    expect(checkpointBubble?.className).toContain("bg-[var(--accent-background)]");
    expect(checkpointBubble?.className).not.toContain("destructive");
    expect(screen.queryByText("System")).not.toBeInTheDocument();
    expect(input).toHaveValue("");
  });

  it("renders assistant logical certificates in a collapsed details block", async () => {
    const assistantMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "assistant",
        content:
          "Done.\n\n---\nPremises:\nThe implementation changed.\nExecution Path:\nThe chat renderer split the response.\nFormal Conclusion:\nThe certificate is preserved.",
        createdAtMs: 1,
      },
    ];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return assistantMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    render(
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

    await waitFor(() => {
      expect(screen.getByText("Done.")).toBeInTheDocument();
    });

    const summary = screen.getByText("Logical Certificate");
    const details = summary.closest("details");

    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");
    expect(details).toHaveClass("group");
    expect(details?.textContent).toContain("Premises");
    expect(details?.textContent).toContain("Formal Conclusion");
    expect(details?.textContent).toContain("The certificate is preserved.");

    fireEvent.click(summary);

    expect(details).toHaveAttribute("open");
  });

  it("renders user-authored markdown and tags as inert plain text", async () => {
    const userMessages: StoredChatMessage[] = [
      {
        id: 1,
        sessionId: "session-1",
        role: "user",
        content: "Literal **bold** </text> [link](https://example.com)",
        createdAtMs: 1,
      },
    ];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return userMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
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

    await waitFor(() => {
      expect(screen.getByText("Literal **bold** </text> [link](https://example.com)")).toBeInTheDocument();
    });

    expect(view.container.querySelector("strong")).toBeNull();
    expect(within(view.container).queryByRole("link", { name: "link" })).not.toBeInTheDocument();
  });

  it("does not invoke local context reads for literal tag text in the composer", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Done.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      target: { value: "The literal </text> tag leaked into the previous response." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });

    expect(invokeMock.mock.calls.some(([command]) => command === "read_local_context")).toBe(false);
  });

  it("does not render legacy adaptive suggestion buttons above the prompt", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return storedMessages;
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    render(
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

    await waitFor(() => {
      expect(screen.getByText(storedMessages[0].content)).toBeInTheDocument();
    });

    expect(screen.queryByRole("button", { name: "Inspect the failing path" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Review the file" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Plan verification" })).not.toBeInTheDocument();
  });

  it("sends only one context budget field when starting a chat turn", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Done.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      target: { value: "Hello there" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });

    const chatTurnCall = invokeMock.mock.calls.find(([command]) => command === "chat_turn");
    const chatTurnArgs = chatTurnCall?.[1] as { request: Record<string, unknown> };
    const classifyCall = invokeMock.mock.calls.find(([command]) => command === "classify_chat_intent_route");
    const classifyArgs = classifyCall?.[1] as Record<string, unknown>;

    expect(classifyArgs).toEqual(expect.objectContaining({
      session_id: "session-1",
      selected_provider_id: "provider-1",
      selected_model_id: "model-1",
    }));
    expect(chatTurnArgs.request.provider_id).toBe("provider-1");
    expect(chatTurnArgs.request.model_id).toBe("model-1");
    expect(chatTurnArgs.request.dynamic_routing_override).toBe(false);
    expect(chatTurnArgs.request.context_budget).toBe(12_288);
    expect(chatTurnArgs.request).not.toHaveProperty("contextBudget");
  });

  it("releases a cloud turn before hydration and safely accepts a local follow-up", async () => {
    let reads = 0;
    let turnCount = 0;
    let resolveCloudTurn: ((response: Record<string, unknown>) => void) | null = null;
    let releaseStaleHydration: (messages: StoredChatMessage[]) => void = () => undefined;
    const staleHydration = new Promise<StoredChatMessage[]>((resolve) => { releaseStaleHydration = resolve; });
    const persistedMessages = ["Cloud request", "Cloud answer", "Local follow-up", "Local answer"]
      .map<StoredChatMessage>((content, index) => ({
        id: index + 1, sessionId: "session-1",
        role: index % 2 === 0 ? "user" : "assistant",
        content, createdAtMs: index + 1,
      }));
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") {
        reads += 1;
        if (reads === 1) return [];
        return reads === 2 ? staleHydration : persistedMessages;
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "classify_chat_intent_route") return {
        route: "conversational_stream", requires_local_access: false, decision_source: "heuristic_filter",
        reason: "test", matched_signals: [], status_label: "Thinking...",
      };
      if (command === "chat_turn") {
        turnCount += 1;
        if (turnCount === 1) return new Promise((resolve) => { resolveCloudTurn = resolve; });
        return {
          text: "Local answer",
          session_id: String(args?.request?.session_id ?? "session-1"),
          metadata: {
            executing_provider_id: "local_model",
            executing_model_id: "gemma-4-12B-it-qat-q4_0-gguf",
          },
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });
    const view = render(
      <ChatScreen
        activeSessionId="session-1" agents={agents} configuredProviders={configuredProviders}
        onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()} privacySettings={null} sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );
    const submit = (message: string) => {
      fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
        target: { value: message },
      });
      fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    };
    submit("Cloud request");
    await waitFor(() => {
      expect(within(view.container).getByPlaceholderText("Message OOMU…")).toHaveValue("");
    });
    expect(await screen.findByText("OOMU is thinking…")).toBeInTheDocument();
    resolveDeferred(resolveCloudTurn, {
      text: "Cloud answer", session_id: "session-1",
      metadata: { executing_provider_id: "gemini", executing_model_id: "gemini-3.5-flash" },
    });
    await screen.findByText("Cloud answer");
    await waitFor(() => expect(within(view.container).getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
    expect(reads).toBeGreaterThanOrEqual(2);
    submit("Local follow-up");
    await screen.findByText("Local answer");
    await waitFor(() => expect(turnCount).toBe(2));
    await act(async () => releaseStaleHydration(persistedMessages.slice(0, 2)));
    await waitFor(() => {
      expect(screen.getByText("Local answer")).toBeInTheDocument();
      expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
    });
  });

  it("uses attached local mail data instead of invoking the Mail reader", async () => {
    tauriRuntimeMock.value = true;
    (window as Window & { __TAURI_IPC__?: unknown }).__TAURI_IPC__ = {};
    let pickerRequest: Record<string, unknown> | null = null;
    let pickerTurnId = "";
    let readRequest: Record<string, unknown> | null = null;
    let chatTurnAttachmentNames: string[] = [];
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "choose_local_context") {
        pickerRequest = args?.request as Record<string, unknown>;
        pickerTurnId = String(pickerRequest.turnId ?? "");
        return {
          results: [
            {
              name: "local_mail.json",
              ok: true,
              grantId: "a".repeat(64),
              mimeType: "application/json",
              decodedByteCount: 86,
              encodedByteCount: 0,
              expiresAtMs: Date.now() + 60_000,
              errorCode: null,
            },
          ],
          countLimit: 5,
          decodedByteLimit: 20 * 1024 * 1024,
          encodedByteLimit: 28 * 1024 * 1024,
        };
      }
      if (command === "read_local_context") {
        readRequest = args?.request as Record<string, unknown>;
        return {
          name: "local_mail.json",
          mime_type: "application/json",
          byte_count: 86,
          text: '[{"sender":"alex@example.com","subject":"Hello","content":"Can you review this?"}]',
          truncated: false,
        };
      }
      if (command === "revoke_local_context_grants") {
        return { revokedCount: 0 };
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_local_context_filter",
          confidence: 0.97,
          reason: "attached mail",
          matched_signals: [],
          status_label: "Reading attached mail...",
        };
      }
      if (command === "read_system_emails") {
        throw new Error("Mail reader should be gated by the attachment.");
      }
      if (command === "chat_turn") {
        const request = args?.request as { attachments?: Array<{ name?: string }> } | undefined;
        chatTurnAttachmentNames = request?.attachments?.flatMap((attachment) =>
          attachment.name ? [attachment.name] : []) ?? [];
        return { text: "Summarized attached mail.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    fireEvent.click(within(view.container).getByRole("button", { name: "Attach file" }));

    await waitFor(() => {
      expect(screen.getByText("local_mail.json")).toBeInTheDocument();
    });

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Check my emails." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "read_system_emails")).toBe(false);
    expect(chatTurnAttachmentNames).toContain("local_mail.json");
    expect(pickerRequest).toMatchObject({
      sessionId: "session-1",
      operation: "read",
      turnId: expect.stringMatching(/^attachment-/),
    });
    expect(pickerRequest).not.toHaveProperty("path");
    expect(readRequest).toMatchObject({
      grantId: "a".repeat(64),
      sessionId: "session-1",
      turnId: pickerTurnId,
    });
    expect(readRequest).not.toHaveProperty("path");
  });

  it("uses conservative local context steps when the hardware profile is unavailable", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
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

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));
    const slider = within(view.container).getByRole("slider", { name: "Context budget" }) as HTMLInputElement;

    expect(slider.min).toBe("0");
    expect(slider.max).toBe("2");
    expect(slider.step).toBe("1");
    expect(slider.value).toBe("2");
    expect(within(view.container).getByText("4K")).toBeInTheDocument();
    expect(within(view.container).getByText("8K")).toBeInTheDocument();
    expect(within(view.container).getByText("12K")).toBeInTheDocument();
    expect(within(view.container).queryByText("16K")).not.toBeInTheDocument();
  });

  it("uses cloud context slider bounds and default for cloud model routes", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
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

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));
    const slider = within(view.container).getByRole("slider", { name: "Context budget" }) as HTMLInputElement;

    expect(slider.min).toBe("0");
    expect(slider.max).toBe("6");
    expect(slider.step).toBe("1");
    expect(slider.value).toBe("1");
  });

  it("does not turn HOME path prompts into directory attachments", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "hydrated_local_context_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Reading folder...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Here is the architectural comparison.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      target: { value: "Can you inspect the Downloads folder?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });

    const chatTurnCall = invokeMock.mock.calls.find(([command]) => command === "chat_turn");
    const chatTurnArgs = chatTurnCall?.[1] as { request: { attachments: Array<{ text?: string }> } };

    expect(invokeMock.mock.calls.some(([command]) => command === "read_local_context")).toBe(false);
    expect(chatTurnArgs.request.attachments).toEqual([]);
    expect(JSON.stringify(chatTurnArgs.request.attachments)).not.toContain("/Users/example/Downloads");
  });

  it("accepts and routes a compound prompt with an absolute path without silently dropping it", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Done.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") {
        return sessions;
      }
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

    const prompt = "Compare the architecture described by `/Users/example/My Files/Missing.md` with our current approach and recommend a rollout; do not open the path.";
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await screen.findByText(prompt);
    await waitFor(() => {
      expect(invokeMock.mock.calls.map(([command]) => command)).toContain("chat_turn");
    });

    expect(invokeMock.mock.calls.some(([command]) => command === "read_local_context")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "execute_command")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "accept_chat_turn")).toBe(true);
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(true);
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
  });

  it("falls through to conversation when the backend rejects a false planner route", async () => {
    let chatCompleted = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return chatCompleted
          ? [{
              id: 91,
              sessionId: "session-1",
              role: "assistant",
              content: "Most users will not know every capability yet.",
              createdAtMs: 91,
            }]
          : [];
      }
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "heuristic_filter",
          reason: "A stale frontend classifier thought this needed a plan.",
          matched_signals: ["file extension: .9"],
          status_label: "Planning...",
        };
      }
      if (command === "process_agent_objective") {
        throw Object.assign(new Error("This objective is conversational."), {
          code: "agent_objective_not_executable",
        });
      }
      if (command === "chat_turn") {
        chatCompleted = true;
        return { text: "Most users will not know every capability yet.", session_id: "session-1" };
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
      target: { value: "99.9% of users will not know that." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });
    expect(await screen.findByText("Most users will not know every capability yet.")).toBeVisible();
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(true);
    expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    expect(
      (invokeMock.mock.calls.find(([command]) => command === "process_agent_objective")?.[1] as {
        request: Record<string, unknown>;
      }).request.user_objective,
    ).toBe("99.9% of users will not know that.");
    expect(screen.queryByText(/agent_objective_not_executable/i)).not.toBeInTheDocument();
  });

  it("turns backend route escalations into pending action plans", async () => {
    const planPersistence = createPlanPersistenceMock("session-1");
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return planPersistence.listMessages();
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return {
          text: "Pivoting to Agentic Planner.",
          session_id: "session-1",
          route_escalation: {
            route: "agentic_planner",
            requires_local_access: true,
            decision_source: "server_preflight",
            confidence: 0.99,
            reason: "Local command intent detected.",
            matched_signals: ["standard user folder: ~/Downloads"],
            status_label: "OOMU is planning local actions...",
          },
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "plan-1",
          objective: "List Downloads",
          steps: [],
          exit_condition: "Plan approved or rejected.",
          trusted_automatic_execution: false,
          model_route: {
            reason: "test",
            requires_principal_authorization: true,
          },
        };
      }
      if (command === "record_browser_chat_turn") return planPersistence.record(args);
      if (command === "list_chat_sessions") {
        return sessions;
      }
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
      target: { value: "Review Downloads" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(screen.getByText(/Action plan compiled/)).toBeInTheDocument();
    });

    expect(screen.getByText(/Plan ID: plan-1/)).toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([command]) => command === "process_agent_objective")).toBe(true);
    const plannerCall = invokeMock.mock.calls.find(([command]) => command === "process_agent_objective");
    const plannerArgs = plannerCall?.[1] as { request: Record<string, unknown> };
    expect(plannerArgs.request).toEqual(expect.objectContaining({
      user_objective: "Review Downloads",
      selected_model: "local_gemma",
      selected_provider_id: "local_model",
      selected_model_id: "model-1",
    }));
  });

  it("discards a deferred planner response after its session is deleted", async () => {
    const onDeleteSession = vi.fn().mockResolvedValue(true);
    let resolvePlan: ((plan: Record<string, unknown>) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_messages") return [];
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "Local mutation requires an approval-gated plan.",
          matched_signals: ["modify local workspace"],
          status_label: "Planning...",
        };
      }
      if (command === "process_agent_objective") {
        return new Promise((resolve) => {
          resolvePlan = resolve;
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
        onDeleteSession={onDeleteSession}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Modify the local workspace configuration safely." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(resolvePlan).not.toBeNull());

    fireEvent.click(within(view.container).getByRole("button", { name: "Delete Debug chat" }));
    await waitFor(() => expect(onDeleteSession).toHaveBeenCalledWith("session-1"));
    resolveDeferred(resolvePlan, {
      id: "late-plan",
      objective: "Late deleted-session plan",
      steps: [],
      exit_condition: "Plan approved or rejected.",
      trusted_automatic_execution: false,
      model_route: {
        reason: "test",
        requires_principal_authorization: true,
      },
    });

    await waitFor(() => {
      expect(screen.queryByText(/Action plan compiled/)).not.toBeInTheDocument();
      expect(screen.queryByText(/late-plan/)).not.toBeInTheDocument();
    });
  });

  it("hands an explicitly approved plan to the native runtime for atomic lease provisioning", async () => {
    const planPersistence = createPlanPersistenceMock("session-1");
    const dynamicSessions = [{ ...sessions[0], dynamicRoutingOverride: true }];
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return planPersistence.listMessages();
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "Local telemetry archive requires approval.",
          matched_signals: ["telemetry archive"],
          status_label: "Planning...",
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "telemetry-plan",
          objective: "Create the approved telemetry audit.",
          steps: [
            {
              step: "Inspect local process state.",
              tool: { kind: "system_audit", scope: "processes" },
              risk_level: "low",
            },
            {
              step: "Package the telemetry audit.",
              tool: {
                kind: "telemetry_archive",
                output_path: "/tmp/testing/telemetry_audit.tar.gz",
              },
              risk_level: "high",
            },
            {
              step: "Write the approved summary.",
              tool: { kind: "file_write", path: "/tmp/testing/summary.md", content: "done" },
              risk_level: "high",
            },
          ],
          exit_condition: "Stop after the approved audit is packaged.",
          trusted_automatic_execution: false,
          model_route: {
            reason: "test",
            requires_principal_authorization: true,
          },
        };
      }
      if (command === "record_browser_chat_turn") return { ...planPersistence.record(args), metadata: { executingProviderId: "provider-1", executingModelId: "model-1" } };
      if (command === "request_agent_plan_authority") {
        return {
          authorityProofId: "telemetry-authority-proof",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "spawn_agent_execution") {
        return {
          executionId: "telemetry-execution",
          planId: "telemetry-plan",
          sessionId: "session-1",
        };
      }
      if (command === "list_chat_sessions") return dynamicSessions;
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
        sessions={dynamicSessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Create a local telemetry archive in the testing directory." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(screen.getByText("Create the approved telemetry audit.")).toBeInTheDocument());

    fireEvent.click(within(view.container).getByRole("button", { name: "Approve & execute" }));
    await waitFor(() =>
      expect(invokeMock.mock.calls.some(([command]) => command === "spawn_agent_execution")).toBe(true),
    );
    expect(screen.queryByText(/Execution ID: telemetry-execution/)).not.toBeInTheDocument();

    expect(invokeMock.mock.calls.some(([command]) => command === "grant_actuation_lease")).toBe(false);
    const spawnCall = invokeMock.mock.calls.find(([command]) => command === "spawn_agent_execution");
    expect(spawnCall?.[1]).toMatchObject({
      request: {
        principal_approved: true,
        authority_proof_id: "telemetry-authority-proof",
        plan: { id: "telemetry-plan" },
        turn_context: { sessionId: "session-1", providerId: "provider-1", modelId: "model-1" },
      },
    });
  });

  it("binds background execution to the planner turn and discards a late start after deletion", async () => {
    const onDeleteSession = vi.fn().mockResolvedValue(true);
    let resolveExecution: ((response: Record<string, unknown>) => void) | null = null;
    const planPersistence = createPlanPersistenceMock("session-1");
    invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return planPersistence.listMessages();
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: true,
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "Local mutation requires an approval-gated plan.",
          matched_signals: ["modify local workspace"],
          status_label: "Planning...",
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "bound-plan",
          objective: "Bound background execution",
          steps: [{
            step: "Inspect the approved workspace.",
            tool: { kind: "file_list", path: "/tmp" },
            risk_level: "low",
          }],
          exit_condition: "Stop after the approved step.",
          trusted_automatic_execution: false,
          model_route: {
            reason: "test",
            requires_principal_authorization: true,
          },
        };
      }
      if (command === "record_browser_chat_turn") return planPersistence.record(args);
      if (command === "request_agent_plan_authority") {
        return {
          authorityProofId: "bound-authority-proof",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "spawn_agent_execution") {
        return new Promise((resolve) => {
          resolveExecution = resolve;
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
        onDeleteSession={onDeleteSession}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Modify the local workspace configuration safely." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(screen.getByText("Bound background execution")).toBeInTheDocument());
    fireEvent.click(within(view.container).getByRole("button", { name: "Approve & execute" }));
    await waitFor(() => expect(resolveExecution).not.toBeNull());

    const spawnCall = invokeMock.mock.calls.find(([command]) => command === "spawn_agent_execution");
    const spawnRequest = (spawnCall?.[1] as { request: Record<string, unknown> }).request;
    expect(spawnRequest).toEqual(expect.objectContaining({
      plan: expect.objectContaining({ id: "bound-plan" }),
      turn_context: expect.objectContaining({
        turnId: expect.any(String),
        generationToken: expect.any(String),
        sessionId: "session-1",
        agentId: "agent-1",
        providerId: "provider-1",
        modelId: "model-1",
        rootTurnId: expect.any(String),
        turnKind: "root",
        attachmentGrants: [],
        createdAtMs: expect.any(Number),
      }),
    }));

    fireEvent.click(within(view.container).getByRole("button", { name: "Delete Debug chat" }));
    await waitFor(() => expect(onDeleteSession).toHaveBeenCalledWith("session-1"));
    resolveDeferred(resolveExecution, {
      executionId: "late-execution",
      planId: "bound-plan",
      sessionId: "session-1",
    });

    await waitFor(() => {
      expect(screen.queryByText(/Execution ID: late-execution/)).not.toBeInTheDocument();
      expect(screen.queryByText(/Progress is streaming below/)).not.toBeInTheDocument();
    });
  });

  it("creates new sessions with the agent default route binding", async () => {
    const onCreateSession = vi.fn(async (_agentId: string, route: { providerId: string; modelId: string }) => ({
      ...sessions[0],
      id: "session-new",
      providerId: route.providerId,
      modelId: route.modelId,
    }));
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return null;
      }
      if (command === "classify_chat_intent_route") {
        return {
          route: "conversational_stream",
          requires_local_access: false,
          decision_source: "deterministic_action_rules",
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        return { text: "Paris.", session_id: "session-new" };
      }
      if (command === "list_chat_sessions") {
        return [{ ...sessions[0], id: "session-new", providerId: "provider-1", modelId: "model-1" }];
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId=""
        agents={agents}
        configuredProviders={configuredProviders}
        onCreateSession={onCreateSession}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[]}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "What is the capital of France?" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(onCreateSession).toHaveBeenCalledWith("agent-1", {
        providerId: "provider-1",
        modelId: "model-1",
      });
    });
  });

  it("moves the first optimistic turn from the new-chat scope into its created session", async () => {
    let completed = false;
    const createdSession = { ...sessions[0], id: "session-new" };
    const onCreateSession = vi.fn(async () => createdSession);
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const sessionId = String(args?.session_id ?? args?.sessionId ?? "");
      if (command === "list_chat_messages") {
        if (completed && sessionId === "session-new") {
          return [
            {
              id: 1,
              sessionId,
              role: "user",
              content: "first scoped request",
              createdAtMs: 1,
            },
            {
              id: 2,
              sessionId,
              role: "assistant",
              content: "first scoped response",
              createdAtMs: 2,
            },
          ];
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
          decision_source: "heuristic_filter",
          confidence: 1,
          reason: "test",
          matched_signals: [],
          status_label: "Thinking...",
        };
      }
      if (command === "chat_turn") {
        completed = true;
        return { text: "first scoped response", session_id: "session-new" };
      }
      if (command === "list_chat_sessions") return [createdSession];
      return null;
    });

    function FirstTurnHarness() {
      const [activeSessionId, setActiveSessionId] = useState("");
      return (
        <ChatScreen
          activeSessionId={activeSessionId}
          agents={agents}
          configuredProviders={configuredProviders}
          onCreateSession={onCreateSession}
          onDeleteSession={vi.fn()}
          onSelectSession={setActiveSessionId}
          onSessionsChange={vi.fn()}
          privacySettings={null}
          sessions={activeSessionId ? [createdSession] : []}
        />
      );
    }

    const view = render(<FirstTurnHarness />, { wrapper: I18nProvider });
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "first scoped request" },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await screen.findByText("first scoped request");
    await screen.findByText("first scoped response");
    expect(onCreateSession).toHaveBeenCalledTimes(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "chat_turn")).toHaveLength(1);
  });

  it("disables tuning controls until a session exists", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId=""
        agents={agents}
        configuredProviders={[configuredProviders[0], geminiConfiguredProviders[0]]}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[]}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));

    expect(within(view.container).getByRole("combobox", { name: "Provider" })).toBeDisabled();
    expect(within(view.container).getByRole("combobox", { name: "Model" })).toBeDisabled();
    expect(within(view.container).getByRole("slider", { name: "Context budget" })).toBeDisabled();
    expect(within(view.container).getByRole("radio", { name: "Off" })).toBeDisabled();
    expect(within(view.container).getByRole("radio", { name: "On" })).toBeDisabled();
  });

  it("starts each new chat from the agent default instead of the active session route", async () => {
    modelRoutingPreferencesMock.primaryRoute = {
      providerConfigId: "provider-1",
      providerId: "provider-1",
      modelId: "model-1",
      label: "Local / model-1",
      updatedAt: 1,
    };
    const agentWithDifferentDefault: ChatAgent[] = [
      {
        ...agents[0],
        endpoint: {
          provider: "provider-1",
          modelId: "model-2",
        },
      },
    ];
    const configuredProvidersWithModels: ConfiguredProvider[] = [
      {
        ...configuredProviders[0],
        customModelIds: "model-1\nmodel-2",
      },
    ];
    const activeSessionOnModelOne: ChatSession = {
      ...sessions[0],
      modelId: "model-1",
    };
    const onCreateSession = vi.fn(async (_agentId: string, route: { providerId: string; modelId: string }) => ({
      ...activeSessionOnModelOne,
      id: "session-new",
      providerId: route.providerId,
      modelId: route.modelId,
    }));
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return {
          providerId: "provider-1",
          modelId: "model-1",
          reasoningDepth: "medium",
          contextBudget: 4096,
        };
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agentWithDifferentDefault}
        configuredProviders={configuredProvidersWithModels}
        onCreateSession={onCreateSession}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[activeSessionOnModelOne]}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "New chat" }));

    await waitFor(() => {
      expect(onCreateSession).toHaveBeenCalledWith("agent-1", {
        providerId: "provider-1",
        modelId: "model-2",
      });
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_session_config",
        expect.objectContaining({
          session_id: "session-new",
          provider_id: "provider-1",
          model_id: "model-2",
        }),
      );
    });
  });

  it("keeps tuning route changes stable while session config persistence is in flight", async () => {
    const multiConfiguredProviders: ConfiguredProvider[] = [
      configuredProviders[0],
      geminiConfiguredProviders[0],
    ];
    let resolveSaveSessionConfig: (() => void) | null = null;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return {
          providerId: "provider-1",
          modelId: "model-1",
          reasoningDepth: "off",
          contextBudget: 8192,
        };
      }
      if (command === "save_session_config") {
        return new Promise<void>((resolve) => {
          resolveSaveSessionConfig = resolve;
        });
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={multiConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));

    const providerSelect = within(view.container).getByRole("combobox", { name: "Provider" }) as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: "gemini-provider-1" } });

    await waitFor(() => {
      expect(providerSelect.value).toBe("gemini-provider-1");
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_session_config",
        expect.objectContaining({
          session_id: "session-1",
          provider_id: "gemini-provider-1",
          model_id: "gemini-3.5-flash",
        }),
      );
    });

    const initialConfigReads = invokeMock.mock.calls.filter(([command]) => command === "get_session_config").length;

    view.rerender(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={multiConfiguredProviders.map((provider) => ({ ...provider }))}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[{ ...sessions[0] }]}
      />,
    );

    const rerenderedProviderSelect = within(view.container).getByRole("combobox", { name: "Provider" }) as HTMLSelectElement;
    const rerenderedModelSelect = within(view.container).getByRole("combobox", { name: "Model" }) as HTMLSelectElement;
    expect(rerenderedProviderSelect.value).toBe("gemini-provider-1");
    expect(rerenderedModelSelect.value).toBe("gemini-3.5-flash");
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_session_config")).toHaveLength(initialConfigReads);

    (resolveSaveSessionConfig as (() => void) | null)?.();
  });

  it("keeps tuning controls editable when stale config hydration follows a fast save", async () => {
    const multiConfiguredProviders: ConfiguredProvider[] = [
      configuredProviders[0],
      geminiConfiguredProviders[0],
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages") {
        return [];
      }
      if (command === "get_queued_messages") {
        return [];
      }
      if (command === "get_session_config") {
        return {
          providerId: "provider-1",
          modelId: "model-1",
          reasoningDepth: "off",
          contextBudget: 8192,
        };
      }
      if (command === "save_session_config") {
        return undefined;
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={multiConfiguredProviders}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));

    const providerSelect = within(view.container).getByRole("combobox", { name: "Provider" }) as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: "gemini-provider-1" } });

    await waitFor(() => {
      expect(providerSelect.value).toBe("gemini-provider-1");
    });

    fireEvent.click(within(view.container).getByRole("radio", { name: "High" }));
    await waitFor(() => {
      expect(within(view.container).getByRole("radio", { name: "High" })).toHaveAttribute("aria-checked", "true");
    });

    const slider = within(view.container).getByRole("slider", { name: "Context budget" }) as HTMLInputElement;
    fireEvent.change(slider, { target: { value: "4" } });
    await waitFor(() => {
      expect(slider.value).toBe("4");
    });

    const initialConfigReads = invokeMock.mock.calls.filter(([command]) => command === "get_session_config").length;

    view.rerender(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={multiConfiguredProviders.map((provider) => ({ ...provider }))}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[{ ...sessions[0] }]}
      />,
    );

    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(([command]) => command === "get_session_config").length).toBeGreaterThan(
        initialConfigReads,
      );
    });

    const rerenderedProviderSelect = within(view.container).getByRole("combobox", { name: "Provider" }) as HTMLSelectElement;
    const rerenderedModelSelect = within(view.container).getByRole("combobox", { name: "Model" }) as HTMLSelectElement;
    const rerenderedSlider = within(view.container).getByRole("slider", { name: "Context budget" }) as HTMLInputElement;
    expect(rerenderedProviderSelect.value).toBe("gemini-provider-1");
    expect(rerenderedModelSelect.value).toBe("gemini-3.5-flash");
    expect(rerenderedSlider.value).toBe("4");
    expect(within(view.container).getByRole("radio", { name: "High" })).toHaveAttribute("aria-checked", "true");
  });

  it("toggles Auto-route UI", async () => {
    const updatedSession = {
      ...sessions[0],
      dynamicRoutingOverride: true,
      updatedAtMs: 2,
    };
    const onSessionsChange = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") {
        return [];
      }
      if (command === "update_chat_session_dynamic_routing_override") {
        return {
          session: updatedSession,
          receipt: {
            kind: "auto_route_activation",
            receiptId: "act1",
            dynamicRoutingEnabled: true,
            committed: true,
            rolledBack: false,
            changed: true,
          },
        };
      }
      return null;
    });
    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={configuredProviders.map((provider) => ({ ...provider, providerId: "local_model" }))}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={onSessionsChange}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Auto-route" }));
    await waitFor(() => expect(onSessionsChange).toHaveBeenCalled());
  });
});
