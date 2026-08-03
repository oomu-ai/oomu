import { describe, expect, it } from "vitest";
import {
  chatTurnContextMatches,
  createChatTurnContext,
  deriveChatTurnContext,
  rebindChatTurnExecutionRoute,
  type ChatTurnContext,
} from "../chatTurnContext";

function rootTurn(sessionId: string, suffix: string): ChatTurnContext {
  return createChatTurnContext({
    turnId: `turn-${suffix}`,
    generationToken: `generation-${suffix}`,
    sessionId,
    agentId: `agent-${sessionId}`,
    projectId: `project-${sessionId}`,
    route: {
      providerId: `provider-${sessionId}`,
      modelId: `model-${sessionId}`,
      reasoning: "high",
      contextBudget: 8192,
      primaryRouteId: "primary",
      fallbackRouteId: "fallback",
      dynamicRoutingEnabled: false,
      automatedWebGroundingEnabled: true,
    },
    attachmentGrants: [{ name: `${suffix}.txt`, mimeType: "text/plain", byteCount: 12 }],
    createdAtMs: 100,
  });
}

describe("immutable chat turn context", () => {
  it("snapshots nested route and attachment values", () => {
    const context = rootTurn("session-a", "a1");

    expect(Object.isFrozen(context)).toBe(true);
    expect(Object.isFrozen(context.route)).toBe(true);
    expect(Object.isFrozen(context.attachmentGrants)).toBe(true);
    expect(Object.isFrozen(context.attachmentGrants[0])).toBe(true);
    expect(context.ancestry).toEqual({
      kind: "root",
      parentTurnId: null,
      rootTurnId: "turn-a1",
    });
    expect(chatTurnContextMatches(context, { ...context, projectId: "project-other" })).toBe(false);
  });

  it.each(["queued", "steer", "retry"] as const)(
    "derives %s turns from the parent's immutable ownership and route",
    (kind) => {
      const parent = rootTurn("session-a", "parent");
      const derived = deriveChatTurnContext(parent, kind, {
        turnId: `turn-${kind}`,
        generationToken: `generation-${kind}`,
      });

      expect(derived.sessionId).toBe(parent.sessionId);
      expect(derived.agentId).toBe(parent.agentId);
      expect(derived.projectId).toBe(parent.projectId);
      expect(derived.route).toEqual(parent.route);
      expect(derived.ancestry).toEqual({
        kind,
        parentTurnId: parent.turnId,
        rootTurnId: parent.turnId,
      });
    },
  );

  it("rebinds only the execution route after an Auto-route turn is claimed", () => {
    const context = rootTurn("session-a", "dynamic");
    const rebound = rebindChatTurnExecutionRoute(context, "provider-concrete", "model-concrete");

    expect(rebound).toEqual({
      ...context,
      route: {
        ...context.route,
        providerId: "provider-concrete",
        modelId: "model-concrete",
      },
    });
    expect(rebound.turnId).toBe(context.turnId);
    expect(rebound.generationToken).toBe(context.generationToken);
    expect(rebound.ancestry).toEqual(context.ancestry);
    expect(rebound.attachmentGrants).toEqual(context.attachmentGrants);
  });
});
