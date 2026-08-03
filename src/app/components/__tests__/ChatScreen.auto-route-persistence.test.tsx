import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession } from "@/lib/chatSessions";
import { ChatScreen } from "../ChatScreen";
import { agents, configuredProviders, sessions } from "./ChatScreen.fixtures";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
    if (command === "accept_chat_turn" && response == null) {
      return { turnId: args?.request?.turn_id, messageId: 1_000_000, accepted: true };
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
  isTauriRuntime: false,
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

const dynamicSessions: ChatSession[] = [{
  ...sessions[0],
  providerId: "dynamic",
  modelId: "dynamic",
  dynamicRoutingOverride: true,
}];

const dynamicConfig = {
  localProviderConfigId: "provider-1",
  localProviderType: "local",
  modelId: "model-1",
  reasoningDepth: "medium",
  contextBudget: 4096,
  localRouteGeneration: 1,
};

const secondProvider = {
  ...configuredProviders[0],
  id: "provider-2",
  providerId: "openai",
  providerName: "Cloud",
  customModelIds: "model-2",
};

function conversationalRoute() {
  return {
    route: "conversational_stream",
    requires_local_access: false,
    decision_source: "heuristic_filter",
    reason: "test",
    matched_signals: [],
    status_label: "Thinking...",
  };
}

function renderAutoRouteChat() {
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
      sessions={dynamicSessions}
    />,
    { wrapper: I18nProvider },
  );
}

async function verifyNormalSend() {
  let turnRequest: Record<string, unknown> | null = null;
  invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return dynamicConfig;
    if (command === "classify_chat_intent_route") return conversationalRoute();
    if (command === "chat_turn") {
      turnRequest = args?.request ?? null;
      return { text: "Auto-route answer", session_id: "session-1" };
    }
    if (command === "list_chat_sessions") return dynamicSessions;
    return null;
  });
  const view = renderAutoRouteChat();
  const composer = within(view.container).getByPlaceholderText("Message OOMU…");
  fireEvent.change(composer, { target: { value: "Answer with the frozen local route" } });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

  await waitFor(() => expect(turnRequest).not.toBeNull());
  expect(turnRequest).toEqual(expect.objectContaining({
    provider_id: "dynamic",
    model_id: "dynamic",
    dynamic_routing_override: true,
  }));
  expect(invokeMock.mock.calls.some(([command]) => command === "save_session_config")).toBe(false);
}

async function verifyQueuedSend() {
  let resolveTurn: ((response: Record<string, unknown>) => void) | null = null;
  let queuedRequest: Record<string, unknown> | null = null;
  invokeMock.mockImplementation((command: string, args?: { request?: Record<string, unknown> }) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return dynamicConfig;
    if (command === "classify_chat_intent_route") return conversationalRoute();
    if (command === "chat_turn") return new Promise((resolve) => { resolveTurn = resolve; });
    if (command === "queue_message") {
      queuedRequest = args?.request ?? null;
      return { id: 1, sessionId: "session-1", agentId: "agent-1", message: "queued",
        attachments: [], status: "queued", createdAtMs: 1, updatedAtMs: 1 };
    }
    if (command === "list_chat_sessions") return dynamicSessions;
    return null;
  });
  const view = renderAutoRouteChat();
  const composer = within(view.container).getByPlaceholderText("Message OOMU…");
  fireEvent.change(composer, { target: { value: "First Auto-route turn" } });
  fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));
  await waitFor(() => expect(resolveTurn).not.toBeNull());
  const queueComposer = within(view.container).getByPlaceholderText("Message OOMU…");
  fireEvent.change(queueComposer, { target: { value: "Queued Auto-route follow-up" } });
  fireEvent.click(within(view.container).getByRole("button", { name: "Show send options" }));
  fireEvent.click(within(view.container).getByRole("menuitem", { name: "Queue message" }));

  await waitFor(() => expect(queuedRequest).not.toBeNull());
  expect(queuedRequest).toEqual(expect.objectContaining({
    provider_id: "dynamic",
    model_id: "dynamic",
    dynamic_routing_override: true,
  }));
  expect(invokeMock.mock.calls.some(([command]) => command === "save_session_config")).toBe(false);
}

async function verifyTypedManualHydration() {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") {
      return {
        localProviderConfigId: "provider-1",
        localProviderType: "local",
        modelId: "model-1",
        reasoningDepth: "medium",
        contextBudget: 4096,
        localRouteGeneration: 3,
      };
    }
    return null;
  });
  const view = render(
    <ChatScreen
      activeSessionId="session-1"
      agents={[{ ...agents[0], endpoint: { provider: "provider-2", modelId: "model-2" } }]}
      configuredProviders={[configuredProviders[0], secondProvider]}
      onCreateSession={vi.fn()}
      onDeleteSession={vi.fn()}
      onSelectSession={vi.fn()}
      onSessionsChange={vi.fn()}
      privacySettings={null}
      sessions={[{ ...sessions[0], providerId: "provider-2", modelId: "model-2" }]}
    />,
    { wrapper: I18nProvider },
  );

  fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));
  const provider = view.container.querySelector<HTMLSelectElement>(
    '[data-oomu-routing-control="provider"]',
  );
  await waitFor(() => expect(provider?.value).toBe("provider-1"));
}

