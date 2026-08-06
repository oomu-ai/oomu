import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ChatSession } from "@/lib/chatSessions";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { ChatScreen, type ChatAgent } from "../ChatScreen";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/hooks/useMcp", () => ({ useOptionalMcp: () => null }));
vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
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

const agents: ChatAgent[] = [{
  id: "agent-1",
  name: "OOMU",
  description: "Test agent",
  endpoint: { provider: "provider-1", modelId: "model-1" },
}];
const providers: ConfiguredProvider[] = [{
  id: "provider-1",
  providerId: "local",
  providerName: "Local",
  authMethod: "api_key",
  baseUrl: "",
  apiKeyLabel: "",
  customModelIds: "model-1",
}];
const sessions: ChatSession[] = [{
  id: "session-1",
  agentId: "agent-1",
  title: "Scenario 1 Test 6",
  providerId: "provider-1",
  modelId: "model-1",
  createdAtMs: 1,
  updatedAtMs: 1,
}];
const persistedRecoveryUserMessage = {
  id: 5,
  sessionId: "session-1",
  role: "user",
  content: "Continue the saved work.",
  providerId: "local_model",
  modelId: "gemma-test",
  metadataJson: JSON.stringify({
    turnId: "turn-recovery-1",
    rootTurnId: "turn-recovery-1",
    generationToken: "generation-recovery-1",
    turnState: "escalated",
  }),
  createdAtMs: 5,
};

function recoveryOwnership(sessionId: string) {
  return {
    sessionId,
    rootTurnId: "turn-recovery-1",
    failedTurnId: "turn-recovery-1",
    generationToken: "generation-recovery-1",
  };
}
const recoveryContent = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "agent-exec-scenario-1-test-6",
  planId: "plan-scenario-1-test-6",
  code: "decision_pack_research_evidence_unavailable",
  boundary: "DecisionPack",
  recoverable: true,
  message: "No current official freight source qualified.",
  context: { subject: "freight", attemptCount: 3, pageCount: 7, verifiedInputCount: 2 },
  changedState: "none",
});
const mailAutomationRecoveryContent = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "agent-exec-scenario-1-test-6",
  planId: "plan-scenario-1-test-6",
  code: "mail_automation_permission_required",
  boundary: "Mail",
  recoverable: true,
  recoveryAction: "resume_same_execution",
  message: "macOS has not allowed OOMU to control Mail. No Mail draft was created.",
  context: { changedState: false },
  changedState: "checkpoint_saved",
});
const calendarRecoveryContent = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "agent-exec-scenario-2",
  planId: "plan-scenario-2",
  code: "calendar_not_found",
  boundary: "Calendar",
  recoverable: true,
  recoveryAction: "resolve_calendar_target",
  message: "The requested calendar was not found.",
  context: {
    requestedCalendarName: "OOMU Test Denial",
    availableCalendarNames: ["Family", "OOMU Test"],
  },
  changedState: "checkpoint_saved",
});
const durableObjective = "Prepare the supplier decision pack from the verified source files.";
const startNewPlanRecoveryContent = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "agent-exec-scenario-1-test-7",
  planId: "plan-scenario-1-test-7",
  code: "preflight_verification_failed",
  boundary: "DecisionPack",
  recoverable: false,
  recoveryAction: "start_new_plan",
  message: "The prior execution cannot safely resume.",
  context: { subject: "freight", verifiedInputCount: 2 },
  changedState: "none",
});
const reviewRecoveryContent = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "agent-exec-external-changes",
  planId: "plan-external-changes",
  code: "agent_execution_failed",
  boundary: "DecisionPack",
  recoverable: false,
  recoveryAction: "review_external_changes",
  message: "Review external changes before continuing.",
  context: { subject: "freight", verifiedInputCount: 2 },
  changedState: "external_changes",
});

