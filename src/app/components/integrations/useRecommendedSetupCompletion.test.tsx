import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_LOCAL_MODEL_ID } from "@/lib/modelRegistry";
import { useRecommendedSetupCompletion } from "./useRecommendedSetupCompletion";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const configuredProvider = {
  id: "local-model",
  providerId: "local_model",
  providerName: "On-device model",
  authMethod: "custom" as const,
  baseUrl: "",
  apiKeyLabel: "",
  customModelIds: DEFAULT_LOCAL_MODEL_ID,
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([configuredProvider]);
});

describe("useRecommendedSetupCompletion deferral", () => {
  it("never advances a stale model step after the user continues during verification", async () => {
    let resolveModels: ((models: Array<{ id: string; compatibility: string }>) => void) | undefined;
    const refreshLocalModels = vi.fn().mockImplementation(() => new Promise((resolve) => {
      resolveModels = resolve;
    }));
    const advance = vi.fn().mockResolvedValue(undefined);
    const options = {
      advance,
      applyProviders: vi.fn(),
      onError: vi.fn(),
      refreshLocalModels,
      setBusy: vi.fn(),
    };
    const { result } = renderHook(() => useRecommendedSetupCompletion(options));
    let acceptance: Promise<void> | undefined;

    act(() => {
      acceptance = result.current.accept({
        providerId: "local-model",
        providerType: "local_model",
        modelId: DEFAULT_LOCAL_MODEL_ID,
        verified: true,
      });
    });
    await waitFor(() => expect(refreshLocalModels).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.defer();
    });
    await act(async () => {
      resolveModels?.([{ id: DEFAULT_LOCAL_MODEL_ID, compatibility: "ready" }]);
      await acceptance;
    });

    expect(advance).toHaveBeenCalledTimes(1);
    expect(options.onError).not.toHaveBeenCalled();
  });
});
