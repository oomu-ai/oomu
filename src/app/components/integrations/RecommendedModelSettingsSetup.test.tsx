import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { RecommendedModelSettingsSetup } from "./RecommendedModelSettingsSetup";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));
vi.mock("./RecommendedLocalModelSetup", () => ({
  RecommendedLocalModelSetup: () => <div data-testid="recommended-model-recovery" />,
}));

const modelId = "gemma-4-E2B-it-qat-q4_0-gguf";
const configuredProvider: ConfiguredProvider = {
  id: "local-model",
  providerId: "local_model",
  providerName: "On-device model",
  authMethod: "custom",
  baseUrl: "",
  apiKeyLabel: "",
  customModelIds: modelId,
};

function renderSettings(providers: ConfiguredProvider[] = [configuredProvider]) {
  return render(
    <RecommendedModelSettingsSetup
      configuredProviders={providers}
      onProvidersChange={vi.fn()}
    />,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(cleanup);

describe("RecommendedModelSettingsSetup", () => {
  it("hides recovery only when the exact provider and exact ready model both exist", async () => {
    invokeMock.mockResolvedValue([{ id: modelId, compatibility: "ready" }]);

    renderSettings();

    expect(screen.getByTestId("recommended-model-recovery")).toBeVisible();
    await waitFor(() => expect(
      screen.queryByTestId("recommended-model-recovery"),
    ).toBeNull());
  });

  it("keeps recovery visible when a configured exact model is missing or damaged", async () => {
    invokeMock.mockResolvedValue([{ id: modelId, compatibility: "incompatible" }]);

    renderSettings();

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_local_models"));
    expect(screen.getByTestId("recommended-model-recovery")).toBeVisible();
  });

  it("keeps recovery visible when the model is ready but its provider is absent", async () => {
    invokeMock.mockResolvedValue([{ id: modelId, compatibility: "ready" }]);

    renderSettings([]);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_local_models"));
    expect(screen.getByTestId("recommended-model-recovery")).toBeVisible();
  });
});