async function verifyAtomicAutoRouteActivation() {
  const typedLocalProviders = configuredProviders.map((provider) => ({
    ...provider,
    providerId: "local_model",
  }));
  const updatedSession = {
    ...sessions[0],
    dynamicRoutingOverride: true,
    updatedAtMs: 2,
  };
  const onSessionsChange = vi.fn();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return null;
    if (command === "update_chat_session_dynamic_routing_override") {
      return {
        session: updatedSession,
        receipt: {
          kind: "auto_route_activation",
          receiptId: "activation-1",
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
      configuredProviders={typedLocalProviders}
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

  await waitFor(() => expect(invokeMock.mock.calls.filter(([command]) =>
    command === "update_chat_session_dynamic_routing_override")).toHaveLength(1));
  expect(invokeMock.mock.calls.some(([command]) => command === "save_session_config")).toBe(false);
  expect(invokeMock.mock.calls.find(([command]) =>
    command === "update_chat_session_dynamic_routing_override")?.[1]).toMatchObject({
    sessionId: "session-1",
    dynamicRoutingOverride: true,
    autoRouteBaseline: {
      providerConfigId: expect.any(String),
      providerType: "local_model",
      modelId: expect.any(String),
    },
  });
  expect(onSessionsChange).toHaveBeenCalledWith([updatedSession]);
}

async function verifyDisabledAutoRouteBaselineUpdate() {
  const providersWithTwoLocalModels = [{
    ...configuredProviders[0],
    providerId: "local_model",
    customModelIds: "model-1\nmodel-2",
  }];
  const disabledDynamicSession = {
    ...dynamicSessions[0],
    dynamicRoutingOverride: false,
  };
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return dynamicConfig;
    return null;
  });

  const view = render(
    <ChatScreen
      activeSessionId="session-1"
      agents={agents}
      configuredProviders={providersWithTwoLocalModels}
      onCreateSession={vi.fn()}
      onDeleteSession={vi.fn()}
      onSelectSession={vi.fn()}
      onSessionsChange={vi.fn()}
      privacySettings={null}
      sessions={[disabledDynamicSession]}
    />,
    { wrapper: I18nProvider },
  );

  fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));
  const model = await within(view.container).findByRole("combobox", { name: "Model" });
  await waitFor(() => expect((model as HTMLSelectElement).value).toBe("model-1"));
  fireEvent.change(model, { target: { value: "model-2" } });

  await waitFor(() => expect(invokeMock.mock.calls.some(([command, args]) =>
    command === "save_session_config"
      && (args as { modelId?: string } | undefined)?.modelId === "model-2"
  )).toBe(true));
  expect(invokeMock.mock.calls.some(([command]) =>
    command === "update_chat_session_dynamic_routing_override"
  )).toBe(false);
}

async function verifyUiSelectionBecomesAtomicBaseline() {
  const providersWithTwoLocalModels = [{
    ...configuredProviders[0],
    providerId: "local_model",
    providerName: "On-device model",
    customModelIds: "model-1\nmodel-2",
  }];
  let activationPayload: Record<string, unknown> | null = null;
  invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "list_chat_messages" || command === "get_queued_messages") return [];
    if (command === "get_session_config") return dynamicConfig;
    if (command === "update_chat_session_dynamic_routing_override") {
      activationPayload = args ?? null;
      const enabled = Boolean(args?.dynamicRoutingOverride);
      return {
        session: { ...dynamicSessions[0], dynamicRoutingOverride: enabled, updatedAtMs: 2 },
        receipt: {
          kind: "auto_route_activation",
          receiptId: enabled ? "activation-on" : "activation-off",
          dynamicRoutingEnabled: enabled,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      };
    }
    return null;
  });

  function StatefulChat() {
    const [currentSessions, setCurrentSessions] = useState(dynamicSessions);
    return (
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providersWithTwoLocalModels}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={setCurrentSessions}
        privacySettings={null}
        sessions={currentSessions}
      />
    );
  }

  const view = render(<StatefulChat />, { wrapper: I18nProvider });
  expect(view.container.querySelector("#oomu-chat-session-session-1")).not.toBeNull();
  const tuning = within(view.container).getByRole("button", { name: "Tuning" });
  expect(tuning.id).toBe("oomu-chat-tuning");
  fireEvent.click(tuning);
  fireEvent.click(within(view.container).getByRole("button", { name: "Auto-route" }));

  const model = await within(view.container).findByRole("combobox", { name: "Model" });
  await waitFor(() => expect(model).toBeEnabled());
  fireEvent.change(model, { target: { value: "model-2" } });
  await waitFor(() => expect(invokeMock.mock.calls.some(([command, args]) =>
    command === "save_session_config"
      && (args as { modelId?: string } | undefined)?.modelId === "model-2"
  )).toBe(true));

  activationPayload = null;
  fireEvent.click(within(view.container).getByRole("button", { name: "Auto-route" }));
  await waitFor(() => expect(activationPayload).toMatchObject({
    dynamicRoutingOverride: true,
    autoRouteBaseline: {
      providerConfigId: "provider-1",
      providerType: "local_model",
      modelId: "model-2",
    },
  }));
}

describe("ChatScreen Auto-route persistence boundary", () => {
  beforeEach(() => invokeMock.mockReset());
  afterEach(cleanup);

  it("keeps a normal Auto-route send out of legacy config persistence", verifyNormalSend);
  it("keeps a queued Auto-route send out of legacy config persistence", verifyQueuedSend);
  it("hydrates a manual session from typed provider identity, not legacy providerId", verifyTypedManualHydration);
  it("activates Auto-route atomically without legacy config persistence", verifyAtomicAutoRouteActivation);
  it("saves an explicit model while Auto-route is off on a dynamic-bound session", verifyDisabledAutoRouteBaselineUpdate);
  it("uses the model chosen through the real tuning control as the next Auto-route baseline", verifyUiSelectionBecomesAtomicBaseline);
});
