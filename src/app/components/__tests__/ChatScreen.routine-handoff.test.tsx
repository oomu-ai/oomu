import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import { agents, configuredProviders, sessions } from "./ChatScreen.execution-boundaries.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/hooks/useMcp", () => ({ useOptionalMcp: () => null }));
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
afterEach(cleanup);

describe("ChatScreen one-time Routine handoff", () => {
  it("opens confirmation with the exact request before touching the file", async () => {
    const prompt =
      "At 4:35 PM today, check whether /Users/example/report.md still exists and tell me in this task. Do not change the file.";
    const onOpenRoutine = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner",
          requires_local_access: false,
          decision_source: "routine_scheduler_filter",
          matched_signals: ["future one-time routine"],
        };
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
        onOpenRoutine={onOpenRoutine}
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

    await waitFor(() => expect(onOpenRoutine).toHaveBeenCalledWith(expect.objectContaining({
      requestText: prompt,
      scheduleText: expect.stringMatching(/^on \d{4}-\d{2}-\d{2} at 4:35 PM$/),
      scheduleKind: "one_shot",
    })));
    expect(view.container).toHaveTextContent(
      "OOMU opened the schedule for review. Nothing will run until you confirm it.",
    );
    expect(
      invokeMock.mock.calls.some(([command]) =>
        ["choose_local_context", "read_local_context", "request_shield_approval"].includes(command),
      ),
    ).toBe(false);
  });
});

describe("ChatScreen recurring Routine handoff", () => {
  it("preserves the native Mail target for a project-bound executable workflow", async () => {
    const prompt = "Check my unread email every hour until midnight. Once you set it up, run it once.";
    const onOpenRoutine = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "classify_chat_intent_route") {
        return {
          route: "agentic_planner", requires_local_access: true,
          decision_source: "routine_scheduler_filter",
          matched_signals: [
            "recurring routine", "routine cadence:v1:1:hour",
            "routine schedule seed: every 1 hour",
            "routine target private app:v1:mail",
            "explicit run once requested", "end at midnight requested",
          ],
        };
      }
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1" agents={agents}
        configuredProviders={configuredProviders} onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()} onOpenRoutine={onOpenRoutine}
        onSelectSession={vi.fn()} onSessionsChange={vi.fn()}
        privacySettings={null} projectId="project-1" sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => expect(onOpenRoutine).toHaveBeenCalledWith(expect.objectContaining({
      requestText: prompt,
      targetAction: { kind: "read_unread_mail" },
    })));
    expect(invokeMock.mock.calls.some(([command]) => command === "mcp_execute_tool")).toBe(false);
  });
});
