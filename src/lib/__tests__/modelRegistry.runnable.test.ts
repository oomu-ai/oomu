import { describe, expect, it } from "vitest";
import {
  configuredModelOptions,
  configuredProviderIsRunnable,
  providerOptionsFromConfigured,
  resolveConfiguredModelRoute,
} from "../modelRegistry";

describe("runnable model providers", () => {
  it("excludes an incomplete cloud provider without hiding a local model", () => {
    const providers = [
      {
        id: "cloud-missing-key", providerId: "openai", providerName: "OpenAI",
        authMethod: "api_key" as const, baseUrl: "https://api.openai.com/v1",
        apiKeyLabel: "OPENAI_API_KEY", customModelIds: "gpt-5.6-sol", credentialConfigured: false,
      },
      {
        id: "local-ready", providerId: "local_model", providerName: "Local",
        authMethod: "custom" as const, baseUrl: "", apiKeyLabel: "",
        customModelIds: "gemma-4-E4B-it-qat-q4_0-gguf", credentialConfigured: false,
      },
    ];
    expect(configuredProviderIsRunnable(providers[0])).toBe(false);
    expect(configuredProviderIsRunnable(providers[1])).toBe(true);
    expect(providerOptionsFromConfigured(providers)).toEqual([{ id: "local-ready", label: "Local" }]);
    expect(configuredModelOptions(providers).map((option) => option.providerId)).toEqual(["local-ready"]);
  });

  it("keeps legacy cloud fixtures eligible when credential state is absent", () => {
    expect(configuredProviderIsRunnable({
      id: "legacy", providerId: "google", providerName: "Google",
      authMethod: "api_key", baseUrl: "", apiKeyLabel: "GOOGLE_API_KEY",
      customModelIds: "gemini-3.6-flash",
    })).toBe(true);
  });

  it("resolves a provider type and model to its unique runnable configuration", () => {
    const providers = [
      {
        id: "gemini-first", providerId: "google", providerName: "Gemini",
        authMethod: "api_key" as const, baseUrl: "", apiKeyLabel: "GEMINI_API_KEY",
        customModelIds: "gemini-3.6-flash",
      },
      {
        id: "local-e2b", providerId: "local_model", providerName: "On-device",
        authMethod: "custom" as const, baseUrl: "", apiKeyLabel: "",
        customModelIds: "gemma-4-E2B-it-qat-q4_0-gguf",
      },
    ];

    expect(resolveConfiguredModelRoute(
      providers,
      "local_model",
      "gemma-4-E2B-it-qat-q4_0-gguf",
    )).toMatchObject({
      providerConfigId: "local-e2b",
      providerType: "local_model",
      modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    });
  });

  it("prefers an exact configuration and rejects an ambiguous provider type", () => {
    const providers = ["local-a", "local-b"].map((id) => ({
      id, providerId: "local_model", providerName: id,
      authMethod: "custom" as const, baseUrl: "", apiKeyLabel: "",
      customModelIds: "gemma-4-E2B-it-qat-q4_0-gguf",
    }));

    expect(resolveConfiguredModelRoute(
      providers,
      "local-a",
      "gemma-4-E2B-it-qat-q4_0-gguf",
    )?.providerConfigId).toBe("local-a");
    expect(resolveConfiguredModelRoute(
      providers,
      "local_model",
      "gemma-4-E2B-it-qat-q4_0-gguf",
    )).toBeNull();
  });
});
