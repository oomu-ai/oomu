import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useConfiguredProviders } from "./useConfiguredProviders";
import type { ConfiguredProvider } from "@/lib/modelRegistry";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const localProvider: ConfiguredProvider = {
  id: "local-provider",
  providerId: "local_model",
  providerName: "Local",
  authMethod: "custom",
  baseUrl: "http://127.0.0.1",
  apiKeyLabel: "",
  customModelIds: "ready-model\nmissing-model",
};

const remoteProvider: ConfiguredProvider = {
  ...localProvider,
  id: "remote-provider",
  providerId: "google",
  providerName: "Google",
  customModelIds: "gemini-model",
};

describe("useConfiguredProviders", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("does not read or seed native provider state before license acceptance", async () => {
    invokeMock.mockResolvedValue([]);
    const { rerender, result } = renderHook(
      ({ accepted }) => useConfiguredProviders(accepted),
      { initialProps: { accepted: false } },
    );

    expect(result.current[0]).toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();

    rerender({ accepted: true });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_provider_configs");
    });
  });

  it("reconciles a local provider only to models confirmed ready by native state", async () => {
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "list_provider_configs") return [localProvider];
      if (command === "list_local_models") {
        return [{ id: "ready-model", compatibility: "ready" }];
      }
      if (command === "save_provider_config") {
        return (args as { request: ConfiguredProvider }).request;
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const { result } = renderHook(() => useConfiguredProviders(true));

    await waitFor(() => {
      expect(result.current[0]).toEqual([
        { ...localProvider, customModelIds: "ready-model" },
      ]);
    });
    expect(invokeMock).toHaveBeenCalledWith("save_provider_config", {
      request: { ...localProvider, customModelIds: "ready-model" },
    });
  });

  it("keeps persisted providers visible when local model discovery fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_provider_configs") return [remoteProvider, localProvider];
      if (command === "list_local_models") throw new Error("inventory not ready");
      throw new Error(`unexpected command: ${command}`);
    });

    const { result } = renderHook(() => useConfiguredProviders(true));

    await waitFor(() => {
      expect(result.current[0]).toEqual([remoteProvider, localProvider]);
    });
  });

  it("does not clear a local provider while native inventory is still loading", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_provider_configs") return [remoteProvider, localProvider];
      if (command === "list_local_models") return [];
      throw new Error(`unexpected command: ${command}`);
    });

    const { result } = renderHook(() => useConfiguredProviders(true));

    await waitFor(() => {
      expect(result.current[0]).toEqual([remoteProvider, localProvider]);
    });
    expect(invokeMock).not.toHaveBeenCalledWith("save_provider_config", expect.anything());
  });

  it("keeps a settings update available through the returned state setter", () => {
    invokeMock.mockResolvedValue([]);
    const { result } = renderHook(() => useConfiguredProviders(false));

    act(() => {
      result.current[1]([remoteProvider]);
    });

    expect(result.current[0]).toEqual([remoteProvider]);
  });
});
