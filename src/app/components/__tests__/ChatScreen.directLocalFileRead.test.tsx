import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatScreen } from "../ChatScreen";
import {
  agents,
  approvedFilePreparation,
  cloudAgents,
  cloudConfiguredProviders,
  cloudSessions,
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

describe("ChatScreen direct local file reads", () => {
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

describe("ChatScreen direct multi-file reads", () => {
  it("ingests every named file before auto-routed strategic analysis", async () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const prompt = `Perform a comprehensive strategic evaluation of the supplier proposals in supplier_proposals.json and cross-reference them with the requirements in q3_strategic_vendor_proposals.txt located in "${folder}". Compare technical compliance, unit pricing, and delivery risks, and provide a multi-scenario vendor trade-off matrix.`;
    const preparedPaths: string[] = [];
    let chatTurnRequest: {
      message: string;
      display_message?: string;
      attachments: Array<{ name: string; approved_file_receipt?: unknown }>;
    } | undefined;
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_approved_chat_file") {
        const path = (payload as { request: { access: { action: { path: string } } } }).request.access.action.path;
        preparedPaths.push(path);
        return path.endsWith(".json")
          ? approvedFilePreparation("supplier_proposals.json", "{\"supplier\":\"A\"}", "application/json", 16)
          : approvedFilePreparation("q3_strategic_vendor_proposals.txt", "Requirement: 30 day delivery", "text/plain", 28);
      }
      if (command === "chat_turn") {
        chatTurnRequest = (payload as { request: typeof chatTurnRequest }).request;
        return { text: "Supplier A meets the requirement.", session_id: "session-1" };
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
    expect(preparedPaths).toEqual([
      `${folder}/supplier_proposals.json`,
      `${folder}/q3_strategic_vendor_proposals.txt`,
    ]);
    expect(chatTurnRequest?.attachments.map((attachment) => attachment.name)).toEqual([
      "supplier_proposals.json",
      "q3_strategic_vendor_proposals.txt",
    ]);
    expect(chatTurnRequest?.attachments.every((attachment) => attachment.approved_file_receipt)).toBe(true);
    expect(chatTurnRequest?.display_message).toBe(prompt);
    expect(chatTurnRequest?.message).not.toContain("/Users/example");
    expect(chatTurnRequest?.message).not.toContain("supplier_proposals.json");
    expect(chatTurnRequest?.message).not.toContain("q3_strategic_vendor_proposals.txt");
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
  });

});

describe("ChatScreen direct file consent resume", () => {
  it("reuses one approved batch when cloud consent resumes the same turn", async () => {
    const folder = "/Users/example/Documents/OOMU/Projects/mock_data";
    const prompt = `Compare supplier_proposals.json with q3_strategic_vendor_proposals.txt located in "${folder}" and explain the trade-offs.`;
    const preparationRequests: Array<Record<string, unknown>> = [];
    const chatTurnRequests: Array<Record<string, unknown>> = [];
    let receiptSequence = 0;
    invokeMock.mockImplementation(async (command: string, payload?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      if (command === "get_local_generation_health") return "ready";
      if (command === "prepare_approved_chat_file") {
        preparationRequests.push(payload?.request ?? {});
        const path = ((payload?.request?.access as { action?: { path?: string } })?.action?.path ?? "");
        const name = path.split("/").at(-1) ?? "approved.txt";
        const prepared = approvedFilePreparation(name, `Verified context for ${name}`);
        receiptSequence += 1;
        prepared.receipt.payload = `signed-approved-file-payload-${receiptSequence}`;
        prepared.receipt.signature.signature = `test-receipt-signature-${receiptSequence}`;
        return prepared;
      }
      if (command === "chat_turn") {
        chatTurnRequests.push(payload?.request ?? {});
        if (chatTurnRequests.length === 1) {
          throw { code: "private_egress_confirmation_required" };
        }
        return { text: "Supplier A is cheaper; Supplier B has lower delivery risk.", session_id: "session-1" };
      }
      if (command === "get_private_egress_confirmation") return {
        challengeId: "challenge-file-batch",
        destinationProviderId: "openai",
        destinationModelId: "gpt-5.5",
        sourceNames: ["supplier_proposals.json", "q3_strategic_vendor_proposals.txt"],
      };
      if (command === "resolve_private_egress_confirmation") return { decision: "approved" };
      if (command === "list_chat_sessions") return cloudSessions;
      return null;
    });

    const view = render(
      <ChatScreen activeSessionId="session-1" agents={cloudAgents} configuredProviders={cloudConfiguredProviders} onCreateSession={vi.fn()} onDeleteSession={vi.fn()} onSelectSession={vi.fn()} onSessionsChange={vi.fn()} privacySettings={null} sessions={cloudSessions} />,
      { wrapper: I18nProvider },
    );
    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: prompt },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    const consent = await within(view.container).findByRole("alert");
    expect(consent).toHaveTextContent("supplier_proposals.json");
    expect(consent).toHaveTextContent("q3_strategic_vendor_proposals.txt");
    fireEvent.click(screen.getByRole("button", { name: "Send once" }));

    await waitFor(() => expect(chatTurnRequests).toHaveLength(2));
    expect(preparationRequests).toHaveLength(2);
    const approvalTurnIds = preparationRequests.map((request) =>
      (request.access as { turnId?: string })?.turnId
    );
    expect(approvalTurnIds.every(Boolean)).toBe(true);
    expect(new Set(approvalTurnIds).size).toBe(1);
    expect(approvalTurnIds[0]).toBe(chatTurnRequests[0]?.turn_id);
    expect(preparationRequests.map((request) =>
      (request.access as { action?: { path?: string } })?.action?.path
    )).toEqual([
      `${folder}/supplier_proposals.json`,
      `${folder}/q3_strategic_vendor_proposals.txt`,
    ]);
    expect(preparationRequests.map((request) => request.displayMessage)).toEqual([prompt, prompt]);
    expect(chatTurnRequests[1]).toEqual(expect.objectContaining({
      session_id: chatTurnRequests[0]?.session_id,
      turn_id: chatTurnRequests[0]?.turn_id,
      generation_token: chatTurnRequests[0]?.generation_token,
      attachments: chatTurnRequests[0]?.attachments,
    }));
    const resumedAttachments = chatTurnRequests[1]?.attachments as Array<{
      approved_file_receipt?: { payload?: string };
    }>;
    expect(resumedAttachments.map((attachment) => attachment.approved_file_receipt?.payload)).toEqual([
      "signed-approved-file-payload-1",
      "signed-approved-file-payload-2",
    ]);
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_private_egress_confirmation")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "resolve_private_egress_confirmation")).toHaveLength(1);
  });
});
