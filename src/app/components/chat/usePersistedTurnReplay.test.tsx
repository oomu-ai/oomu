import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AutoRouteAttention } from "./AutoRouteAttentionCard";
import { createReplayAwareTurnContext } from "./persistedTurnReplayContext";
import { usePersistedTurnReplay } from "./usePersistedTurnReplay";

const attention: AutoRouteAttention = {
  sessionId: "session-301",
  rootTurnId: "turn-301",
  turnId: "turn-301",
  generationToken: "generation-301",
  localProviderId: "local_model",
  localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
  recommendedLocalProviderId: "local_model",
  recommendedLocalModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
  cloudModelId: "gemini-3.5-flash",
  failureCode: "classifier_inference_timeout",
  failureBoundary: "auto_route_classifier_inference",
  kind: "timeout",
};

afterEach(cleanup);

describe("persisted turn replay", () => {
  it("retrieves the exact encrypted-transcript turn and submits it once", async () => {
    const submit = vi.fn(async (_message, options) => options.onAccepted?.());
    const hook = renderHook(() => usePersistedTurnReplay({
      activeSessionId: attention.sessionId,
      messages: [{
        content: "What is on my calendar today?",
        role: "user",
        providerId: "dynamic",
        modelId: "dynamic",
        metadata: { turnId: attention.turnId, generationToken: attention.generationToken, turnState: "accepted" },
      }],
      submit,
    }));
    let resumed = false;
    await act(async () => {
      resumed = await hook.result.current.resumeAutoRouteTurn(attention, "local");
    });

    expect(resumed).toBe(true);
    expect(submit).toHaveBeenCalledTimes(1);
    expect(submit).toHaveBeenCalledWith(
      "What is on my calendar today?",
      expect.objectContaining({
        autoRouteResumeChoice: "local",
        resumeAcceptedTurn: {
          sessionId: attention.sessionId,
          rootTurnId: attention.rootTurnId,
          turnId: attention.turnId,
          generationToken: attention.generationToken,
          providerId: "dynamic",
          modelId: "dynamic",
          turnState: "accepted",
          dynamicRoutingEnabled: true,
        },
      }),
    );
  });

  it("refuses to invent a route for the accepted turn", async () => {
    const submit = vi.fn();
    const hook = renderHook(() => usePersistedTurnReplay({
      activeSessionId: attention.sessionId,
      messages: [{
        content: "What is on my calendar today?",
        role: "user",
        metadata: { turnId: attention.turnId, generationToken: attention.generationToken, turnState: "accepted" },
      }],
      submit,
    }));

    await expect(hook.result.current.resumeAutoRouteTurn(attention, "retry"))
      .resolves.toBe(false);
    expect(submit).not.toHaveBeenCalled();
  });

  it("marks a restart-interrupted replay for atomic native resumption", async () => {
    const submit = vi.fn(async (_message, options) => options.onAccepted?.());
    const hook = renderHook(() => usePersistedTurnReplay({
      activeSessionId: attention.sessionId,
      messages: [{
        content: "What is on my calendar today?",
        role: "user",
        providerId: "dynamic",
        modelId: "dynamic",
        metadata: {
          turnId: attention.turnId,
          generationToken: attention.generationToken,
          turnState: "interrupted",
        },
      }],
      submit,
    }));

    await expect(hook.result.current.resumeAutoRouteTurn(attention, "retry"))
      .resolves.toBe(true);
    expect(submit).toHaveBeenCalledWith(
      "What is on my calendar today?",
      expect.objectContaining({
        resumeAcceptedTurn: expect.objectContaining({ turnState: "interrupted" }),
      }),
    );
  });

  it("refuses to replay a turn that is already terminal", async () => {
    const submit = vi.fn();
    const hook = renderHook(() => usePersistedTurnReplay({
      activeSessionId: attention.sessionId,
      messages: [{
        content: "What is on my calendar today?",
        role: "user",
        providerId: "dynamic",
        modelId: "dynamic",
        metadata: {
          turnId: attention.turnId,
          generationToken: attention.generationToken,
          turnState: "completed",
        },
      }],
      submit,
    }));

    await expect(hook.result.current.resumeAutoRouteTurn(attention, "retry"))
      .resolves.toBe(false);
    expect(submit).not.toHaveBeenCalled();
  });
});

describe("persisted turn replay identity", () => {
  it("rebuilds the original accepted identity and frozen route", () => {
    const resumed = createReplayAwareTurnContext({
      sessionId: attention.sessionId,
      rootTurnId: attention.rootTurnId,
      turnId: attention.turnId,
      generationToken: attention.generationToken,
      providerId: "dynamic",
      modelId: "dynamic",
      turnState: "accepted",
      dynamicRoutingEnabled: true,
    }, {
      turnId: "new-turn-that-must-not-be-used",
      generationToken: "new-generation-that-must-not-be-used",
      sessionId: attention.sessionId,
      agentId: "agent-301",
      route: {
        providerId: "local_model",
        modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        dynamicRoutingEnabled: false,
        automatedWebGroundingEnabled: false,
      },
    });

    expect(resumed).toMatchObject({
      turnId: attention.turnId,
      generationToken: attention.generationToken,
      route: { providerId: "dynamic", modelId: "dynamic", dynamicRoutingEnabled: true },
    });
  });

  it("preserves Auto-route while replaying the exact claimed local executor", () => {
    const resumed = createReplayAwareTurnContext({
      sessionId: attention.sessionId,
      rootTurnId: attention.rootTurnId,
      turnId: attention.turnId,
      generationToken: attention.generationToken,
      providerId: "prov-local-sprint-302",
      modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
      turnState: "interrupted",
      dynamicRoutingEnabled: true,
    }, {
      turnId: "new-turn",
      generationToken: "new-generation",
      sessionId: attention.sessionId,
      agentId: "agent-301",
      route: {
        providerId: "prov-local-sprint-302",
        modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
        dynamicRoutingEnabled: false,
        automatedWebGroundingEnabled: false,
      },
    });

    expect(resumed.route).toMatchObject({
      providerId: "prov-local-sprint-302",
      modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
      dynamicRoutingEnabled: true,
    });
  });

  it("refuses to guess when the original turn is absent", async () => {
    const submit = vi.fn();
    const hook = renderHook(() => usePersistedTurnReplay({
      activeSessionId: attention.sessionId,
      messages: [],
      submit,
    }));

    await expect(hook.result.current.resumeAutoRouteTurn(attention, "retry"))
      .resolves.toBe(false);
    expect(submit).not.toHaveBeenCalled();
  });
});
