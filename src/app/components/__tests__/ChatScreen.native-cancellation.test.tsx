import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import { cloudAgents, cloudConfiguredProviders, cloudSessions, rejectDeferred } from "./ChatScreen.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const eventListeners = vi.hoisted(() => new Map<string, Set<(event: { payload: unknown }) => void>>());

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1, accepted: true };
    }
    return response;
  },
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    const listeners = eventListeners.get(event) ?? new Set();
    listeners.add(handler);
    eventListeners.set(event, listeners);
    return () => listeners.delete(handler);
  }),
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

function emitToken(payload: Record<string, unknown>) {
  for (const listener of eventListeners.get("chat://token") ?? []) {
    listener({ payload: { delivery_state: "validated", ...payload } });
  }
}

beforeEach(() => {
  invokeMock.mockReset();
  eventListeners.clear();
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

it("replaces a transient reply with a visible result when native inference stops", async () => {
  let pendingRequest: Record<string, string> | null = null;
  let rejectTurn: ((reason?: unknown) => void) | null = null;
  let nativeStopped = false;
  invokeMock.mockImplementation((command: string, args?: { request?: Record<string, string> }) => {
    if (command === "list_chat_messages") {
      if (!nativeStopped || !pendingRequest) return [];
      return [
        {
          id: 1,
          sessionId: "session-1",
          role: "user",
          content: "Prepare the Project report",
          metadataJson: JSON.stringify({
            turnId: pendingRequest.turn_id,
            generationToken: pendingRequest.generation_token,
            turnState: "interrupted",
          }),
          createdAtMs: 1,
        },
      ];
    }
    if (command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "get_local_generation_health") return "ready";
    if (command === "classify_chat_intent_route")
      return {
        route: "conversational_stream",
        requires_local_access: false,
        decision_source: "heuristic_filter",
        confidence: 1,
        reason: "test",
        matched_signals: [],
        status_label: "Thinking...",
      };
    if (command === "chat_turn") {
      pendingRequest = args?.request ?? {};
      return new Promise((_, reject) => {
        rejectTurn = reject;
      });
    }
    if (command === "list_chat_sessions") return cloudSessions;
    return null;
  });

  const view = render(<ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders} onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()} onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />, { wrapper: I18nProvider });
  fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
    target: { value: "Prepare the Project report" },
  });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
  await waitFor(() => expect(pendingRequest).not.toBeNull());
  emitToken({
    stream_id: pendingRequest!.stream_id,
    session_id: "session-1",
    turn_id: pendingRequest!.turn_id,
    generation_token: pendingRequest!.generation_token,
    sequence: 1,
    token: "A draft that must not disappear silently",
    elapsed_ms: 1,
  });
  await screen.findByText("A draft that must not disappear silently");
  nativeStopped = true;
  rejectDeferred(rejectTurn, { code: "local_inference_cancelled" });

  expect(await screen.findByText("This task was cancelled before it finished.")).toBeInTheDocument();
  expect(screen.queryByText("A draft that must not disappear silently")).not.toBeInTheDocument();
  expect(screen.queryByText("OOMU is thinking…")).not.toBeInTheDocument();
  expect(invokeMock.mock.calls.some(([command]) => command === "finalize_accepted_chat_turn")).toBe(false);
});
