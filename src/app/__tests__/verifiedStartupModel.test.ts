import { describe, expect, it } from "vitest";
import {
  canCreateAgentWithModel,
  resolvedAgentSessionRoute,
  verifiedStartupRouteForAgentEndpoint,
  verifiedStartupModelId,
} from "../verifiedStartupModel";

function readyHealth(modelId: string) {
  return {
    status: "ready",
    requestedModelId: modelId,
    classifierModelId: modelId,
    readinessGeneration: 4,
    residencyGeneration: 2,
    verifiedResidencyGeneration: 2,
    lastVerifiedAtMs: 1_784_000_000_000,
  };
}

describe("verified startup model", () => {
  it("keeps an explicitly verified E4B startup assignment", () => {
    expect(verifiedStartupModelId(readyHealth("gemma-4-E4B-it-qat-q4_0-gguf")))
      .toBe("gemma-4-E4B-it-qat-q4_0-gguf");
  });

  it("fails closed when residency is missing, stale, or ambiguous", () => {
    expect(verifiedStartupModelId(null)).toBeNull();
    expect(verifiedStartupModelId({ ...readyHealth("E4B"), status: "recovering" })).toBeNull();
    expect(verifiedStartupModelId({
      ...readyHealth("E4B"),
      verifiedResidencyGeneration: 1,
    })).toBeNull();
  });

  it("uses verified E4B for an implicit session and never guesses when it is missing", () => {
    expect(resolvedAgentSessionRoute("", "", "gemma-4-E4B-it-qat-q4_0-gguf"))
      .toEqual({
        providerId: "local_model",
        modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
      });
    expect(resolvedAgentSessionRoute("", "", null)).toEqual({
      providerId: "",
      modelId: "",
    });
  });

  it("requires either a verified implicit model or an explicit model choice", () => {
    expect(canCreateAgentWithModel("Avery", "", "local_model", "E4B")).toBe(true);
    expect(canCreateAgentWithModel("Avery", "", "local_model", null)).toBe(false);
    expect(canCreateAgentWithModel("Avery", "cloud-model", "google", null)).toBe(true);
  });

  it("recovers only an implicit or matching local agent route from verified startup", () => {
    expect(verifiedStartupRouteForAgentEndpoint("local_model", "E2B", "E2B"))
      .toEqual({ providerId: "local_model", modelId: "E2B" });
    expect(verifiedStartupRouteForAgentEndpoint("", "", "E2B"))
      .toEqual({ providerId: "local_model", modelId: "E2B" });
    expect(verifiedStartupRouteForAgentEndpoint("local_model", "E4B", "E2B"))
      .toBeNull();
    expect(verifiedStartupRouteForAgentEndpoint("google", "gemini-3.6-flash", "E2B"))
      .toBeNull();
    expect(verifiedStartupRouteForAgentEndpoint("local_model", "E2B", null))
      .toBeNull();
  });
});