describe("ChatScreen recovery receipt integration", () => {
  let displayedRecoveryContent = recoveryContent;
  let acceptedObjective: string | null = null;
  let activeLocale = "en-US";
  let recordedPlanText: string | null = null;
  let calendarResolutionError: { code: string; message: string } | null = null;
  let finalizedTerminal: { role: string; content: string } | null = null;
  beforeEach(() => {
    displayedRecoveryContent = recoveryContent;
    acceptedObjective = null;
    activeLocale = "en-US";
    recordedPlanText = null;
    calendarResolutionError = null;
    finalizedTerminal = null;
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
    invokeMock.mockImplementation(async (command: string, args?: {
      request?: Record<string, unknown>;
      executionIds?: string[];
      sessionId?: string;
    }) => {
      if (command === "get_locale_state") return {
        activeLocale,
        availableLocales: [
          { id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true },
          ...(activeLocale === "en-US" ? [] : [{ id: activeLocale, label: "Español", fileName: "es-ES.json", isDefault: false, verified: true }]),
        ],
        translations: {},
      };
      if (command === "list_chat_messages") return [
        persistedRecoveryUserMessage,
        {
          id: 6,
          sessionId: "session-1",
          role: "assistant",
          content: displayedRecoveryContent,
          createdAtMs: 6,
        },
        ...(acceptedObjective ? [{
          id: 7,
          sessionId: "session-1",
          role: "user",
          content: acceptedObjective,
          createdAtMs: 7,
        }] : []),
        ...(recordedPlanText ? [{
          id: 8,
          sessionId: "session-1",
          role: "assistant",
          content: recordedPlanText,
          createdAtMs: 8,
        }] : []),
        ...(finalizedTerminal ? [{
          id: 9,
          sessionId: "session-1",
          role: finalizedTerminal.role,
          content: finalizedTerminal.content,
          createdAtMs: 9,
        }] : []),
      ];
      if (command === "get_queued_messages" || command === "list_installed_mods") return [];
      if (command === "get_agent_execution_recovery_states") {
        const receipt = JSON.parse(displayedRecoveryContent) as {
          executionId: string;
          planId: string;
          code: string;
          context?: { frozenArgumentSha256?: string; nextOperation?: string };
        };
        const interruptedMail = receipt.code === "agent_execution_interrupted"
          && receipt.context?.nextOperation === "draft_release_recovery_email"
          && /^[a-f0-9]{64}$/.test(receipt.context?.frozenArgumentSha256 ?? "");
        return (args?.executionIds ?? []).flatMap((executionId) =>
          executionId === receipt.executionId ? [{
            executionId,
            planId: receipt.planId,
            ...recoveryOwnership(args?.sessionId ?? ""),
            status: "halted",
            terminalPhase: interruptedMail ? "restart_recovery_ready" : "halted",
            terminalVerified: false,
            verifiedComplete: false,
          }] : []
        );
      }
      if (command === "get_local_model_status") return "ready";
      if (command === "get_session_config") return null;
      if (command === "choose_local_context") return {
        results: [{
          name: "private-draft-notes.txt",
          ok: true,
          grantId: "a".repeat(64),
          mimeType: "text/plain",
          decodedByteCount: 20,
          encodedByteCount: 0,
          expiresAtMs: Date.now() + 60_000,
          errorCode: null,
        }],
        countLimit: 5,
        decodedByteLimit: 20 * 1024 * 1024,
        encodedByteLimit: 28 * 1024 * 1024,
      };
      if (command === "read_local_context") return {
        name: "private-draft-notes.txt",
        mime_type: "text/plain",
        byte_count: 20,
        text: "unrelated private note",
        truncated: false,
      };
      if (command === "revoke_local_context_grants") return { revokedCount: 0 };
      if (command === "check_mail_automation_access") return {
        status: "authorized",
        authorized: true,
        retrySupported: true,
      };
      if (command === "open_mail_automation_settings") return undefined;
      if (command === "resolve_agent_calendar_recovery") {
        if (calendarResolutionError) throw calendarResolutionError;
        const executionId = String(args?.request?.executionId ?? "");
        const calendarName = String(args?.request?.calendarName ?? "");
        displayedRecoveryContent = JSON.stringify({
          ...JSON.parse(calendarRecoveryContent),
          executionId,
          code: "calendar_target_resolved",
          recoveryAction: "resume_same_execution",
          context: {
            requestedCalendarName: calendarName,
            availableCalendarNames: ["Family", "OOMU Test"],
          },
        });
        return { status: "ready_to_resume", selectedCalendarName: calendarName };
      }
      if (command === "resume_agent_execution") return {
        executionId: String(args?.request?.executionId ?? "agent-exec-scenario-1-test-6"),
        planId: String(args?.request?.executionId ?? "plan-scenario-1-test-6") === "agent-exec-scenario-2"
          ? "plan-scenario-2"
          : "plan-scenario-1-test-6",
        sessionId: "session-1",
        streamStartAfterLogId: 41,
      };
      if (command === "prepare_agent_execution_replan") return { objective: durableObjective };
      if (command === "accept_chat_turn") {
        acceptedObjective = String(args?.request?.message ?? "");
        return {
          turnId: args?.request?.turn_id,
          messageId: 7,
          accepted: true,
        };
      }
      if (command === "finalize_accepted_chat_turn") {
        finalizedTerminal = {
          role: String(args?.request?.role ?? "system"),
          content: String(args?.request?.content ?? ""),
        };
        return 9;
      }
      if (command === "classify_chat_intent_route") return {
        route: "agentic_planner",
        requires_local_access: true,
        decision_source: "deterministic_action_rules",
        confidence: 1,
        reason: "The durable objective requires a fresh plan.",
        matched_signals: ["durable recovery objective"],
        status_label: "Planning…",
      };
      if (command === "process_agent_objective") return {
        id: "fresh-plan-test-7",
        objective: durableObjective,
        steps: [{
          step: "Reassess the decision-pack work before any action.",
          tool: { kind: "unsupported", requested: "replan" },
          risk_level: "medium",
        }],
        exit_condition: "Present a fresh approval-gated plan.",
        trusted_automatic_execution: false,
        model_route: { reason: "Fresh plan required.", requires_principal_authorization: true },
      };
      if (command === "record_browser_chat_turn") {
        recordedPlanText = String(args?.request?.assistant_text ?? "");
        return {
          text: recordedPlanText,
          session_id: "session-1",
          turn_id: args?.request?.turn_id,
          generation_token: args?.request?.generation_token,
        };
      }
      if (command === "list_chat_sessions") return sessions;
      return null;
    });
  });
  afterEach(cleanup);

  it("resumes the same halted execution and never presents it as complete", async () => {
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    const retry = await screen.findByRole("button", { name: "Retry research" });
    expect(screen.getByText("freight research needs another pass")).toBeVisible();
    expect(screen.queryByText(/completed|complete/i)).toBeNull();
    fireEvent.click(retry);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resume_agent_execution",
      { request: { executionId: "agent-exec-scenario-1-test-6" } },
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "stream_execution_steps",
      expect.objectContaining({
        executionId: "agent-exec-scenario-1-test-6",
        lastSeenId: 41,
        last_seen_id: 41,
      }),
    ));
    expect(window.localStorage.setItem).toHaveBeenCalledWith(
      "oomu.chat.activeAgentExecution:session-1",
      expect.stringContaining('"lastSeenId":41'),
    );
  });

  it("opens native Mail Automation Settings, checks access, and resumes the exact checkpoint", async () => {
    displayedRecoveryContent = mailAutomationRecoveryContent;
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(await screen.findByRole("button", { name: "Open Automation Settings" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "open_mail_automation_settings",
      undefined,
    ));

    fireEvent.click(screen.getByRole("button", { name: "Check access and continue" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "check_mail_automation_access",
      undefined,
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resume_agent_execution",
      { request: { executionId: "agent-exec-scenario-1-test-6" } },
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "stream_execution_steps",
      expect.objectContaining({
        executionId: "agent-exec-scenario-1-test-6",
        lastSeenId: 41,
      }),
    ));
  });

  it("uses an exact same-session calendar correction to amend and resume the stopped execution", async () => {
    displayedRecoveryContent = calendarRecoveryContent;
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    expect((await screen.findAllByText(/OOMU Test Denial/)).length).toBeGreaterThan(0);
    const composer = screen.getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, {
      target: { value: "Use my OOMU Test calendar instead and continue." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resolve_agent_calendar_recovery",
      { request: {
        executionId: "agent-exec-scenario-2",
        sessionId: "session-1",
        resolution: "select_existing",
        calendarName: "OOMU Test",
      } },
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resume_agent_execution",
      { request: { executionId: "agent-exec-scenario-2" } },
    ));
    expect(invokeMock.mock.calls.some(([command]) =>
      command === "classify_chat_intent_route" || command === "process_agent_objective"
    )).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith(
      "finalize_accepted_chat_turn",
      { request: expect.objectContaining({
        session_id: "session-1",
        role: "assistant",
        status: "completed",
        content: "The calendar is set. OOMU is continuing from the paused step.",
      }) },
    );
  });

  it("explains an incompatible selected calendar and leaves the durable choice recoverable", async () => {
    displayedRecoveryContent = calendarRecoveryContent;
    calendarResolutionError = {
      code: "calendar_availability_unsupported",
      message: "The selected calendar cannot represent the event availability required by this task.",
    };
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    const composer = await screen.findByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, {
      target: { value: "Use my OOMU Test calendar instead and continue." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByText(
      "The calendar “OOMU Test” can’t mark events as tentative. Choose a compatible calendar.",
    )).toBeVisible();
    await waitFor(() => expect(
      screen.getByRole("region", { name: "Choose a calendar" }),
    ).toBeVisible());
    expect(invokeMock).not.toHaveBeenCalledWith(
      "resume_agent_execution",
      expect.anything(),
    );
    expect(invokeMock).toHaveBeenCalledWith(
      "finalize_accepted_chat_turn",
      { request: expect.objectContaining({
        session_id: "session-1",
        role: "system",
        status: "failed",
        content: "The calendar “OOMU Test” can’t mark events as tentative. Choose a compatible calendar.",
      }) },
    );
  });

  it("starts a fresh approval-gated plan from the session-owned durable objective", async () => {
    displayedRecoveryContent = startNewPlanRecoveryContent;
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    const composer = await screen.findByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "Keep this private draft untouched." } });
    fireEvent.click(screen.getByRole("button", { name: "Attach file" }));
    expect(await screen.findByText("private-draft-notes.txt")).toBeVisible();
    fireEvent.click(await screen.findByRole("button", { name: "Start a new plan" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "prepare_agent_execution_replan",
      { request: {
        executionId: "agent-exec-scenario-1-test-7",
        sessionId: "session-1",
      } },
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "accept_chat_turn",
      { request: expect.objectContaining({
        session_id: "session-1",
        message: durableObjective,
      }) },
    ));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "process_agent_objective",
      { request: expect.objectContaining({
        session_id: "session-1",
        user_objective: durableObjective,
        prompt: durableObjective,
      }) },
    ));
    expect(await screen.findByRole("button", { name: "Approve & execute" })).toBeVisible();
    expect(composer).toHaveValue("Keep this private draft untouched.");
    expect(screen.getByText("private-draft-notes.txt")).toBeVisible();
    expect(screen.getByText(/new plan is ready/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Start a new plan" })).toBeNull();
    expect(invokeMock.mock.calls.some(([command]) => command === "classify_chat_intent_route")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "resume_agent_execution")).toBe(false);
  });

  it("turns external-change review into a fresh approval-gated plan", async () => {
    displayedRecoveryContent = reviewRecoveryContent;
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    expect(await screen.findByText(/won’t replay an uncertain step/i)).toBeVisible();
    expect(screen.getByText(/nothing will run until you review and approve/i)).toBeVisible();
    expect(screen.getByText("Decision pack research")).toBeVisible();
    expect(screen.getByText("Review external changes before continuing.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Review and continue" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "prepare_agent_execution_replan",
      { request: {
        executionId: "agent-exec-external-changes",
        sessionId: "session-1",
      } },
    ));
    expect(await screen.findByRole("button", { name: "Approve & execute" })).toBeVisible();
    expect(screen.getByText(/new plan is ready/i)).toBeVisible();
    expect(invokeMock.mock.calls.some(([command]) => command === "resume_agent_execution"))
      .toBe(false);
  });

  it("persists the recovered plan summary through English fallback", async () => {
    activeLocale = "es-ES";
    displayedRecoveryContent = startNewPlanRecoveryContent;
    render(
      <ChatScreen
        activeSessionId="session-1"
        agents={agents}
        configuredProviders={providers}
        onCreateSession={vi.fn()}
        onDeleteSession={vi.fn()}
        onSelectSession={vi.fn()}
        onSessionsChange={vi.fn()}
        privacySettings={null}
        sessions={sessions}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(await screen.findByRole("button", { name: "Start a new plan" }));
    await waitFor(() => expect(recordedPlanText).toContain("Action plan compiled."));
    expect(recordedPlanText).toContain("steps awaiting approval");
  });
});
