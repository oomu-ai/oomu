import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { DegradedModeStatus } from "../../homeAgents";
import { DegradedModeLanding } from "../HomeChrome";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

const INFERENCE_DEGRADED_STATUS: DegradedModeStatus = {
  active: true,
  reason: "inference: selected model unavailable",
  hasVolatileStorage: false,
  subsystems: [
    {
      subsystem: "inference",
      active: true,
      cause: "selected model unavailable",
      firstOccurredAtMs: 1,
      backingStoreClass: "notApplicable",
      recoveryEligible: true,
      lastProbeResult: null,
      userVisibleImpact: "Local model generation is unavailable.",
    },
  ],
};

const READY_MODELS = [
  {
    id: "gemma-ready-small",
    name: "Gemma Ready Small",
    compatibility: "ready",
  },
  {
    id: "broken-model",
    name: "Broken Model",
    compatibility: "invalid",
  },
  {
    id: "gemma-ready-large",
    name: "Gemma Ready Large",
    compatibility: "ready",
  },
];

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [
      {
        id: "en-US",
        label: "English (US)",
        fileName: "en-US.json",
        isDefault: true,
        verified: true,
      },
    ],
    translations: {},
  };
}

describe("DegradedModeLanding local inference recovery", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  afterEach(cleanup);

  it("lists only ready models and recovers the exact selected model", async () => {
    const onStatusChange = vi.fn();
    const recoveredStatus = {
      active: false,
      reason: null,
      hasVolatileStorage: false,
      subsystems: [],
    };
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_local_models") return READY_MODELS;
      if (command === "recover_local_inference") {
        const selected = String(args?.modelId ?? "");
        return {
          modelId: selected,
          modelName: "Gemma Ready Large",
          degradedMode: recoveredStatus,
        };
      }
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        onStatusChange={onStatusChange}
        status={INFERENCE_DEGRADED_STATUS}
      />,
      { wrapper: I18nProvider },
    );

    const modelSelect = await screen.findByRole("combobox", { name: "Local model" });
    await waitFor(() => expect(modelSelect).not.toBeDisabled());
    expect(within(modelSelect).getByRole("option", { name: "Gemma Ready Small" })).toBeTruthy();
    expect(within(modelSelect).queryByRole("option", { name: "Broken Model" })).toBeNull();

    fireEvent.change(modelSelect, { target: { value: "gemma-ready-large" } });
    fireEvent.click(screen.getByRole("button", { name: "Use selected model" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("recover_local_inference", {
        modelId: "gemma-ready-large",
        model_id: "gemma-ready-large",
      }),
    );
    expect(await screen.findByText("Gemma Ready Large is ready. OOMU is resuming.")).toBeVisible();
    expect(onStatusChange).toHaveBeenCalledWith(recoveredStatus);
  });

  it("refreshes selectable models after choosing a different folder", async () => {
    let folderSelected = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_local_models") return folderSelected ? [READY_MODELS[0]] : [];
      if (command === "choose_local_model_directory") {
        folderSelected = true;
        return { path: "/tmp/ready-models", isDefault: false };
      }
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        status={INFERENCE_DEGRADED_STATUS}
      />,
      { wrapper: I18nProvider },
    );

    const recoverButton = screen.getByRole("button", { name: "Use selected model" });
    await waitFor(() => expect(recoverButton).toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Choose model folder..." }));

    const modelSelect = await screen.findByRole("combobox", { name: "Local model" });
    await waitFor(() => expect(modelSelect).not.toBeDisabled());
    expect(within(modelSelect).getByRole("option", { name: "Gemma Ready Small" })).toBeTruthy();
    expect(screen.getByText(/Choose a ready model below to finish recovery/)).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "list_local_models").length)
      .toBeGreaterThanOrEqual(2);
  });

  it("replaces backend recovery prose with calm localized impact copy", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_persistence_recovery_status") return null;
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        status={{
          active: true,
          reason: "storage unavailable",
          hasVolatileStorage: true,
          subsystems: [
            {
              subsystem: "chatSessionPersistence",
              active: true,
              cause: "database unavailable",
              firstOccurredAtMs: 1,
              backingStoreClass: "volatile",
              recoveryEligible: true,
              lastProbeResult: null,
              userVisibleImpact:
                "BACKEND CANARY: Chats are being written to private volatile storage.",
            },
          ],
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(
      await screen.findByText(
        "Recent chats and settings may not be saved after you close OOMU.",
      ),
    ).toBeVisible();
    expect(screen.queryByText(/BACKEND CANARY|private volatile storage/i)).toBeNull();
  });

  it("never puts native recovery results or failures on default glass", async () => {
    const nativeCanary =
      "BACKEND CANARY: durable recovery reconciliation cleanup code 41";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_persistence_recovery_status") {
        return {
          cleanupEligible: true,
          requiresConfirmation: false,
          lastResult: nativeCanary,
        };
      }
      if (
        command === "reconcile_volatile_persistence" ||
        command === "choose_volatile_persistence_export" ||
        command === "cleanup_reconciled_volatile_persistence"
      ) {
        throw new Error(nativeCanary);
      }
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        status={{
          active: true,
          reason: "storage unavailable",
          hasVolatileStorage: true,
          subsystems: [],
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(
      await screen.findByText(
        "Your saved work is ready. You can now remove the temporary copy.",
      ),
    ).toBeVisible();
    expect(screen.queryByText(/BACKEND CANARY|durable recovery|reconciliation/i)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Check and recover" }));
    expect(
      await screen.findByText("OOMU couldn't recover that work yet. Nothing was replaced."),
    ).toBeVisible();

    fireEvent.click(
      screen.getByRole("button", { name: "Save an encrypted recovery file" }),
    );
    expect(
      await screen.findByText(
        "The recovery file wasn't saved. Your temporary copy is still here.",
      ),
    ).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Remove the temporary copy" }));
    expect(
      await screen.findByText(
        "The temporary copy couldn't be removed. Your recovered work is still safe.",
      ),
    ).toBeVisible();
    expect(screen.queryByText(nativeCanary)).toBeNull();
  });

  it("keeps the explicit recovery choice visible after a dev restart", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_persistence_recovery_status") {
        return {
          cleanupEligible: false,
          requiresConfirmation: true,
          lastResult: "native conflict detail",
        };
      }
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        status={{
          active: true,
          reason: "storage recovery pending",
          hasVolatileStorage: true,
          subsystems: [],
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(
      await screen.findByRole("button", { name: "Save and replace" }),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Your recent work still needs to be checked before the temporary copy can be removed.",
      ),
    ).toBeVisible();
    expect(screen.queryByText("native conflict detail")).toBeNull();
  });

  it("leaves recovery immediately after verified cleanup instead of waiting for navigation", async () => {
    const onStatusChange = vi.fn();
    const healthyStatus: DegradedModeStatus = {
      active: false,
      reason: null,
      hasVolatileStorage: false,
      subsystems: [],
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_persistence_recovery_status") {
        return {
          cleanupEligible: true,
          requiresConfirmation: false,
          lastResult: "verified",
        };
      }
      if (command === "cleanup_reconciled_volatile_persistence") return null;
      if (command === "get_degraded_mode_status") return healthyStatus;
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        onStatusChange={onStatusChange}
        status={{
          active: true,
          reason: "storage recovery pending",
          hasVolatileStorage: true,
          subsystems: [],
        }}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "Remove the temporary copy" }),
    );

    await waitFor(() => expect(onStatusChange).toHaveBeenCalledWith(healthyStatus));
    expect(invokeMock).toHaveBeenCalledWith("get_degraded_mode_status");
  });

  it("localizes model-folder and model-recovery failures", async () => {
    const nativeCanary = "BACKEND CANARY: local-inference worker detail";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_local_models") return READY_MODELS;
      if (
        command === "choose_local_model_directory" ||
        command === "recover_local_inference"
      ) {
        throw new Error(nativeCanary);
      }
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={vi.fn()}
        onOpenSettings={vi.fn()}
        status={INFERENCE_DEGRADED_STATUS}
      />,
      { wrapper: I18nProvider },
    );

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Use selected model" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose model folder..." }));
    expect(await screen.findByText("Couldn't open the folder picker.")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Use selected model" }));
    expect(
      await screen.findByText("OOMU couldn't restart its on-device model."),
    ).toBeVisible();
    expect(screen.queryByText(nativeCanary)).toBeNull();
  });

  it("always provides a direct way to continue using OOMU", async () => {
    const onContinue = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_persistence_recovery_status") return null;
      return null;
    });

    render(
      <DegradedModeLanding
        onContinue={onContinue}
        onOpenSettings={vi.fn()}
        status={{
          active: true,
          reason: "migration recovery required",
          hasVolatileStorage: true,
          subsystems: [],
        }}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(await screen.findByRole("button", { name: "Continue" }));
    expect(onContinue).toHaveBeenCalledOnce();
  });
});
