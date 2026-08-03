import { describe, expect, it } from "vitest";
import type { ChatSession } from "@/lib/chatSessions";
import {
  authoritativeSessionConfigRouteIdentity,
  legacySessionConfigWriteAllowed,
  type SessionConfigRecord,
} from "./autoRouteSessionIdentity";
import { routeBindingForDynamicRouting, type RouteOverride } from "./sessionRouting";

const savedLocalRoute: RouteOverride = {
  providerId: "prov-5",
  providerType: "local_model",
  modelId: "gemma-4-12B-it-qat-q4_0-gguf",
  reasoning: "medium",
  context: "16384",
};

describe("auto-route session baseline", () => {
  it("keeps dynamic binding separate from the authoritative local baseline", () => {
    expect(routeBindingForDynamicRouting(true, savedLocalRoute)).toEqual({
      providerId: "dynamic",
      modelId: "dynamic",
      dynamicRoutingOverride: true,
      autoRouteBaseline: savedLocalRoute,
    });
  });

  it("does not attach an Auto-route baseline to a manual session", () => {
    expect(routeBindingForDynamicRouting(false, savedLocalRoute)).toEqual({
      providerId: "prov-5",
      modelId: "gemma-4-12B-it-qat-q4_0-gguf",
      dynamicRoutingOverride: undefined,
      autoRouteBaseline: undefined,
    });
  });

  it("rejects legacy config writes for every dynamic-bound turn lifecycle", () => {
    const dynamicSession = {
      id: "session-auto-route",
      agentId: "agent-302",
      title: "Auto-route",
      providerId: "dynamic",
      modelId: "dynamic",
      dynamicRoutingOverride: true,
      createdAtMs: 1,
      updatedAtMs: 1,
    } satisfies ChatSession;

    expect(legacySessionConfigWriteAllowed(dynamicSession)).toBe(false);
    expect(legacySessionConfigWriteAllowed(dynamicSession, true)).toBe(false);
    expect(legacySessionConfigWriteAllowed({
      ...dynamicSession,
      dynamicRoutingOverride: false,
    })).toBe(true);
    expect(legacySessionConfigWriteAllowed({
      ...dynamicSession,
      dynamicRoutingOverride: false,
    }, true)).toBe(false);
    expect(legacySessionConfigWriteAllowed({
      ...dynamicSession,
      dynamicRoutingOverride: null,
    })).toBe(false);
    expect(legacySessionConfigWriteAllowed({
      ...dynamicSession,
      providerId: savedLocalRoute.providerId,
      modelId: savedLocalRoute.modelId,
    })).toBe(true);
  });

  it("hydrates manual and dynamic routes only from the complete typed identity", () => {
    expect(authoritativeSessionConfigRouteIdentity({
      localProviderConfigId: "prov-5",
      localProviderType: "local_model",
      modelId: "gemma-4-12B-it-qat-q4_0-gguf",
      localRouteGeneration: 7,
    })).toEqual({
      providerConfigId: "prov-5",
      providerType: "local_model",
      modelId: "gemma-4-12B-it-qat-q4_0-gguf",
      generation: 7,
    });

    expect(authoritativeSessionConfigRouteIdentity({
      modelId: "gemma-4-12B-it-qat-q4_0-gguf",
      localProviderConfigId: "prov-5",
      localRouteGeneration: 7,
    })).toBeNull();
  });

  it("does not hydrate a route from legacy provider_id", () => {
    const legacyOnly = {
      provider_id: "legacy-must-not-hydrate",
      model_id: "gemma-4-12B-it-qat-q4_0-gguf",
      local_route_generation: 7,
    } as unknown as SessionConfigRecord;
    expect(authoritativeSessionConfigRouteIdentity(legacyOnly)).toBeNull();
  });
});
