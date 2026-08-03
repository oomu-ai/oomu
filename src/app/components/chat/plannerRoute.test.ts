import { describe, expect, it } from "vitest";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { plannerRequestRoute } from "./plannerRoute";

const localProvider: ConfiguredProvider = {
  id: "prov-2",
  providerId: "local_model",
  providerName: "Local models",
  authMethod: "custom",
  baseUrl: "",
  apiKeyLabel: "",
  customModelIds: "",
};

const route: ChatTurnContext["route"] = {
  providerId: "prov-2",
  modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
  dynamicRoutingEnabled: false,
  automatedWebGroundingEnabled: false,
};

describe("plannerRequestRoute", () => {
  it("canonicalizes an opaque configured local provider before crossing the native boundary", () => {
    expect(plannerRequestRoute([localProvider], route)).toEqual({
      selected_model: "local_gemma",
      selected_provider_id: "local_model",
      selected_model_id: "gemma-4-E4B-it-qat-q4_0-gguf",
      dynamic_routing_enabled: false,
    });
  });

  it("preserves an opaque configured cloud provider so native code can resolve its credentials", () => {
    const cloudProvider = {
      ...localProvider,
      id: "prov-cloud",
      providerId: "google",
      providerName: "Google Gemini",
    };
    expect(
      plannerRequestRoute([cloudProvider], {
        ...route,
        providerId: "prov-cloud",
        modelId: "gemini-3.5-flash",
      }).selected_provider_id,
    ).toBe("prov-cloud");
  });
});
