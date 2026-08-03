import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatScreen, type ChatAgent } from "../ChatScreen";
import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession } from "@/lib/chatSessions";
import { agents, configuredProviders, sessions } from "./ChatScreen.fixtures";
import { createPlanPersistenceMock } from "./ChatScreen.plan-test-runtime";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: async (command: string, args?: { request?: Record<string, unknown> }) => {
    const response = await invokeMock(command, args);
    if (command === "triage_local_app_intent" && response == null) return true;
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
  isTauriRuntime: false,
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
});

afterEach(() => {
  cleanup();
});

const e2bModelId = "gemma-4-E2B-it-qat-q4_0-gguf";
const providersWithGeminiFirst = [
  {
    id: "gemini-provider", providerId: "google", providerName: "Gemini",
    authMethod: "api_key" as const, baseUrl: "", apiKeyLabel: "GEMINI_API_KEY",
    customModelIds: "gemini-3.6-flash", credentialConfigured: true,
  },
  {
    id: "local-e2b-provider", providerId: "local_model", providerName: "On-device",
    authMethod: "custom" as const, baseUrl: "", apiKeyLabel: "",
    customModelIds: e2bModelId, credentialConfigured: true,
  },
];

describe("trusted read-only execution", () => {
  it("runs a signed trusted read-only plan without a second approval prompt", async () => {
    const planPersistence = createPlanPersistenceMock("session-1");
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_chat_messages") return planPersistence.listMessages();
      if (command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
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
        return {
          text: "Pivoting to Agentic Planner.",
          session_id: "session-1",
          route_escalation: {
            route: "agentic_planner",
            requires_local_access: true,
            decision_source: "read_only_project_status_filter",
            reason: "Read-only project status requested.",
            matched_signals: ["read-only project status request"],
            status_label: "Planning...",
          },
        };
      }
      if (command === "process_agent_objective") {
        return {
          id: "read-only-plan",
          objective: "Inspect the current project without modifying it.",
          steps: [{
            step: "Check the working tree.",
            tool: { kind: "terminal_execute", executable: "/usr/bin/git" },
            risk_level: "low",
          }],
          exit_condition: "Report the verified status.",
          trusted_automatic_execution: true,
          model_route: {
            reason: "A deterministic read-only command is ready.",
            requires_principal_authorization: false,
          },
        };
      }
      if (command === "record_browser_chat_turn") return planPersistence.record(args);
      if (command === "request_agent_plan_authority") {
        return { authorityProofId: null, expiresAtMs: null };
      }
      if (command === "spawn_agent_execution") {
        return {
          executionId: "execution-read-only",
          planId: "read-only-plan",
          sessionId: "session-1",
          streamStartAfterLogId: 0,
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
      target: { value: "Inspect the current project without modifying it." },
    });
    fireEvent.click(within(view.container).getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([command]) => command === "spawn_agent_execution"))
        .toBe(true);
    });
    const authorityCall = invokeMock.mock.calls.find(
      ([command]) => command === "request_agent_plan_authority",
    );
    expect(authorityCall?.[1]).toMatchObject({
      request: { request: { principal_approved: false } },
    });
    expect(screen.queryByRole("button", { name: "Approve & execute" }))
      .not.toBeInTheDocument();
  });
});

describe("verified startup model provider binding", () => {
  it("binds a local endpoint to its exact provider configuration even when Gemini is first", async () => {
    const verifiedLocalAgent: ChatAgent[] = [{
      ...agents[0],
      endpoint: { provider: "local_model", modelId: e2bModelId },
    }];
    const onCreateSession = vi.fn(async (
      _agentId: string,
      route: { providerId: string; modelId: string },
    ) => ({
      ...sessions[0],
      id: "session-verified-startup",
      providerId: route.providerId,
      modelId: route.modelId,
    }));
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={verifiedLocalAgent}
        configuredProviders={providersWithGeminiFirst}
        onCreateSession={onCreateSession}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
        verifiedStartupModelId={e2bModelId}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(within(view.container).getByRole("button", { name: "Tuning" }));
    const provider = view.container.querySelector<HTMLSelectElement>(
      '[data-oomu-routing-control="provider"]',
    );
    const model = view.container.querySelector<HTMLSelectElement>(
      '[data-oomu-routing-control="model"]',
    );
    await waitFor(() => expect(provider?.value).toBe("local-e2b-provider"));
    expect(provider?.selectedOptions[0]?.textContent).toBe("On-device");
    expect(model?.value).toBe(e2bModelId);

    fireEvent.click(within(view.container).getByRole("button", { name: "New chat" }));

    await waitFor(() => {
      expect(onCreateSession).toHaveBeenCalledWith("agent-1", {
        providerId: "local-e2b-provider",
        modelId: e2bModelId,
      });
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_session_config",
        expect.objectContaining({
          session_id: "session-verified-startup",
          provider_id: "local-e2b-provider",
          model_id: e2bModelId,
        }),
      );
    });
  });
});

describe("verified startup model empty-session recovery", () => {
  it("recovers an existing empty local session when the startup model becomes verified", async () => {
    const emptyRouteSession: ChatSession = {
      ...sessions[0],
      providerId: "",
      modelId: "",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_chat_messages" || command === "get_queued_messages") return [];
      if (command === "get_session_config") return null;
      return null;
    });

    const view = render(
      <ChatScreen
        activeSessionId="session-1"
        agents={[{
          ...agents[0],
          endpoint: { provider: "", modelId: "" },
        }]}
        configuredProviders={providersWithGeminiFirst}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={[emptyRouteSession]}
        verifiedStartupModelId={e2bModelId}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(within(view.container).getByPlaceholderText("Message OOMU…"), {
      target: { value: "Inspect the current project." },
    });
    await waitFor(() => {
      expect(view.container.querySelector("#oomu-chat-send")).toBeEnabled();
    });
  });
});
