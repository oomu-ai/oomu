import { describe, expect, it } from "vitest";
import {
  agentToStartupConfigRequest,
  configToAgent,
  defaultLocalAgentEndpoint,
  normalizeAgentCards,
  shouldShowDegradedLanding,
  startupAgentConfigRequests,
  type AgentConfigRecord,
  type DegradedModeStatus,
} from "@/app/homeAgents";

function agentConfigWithProfile(
  personalityProfile: AgentConfigRecord["personality_profile"],
): AgentConfigRecord {
  return {
    id: "imported-oomu",
    name: "OOMU",
    system_prompt: "You are OOMU.",
    model_id: "gemma-4-2b",
    provider_id: "local_model",
    description: "A strategic operating partner.",
    image: null,
    personality_profile: personalityProfile,
    favorited: false,
    status: "active",
    created_at_ms: 1,
    updated_at_ms: 1,
  };
}

describe("configToAgent", () => {
  it("does not guess a model for an implicit local agent assignment", () => {
    expect(defaultLocalAgentEndpoint).toEqual({
      provider: "local_model",
      modelId: "",
    });
  });

  it("keeps an intentionally blank description separate from private instructions", () => {
    const stored = agentConfigWithProfile("{}");
    stored.description = "";
    stored.system_prompt = "PRIVATE SYSTEM INSTRUCTION";

    const agent = configToAgent(stored);

    expect(agent.description).toBe("");
    expect(agent.systemPrompt).toBe("PRIVATE SYSTEM INSTRUCTION");
    expect(agent.personalityProfile?.personality.summary).toBe("");
  });

  it("normalizes a stored empty JSON personality profile into default panel data", () => {
    const agent = configToAgent(agentConfigWithProfile("{}"));

    expect(agent.personalityProfile?.identity.displayName).toBe("OOMU");
    expect(agent.personalityProfile?.personality.summary).toBe("A strategic operating partner.");
    expect(agent.personalityProfile?.modelBehavior.maxOutputTokens).toBe(2048);
    expect(agent.personalityProfile?.personality.traits).toEqual([
      "friendly",
      "concise",
      "supportive",
    ]);
  });

  it("normalizes an empty object personality profile into default panel data", () => {
    const agent = configToAgent(
      agentConfigWithProfile({} as AgentConfigRecord["personality_profile"]),
    );

    expect(agent.personalityProfile?.identity.displayName).toBe("OOMU");
    expect(agent.personalityProfile?.relationship.boundaries.length).toBeGreaterThan(0);
  });

  it("preserves and snaps stored maximum output tokens", () => {
    const agent = configToAgent(
      agentConfigWithProfile({
        schemaVersion: 1,
        identity: {
          displayName: "OOMU",
          role: "Strategist",
        },
        personality: {
          summary: "A strategic operating partner.",
          traits: ["strategic"],
          tone: "Focused.",
        },
        relationship: {
          userAddress: "the user",
          boundaries: ["Stay grounded."],
        },
        modelBehavior: {
          baseModelDisclosure: "runtime_only",
          nameQuestionBehavior: "agent_name",
          maxOutputTokens: 7600,
        },
      }),
    );

    expect(agent.personalityProfile?.modelBehavior.maxOutputTokens).toBe(7168);
  });
});

