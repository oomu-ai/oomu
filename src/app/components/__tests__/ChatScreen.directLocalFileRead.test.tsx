import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import {
  agents,
  approvedFilePreparation,
  configuredProviders,
  sessions,
} from "./ChatScreen.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());
const tauriRuntimeMock = vi.hoisted(() => ({ value: true }));

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1e6, accepted: true };
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
  get isTauriRuntime() {
    return tauriRuntimeMock.value;
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class TestChannel {},
}));

vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => ({
    primaryRoute: null,
    fallbackRoute: null,
    loaded: true,
    setRoutePreference: vi.fn(),
  }),
}));

describe("ChatScreen direct local file reads", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    tauriRuntimeMock.value = true;
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

  it("resolves a named file inside an explicit folder before native approval", async () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const filePath = `${folder}/Lab_Inventory.csv`;
    const prompt = `Read the CSV file Lab_Inventory.csv located in "${folder}" and summarize how many items are listed, along with any items that have low stock.`;
    let chatTurnRequest: { message: string; display_message?: string } | undefined;
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_approved_chat_file") {
        return approvedFilePreparation(
          "Lab_Inventory.csv",
          "item,stock\nPipette tips,2\nGloves,24\n",
          "text/csv",
          42,
        );
      }
      if (command === "chat_turn") {
        chatTurnRequest = (payload as { request: { message: string; display_message?: string } }).request;
        return { text: "Two items are listed. Pipette tips are low.", session_id: "session-1" };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });

    const view = render(
      <ChatScreen activeSessionId="session-1" agents={agents} configuredProviders={configuredProviders} onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()} onSessionsChange={vi.fn()} privacySettings={null} sessions={sessions} />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "chat_turn")).toBe(true);
    });
    const preparation = invokeMock.mock.calls.find(
      ([command]) => command === "prepare_approved_chat_file",
    )?.[1] as { request: { access: { action: { path: string } }; displayMessage: string } };
    expect(preparation.request.access.action.path).toBe(filePath);
    expect(preparation.request.displayMessage).toBe(prompt);
    expect(chatTurnRequest?.display_message).toBe(prompt);
    expect(chatTurnRequest?.message).toContain("[approved file]");
    expect(chatTurnRequest?.message).toContain("[approved folder]");
    expect(chatTurnRequest?.message).not.toContain("Lab_Inventory.csv");
    expect(chatTurnRequest?.message).not.toContain("/Users/example");
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
  });
});
