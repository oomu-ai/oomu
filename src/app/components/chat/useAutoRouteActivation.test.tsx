import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@/lib/invoke";

import type { ChatSession } from "@/lib/chatSessions";
import {
  canonicalModelId,
  providerConfigurationId,
  providerTypeId,
} from "@/lib/modelRegistry";

import { useAutoRouteActivation } from "./useAutoRouteActivation";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const session = (id: string, updatedAtMs = 1): ChatSession => ({
  id,
  agentId: "agent-302",
  title: id,
  providerId: "dynamic",
  modelId: "dynamic",
  dynamicRoutingOverride: true,
  createdAtMs: 1,
  updatedAtMs,
});

async function coalesceTuningChange() {
    let resolveFirst: (value: unknown) => void = () => undefined;
    vi.mocked(invoke)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValueOnce({
        session: session("session-302", 3),
        receipt: {
          kind: "auto_route_activation",
          receiptId: "receipt-2",
          dynamicRoutingEnabled: true,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      });
    const onSessionsChange = vi.fn();
    let sessions = [session("session-302")];
    let modelId = "gemma-4-E2B-it-qat-q4_0-gguf";
    const options = () => ({
      activeSessionId: "session-302",
      buildBaseline: () => ({
        providerConfigId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: canonicalModelId(modelId),
        reasoningDepth: "medium",
        contextBudget: 12_288,
      }),
      canActivate: true,
      dynamicRoutingEnabled: true,
      ensureSession: vi.fn(async () => ({ sessionId: "session-302", hydrationLockToken: null })),
      getRoute: () => ({
        providerId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId,
        reasoning: "medium" as const,
        context: "12288",
      }),
      onSessionsChange,
      sessions,
      setStatus: vi.fn(),
      statusBlocked: "blocked",
      statusDisabled: "disabled",
      statusEnabled: "enabled",
      unlockSession: vi.fn(),
    });
    const hook = renderHook(() => useAutoRouteActivation(options()));

    act(() => void hook.result.current.toggle(true));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    modelId = "gemma-4-E4B-it-qat-q4_0-gguf";
    act(() => void hook.result.current.toggle(true));
    sessions = [session("session-302"), session("session-latest")];
    hook.rerender();
    await act(async () => resolveFirst({
        session: session("session-302", 2),
        receipt: {
          kind: "auto_route_activation",
          receiptId: "receipt-1",
          dynamicRoutingEnabled: true,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      }));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(onSessionsChange.mock.calls[0][0].map((entry: ChatSession) => entry.id))
      .toEqual(["session-302", "session-latest"]);
    await waitFor(() => expect(hook.result.current.isSaving).toBe(false));
    expect(vi.mocked(invoke).mock.calls[1][1]).toMatchObject({
      autoRouteBaseline: { modelId: "gemma-4-E4B-it-qat-q4_0-gguf" },
    });
}

async function replayRapidToggle() {
    let resolveSession: (value: { sessionId: string; hydrationLockToken: number | null }) => void =
      () => undefined;
    const ensureSession = vi
      .fn()
      .mockImplementationOnce(() => new Promise((resolve) => { resolveSession = resolve; }))
      .mockResolvedValue({ sessionId: "session-created", hydrationLockToken: null });
    vi.mocked(invoke)
      .mockResolvedValueOnce({
        session: {
          ...session("session-created"),
          dynamicRoutingOverride: true,
        },
        receipt: {
          kind: "auto_route_activation",
          receiptId: "receipt-on",
          dynamicRoutingEnabled: true,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      })
      .mockResolvedValueOnce({
        session: { ...session("session-created"), dynamicRoutingOverride: false },
        receipt: {
          kind: "auto_route_activation",
          receiptId: "receipt-off",
          dynamicRoutingEnabled: false,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      });
    const hook = renderHook(() => useAutoRouteActivation({
      activeSessionId: "",
      buildBaseline: () => ({
        providerConfigId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: canonicalModelId("gemma-4-E2B-it-qat-q4_0-gguf"),
        reasoningDepth: "medium",
        contextBudget: 12_288,
      }),
      canActivate: true,
      dynamicRoutingEnabled: false,
      ensureSession,
      getRoute: () => ({
        providerId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        reasoning: "medium",
        context: "12288",
      }),
      onSessionsChange: vi.fn(),
      sessions: [],
      setStatus: vi.fn(),
      statusBlocked: "blocked",
      statusDisabled: "disabled",
      statusEnabled: "enabled",
      unlockSession: vi.fn(),
    }));

    act(() => void hook.result.current.toggle(true));
    await waitFor(() => expect(ensureSession).toHaveBeenCalledTimes(1));
    act(() => void hook.result.current.toggle(false));
    await act(async () => resolveSession({
      sessionId: "session-created",
      hydrationLockToken: null,
    }));

    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(ensureSession).toHaveBeenNthCalledWith(2, "session-created");
    expect(vi.mocked(invoke).mock.calls.map(([, payload]) =>
      (payload as { sessionId: string }).sessionId
    )).toEqual(["session-created", "session-created"]);
    expect(vi.mocked(invoke).mock.calls[1][1]).toMatchObject({
      dynamicRoutingOverride: false,
    });
}

async function clearStaleFailureOnSessionSwitch() {
    let activeSessionId = "session-a";
    vi.mocked(invoke).mockRejectedValueOnce(
      new Error("auto_route_activation_worker_failed"),
    );
    const hook = renderHook(() => useAutoRouteActivation({
      activeSessionId,
      buildBaseline: () => ({
        providerConfigId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: canonicalModelId("gemma-4-E2B-it-qat-q4_0-gguf"),
        reasoningDepth: "medium",
        contextBudget: 12_288,
      }),
      canActivate: true,
      dynamicRoutingEnabled: false,
      ensureSession: vi.fn(async (preferredSessionId) => ({
        sessionId: preferredSessionId ?? "",
        hydrationLockToken: null,
      })),
      getRoute: () => ({
        providerId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        reasoning: "medium",
        context: "12288",
      }),
      onSessionsChange: vi.fn(),
      sessions: [session("session-a"), session("session-b")],
      setStatus: vi.fn(),
      statusBlocked: "blocked",
      statusDisabled: "disabled",
      statusEnabled: "enabled",
      unlockSession: vi.fn(),
    }));

    await act(async () => hook.result.current.toggle(true));
    expect(hook.result.current.failure).toMatchObject({
      sessionId: "session-a",
      code: "auto_route_activation_worker_failed",
      desiredEnabled: true,
    });
    activeSessionId = "session-b";
    hook.rerender();
    expect(hook.result.current.failure).toBeNull();
    activeSessionId = "session-a";
    hook.rerender();
    expect(hook.result.current.failure).toBeNull();
}

async function clearStalePendingActivationOnSessionSwitch() {
    let activeSessionId = "session-a";
    let resolveFirst: (value: unknown) => void = () => undefined;
    vi.mocked(invoke).mockImplementationOnce(() =>
      new Promise((resolve) => { resolveFirst = resolve; }));
    const ensureSession = vi.fn(async (preferredSessionId?: string | null) => ({
      sessionId: preferredSessionId ?? "",
      hydrationLockToken: null,
    }));
    const hook = renderHook(() => useAutoRouteActivation({
      activeSessionId,
      buildBaseline: () => ({
        providerConfigId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: canonicalModelId("gemma-4-E2B-it-qat-q4_0-gguf"),
        reasoningDepth: "medium",
        contextBudget: 12_288,
      }),
      canActivate: true,
      dynamicRoutingEnabled: true,
      ensureSession,
      getRoute: () => ({
        providerId: providerConfigurationId("provider-302"),
        providerType: providerTypeId("local_model"),
        modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        reasoning: "medium",
        context: "12288",
      }),
      onSessionsChange: vi.fn(),
      sessions: [session("session-a"), session("session-b")],
      setStatus: vi.fn(),
      statusBlocked: "blocked",
      statusDisabled: "disabled",
      statusEnabled: "enabled",
      unlockSession: vi.fn(),
    }));

    act(() => void hook.result.current.toggle(true));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    act(() => void hook.result.current.toggle(false));
    activeSessionId = "session-b";
    hook.rerender();
    await act(async () => {
      resolveFirst({
        session: session("session-a", 2),
        receipt: {
          kind: "auto_route_activation",
          receiptId: "receipt-a",
          dynamicRoutingEnabled: true,
          committed: true,
          rolledBack: false,
          changed: true,
        },
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(hook.result.current.isSaving).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(ensureSession).toHaveBeenCalledTimes(1);
}

describe("atomic Auto-route activation", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());
  afterEach(cleanup);

  it("coalesces a tuning change and merges the receipt into the latest session list", coalesceTuningChange);
  it("replays a rapid toggle against the one session created by the first mutation", replayRapidToggle);
  it("clears a failed activation when the active session changes", clearStaleFailureOnSessionSwitch);
  it("drops a queued activation when its active session changes", clearStalePendingActivationOnSessionSwitch);
});