describe("startup agent persistence", () => {
  const implicitLocalAgent = {
    id: "agent-startup",
    name: "OOMU",
    description: "Keeps work moving.",
    systemPrompt: "Keep work moving.",
    endpoint: defaultLocalAgentEndpoint,
  };

  it("waits instead of saving an implicit local agent before startup verification", () => {
    expect(agentToStartupConfigRequest(implicitLocalAgent, null)).toBeNull();
  });

  it("persists the exact verified startup model without guessing a family", () => {
    const request = agentToStartupConfigRequest(
      implicitLocalAgent,
      "gemma-4-E2B-it-qat-q4_0-gguf",
    );

    expect(request).toMatchObject({
      provider_id: "local_model",
      model_id: "gemma-4-E2B-it-qat-q4_0-gguf",
    });
  });

  it("preserves an explicit agent model instead of replacing it with startup state", () => {
    const request = agentToStartupConfigRequest({
      ...implicitLocalAgent,
      endpoint: {
        provider: "google",
        modelId: "gemini-3.6-flash",
      },
    }, "gemma-4-E2B-it-qat-q4_0-gguf");

    expect(request).toMatchObject({
      provider_id: "google",
      model_id: "gemini-3.6-flash",
    });
  });

  it("preserves an explicit local model instead of replacing its family", () => {
    const request = agentToStartupConfigRequest({
      ...implicitLocalAgent,
      endpoint: {
        provider: "local_model",
        modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
      },
    }, "gemma-4-E2B-it-qat-q4_0-gguf");

    expect(request).toMatchObject({
      provider_id: "local_model",
      model_id: "gemma-4-E4B-it-qat-q4_0-gguf",
    });
  });

  it("keeps an explicit user-created batch idle until one model is verified", () => {
    const userAgents = normalizeAgentCards([implicitLocalAgent], "active");

    expect(startupAgentConfigRequests(userAgents, null)).toBeNull();
    const requests = startupAgentConfigRequests(
      userAgents,
      "gemma-4-E2B-it-qat-q4_0-gguf",
    );
    expect(requests).toHaveLength(userAgents.length);
    expect(requests?.every((request) =>
      request.model_id === "gemma-4-E2B-it-qat-q4_0-gguf"
    )).toBe(true);
  });
});

describe("degraded persistence presentation", () => {
  const status = (
    hasVolatileStorage: boolean,
    subsystem = "inference",
  ): DegradedModeStatus => ({
    active: true,
    reason: "persistence recovery required",
    hasVolatileStorage,
    subsystems: [
      {
        subsystem,
        active: true,
        cause: "recovery required",
        firstOccurredAtMs: 1,
        backingStoreClass: "notApplicable",
        recoveryEligible: true,
        lastProbeResult: null,
        userVisibleImpact: "The affected capability is unavailable.",
      },
    ],
  });

  it("keeps the volatile-storage recovery screen visible in Settings", () => {
    expect(shouldShowDegradedLanding(status(true), "settings")).toBe(true);
  });

  it("allows Settings for an inference-only setup failure", () => {
    expect(shouldShowDegradedLanding(status(false), "settings")).toBe(false);
    expect(shouldShowDegradedLanding(status(false), "chat")).toBe(true);
  });

  it("does not invent subsystem health when the native probe is unavailable", () => {
    expect(shouldShowDegradedLanding(null, "settings")).toBe(false);
  });

  it("keeps feature-local artifact failures from replacing the whole app", () => {
    expect(shouldShowDegradedLanding(status(false, "artifactPipeline"), "chat")).toBe(false);
  });

  it("keeps an unavailable Auto-route classifier inside the chat choice surface", () => {
    expect(shouldShowDegradedLanding(status(false, "autoRouteClassifier"), "chat")).toBe(false);
    expect(shouldShowDegradedLanding(status(false, "autoRouteClassifier"), "projects")).toBe(false);
  });

  it("keeps saved Auto-route model repair inside the Auto-route choice surface", () => {
    expect(shouldShowDegradedLanding(status(false, "autoRouteSessionBaselines"), "chat")).toBe(false);
    expect(shouldShowDegradedLanding(status(false, "autoRouteSessionBaselines"), "projects")).toBe(false);
  });

  it("keeps an identity-only repair local to identity-backed features", () => {
    expect(shouldShowDegradedLanding(status(false, "identity"), "chat")).toBe(false);
    expect(shouldShowDegradedLanding(status(false, "identity"), "projects")).toBe(false);
  });
});
