import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { RecommendedLocalModelSetup } from "./RecommendedLocalModelSetup";

const invokeMock = vi.hoisted(() => vi.fn());
const listeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return () => listeners.delete(name);
  }),
}));

const modelId = "gemma-4-E2B-it-qat-q4_0-gguf";
const totalBytes = 4_336_349_920;

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [{
      id: "en-US",
      label: "English (US)",
      fileName: "en-US.json",
      isDefault: true,
      verified: true,
    }],
    translations: {},
  };
}

function initialInstallState() {
  return {
    manifest: { modelId, displayName: "Gemma 4 E2B IT QAT Q4_0 GGUF", totalBytes },
    location: {
      kind: "managed",
      displayPath: "~/Library/Application Support/OOMU/models",
    },
    packageState: "absent",
    activeInstall: null,
    receipt: null,
  };
}

function renderSetup(props: React.ComponentProps<typeof RecommendedLocalModelSetup> = {}) {
  return render(<RecommendedLocalModelSetup {...props} />, { wrapper: I18nProvider });
}

beforeEach(() => {
  listeners.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_locale_state") return localeState();
    if (command === "get_recommended_model_install_state") return initialInstallState();
    return null;
  });
});

afterEach(cleanup);

describe("RecommendedLocalModelSetup", () => {
  it("removes a progress listener that resolves after the setup card unmounts", async () => {
    const view = renderSetup();
    view.unmount();
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(listeners.has("recommended-model-install-progress")).toBe(false);
  });

  it("presents the exact approved model with a managed default and one primary action", async () => {
    renderSetup();

    expect(screen.getByRole("heading", {
      name: "Gemma 4 E2B IT QAT Q4_0 GGUF",
    })).toBeVisible();
    expect(screen.getByText("The on-device model OOMU is optimized for.")).toBeVisible();
    expect(await screen.findByText("OOMU Models · Managed by OOMU")).toBeVisible();
    expect(await screen.findByRole("button", { name: "Download and continue" })).toBeEnabled();
    expect(invokeMock.mock.calls.some(([command]) =>
      command === "choose_recommended_model_install_location")).toBe(false);
  });

  it("keeps one native install in flight when the primary action is pressed twice", async () => {
    let resolveStart: ((value: unknown) => void) | undefined;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_recommended_model_install_state") return initialInstallState();
      if (command === "start_recommended_model_install") {
        return new Promise((resolve) => { resolveStart = resolve; });
      }
      return null;
    });
    renderSetup();
    const start = await screen.findByRole("button", { name: "Download and continue" });
    fireEvent.click(start);
    fireEvent.click(start);

    expect(invokeMock.mock.calls.filter(([command]) =>
      command === "start_recommended_model_install")).toHaveLength(1);
    await act(async () => {
      resolveStart?.({ progress: { installId: "install-1", state: "downloading" } });
    });
  });

  it("renders emitted byte progress exactly and never traps a user during download", async () => {
    const onDefer = vi.fn();
    renderSetup({ onDefer });
    await waitFor(() => expect(listeners.has("recommended-model-install-progress")).toBe(true));

    await act(async () => {
      listeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-1",
          state: "downloading",
          downloadedBytes: 2_180_000_000,
          totalBytes,
          canCancel: true,
          canResume: false,
        },
      });
    });

    const progress = screen.getByRole("progressbar");
    expect(progress).toHaveAttribute("aria-valuenow", "2180000000");
    expect(progress).toHaveAttribute("aria-valuemax", String(totalBytes));
    expect(screen.getByText("2.18 GB of 4.34 GB")).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    const later = screen.getByRole("button", { name: "Set up later" });
    expect(later).toBeEnabled();
    fireEvent.click(later);
    expect(onDefer).toHaveBeenCalledTimes(1);
  });

  it("uses only a native location grant and accepts verified exact-model evidence", async () => {
    const onVerified = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_recommended_model_install_state") return initialInstallState();
      if (command === "choose_recommended_model_install_location") {
        return { locationGrantId: "grant-1", displayPath: "~/Models" };
      }
      return null;
    });
    renderSetup({ onVerified });
    fireEvent.click(await screen.findByRole("button", { name: "Change location…" }));
    expect(invokeMock).toHaveBeenCalledWith(
      "choose_recommended_model_install_location",
      { dialogTitle: "Change location…" },
    );
    expect(await screen.findByText("OOMU Models · ~/Models")).toBeVisible();

    await act(async () => {
      listeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-1",
          state: "ready",
          downloadedBytes: totalBytes,
          totalBytes,
          canCancel: false,
          canResume: false,
          completedProvider: {
            providerId: "local-model",
            providerType: "local_model",
            modelId,
            verified: true,
          },
        },
      });
    });

    await waitFor(() => expect(onVerified).toHaveBeenCalledWith(
      expect.objectContaining({ modelId, verified: true }),
    ));
  });
});

describe("RecommendedLocalModelSetup completion recovery", () => {
  it("keeps deferral available and reports preparation while native verification finishes", async () => {
    const onDefer = vi.fn();
    const onVerified = vi.fn(() => new Promise<void>(() => undefined));
    renderSetup({ onDefer, onVerified });
    await waitFor(() => expect(listeners.has("recommended-model-install-progress")).toBe(true));

    await act(async () => {
      listeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-preparing",
          state: "ready",
          downloadedBytes: totalBytes,
          totalBytes,
          canCancel: false,
          canResume: false,
          completedProvider: {
            providerId: "local-model",
            providerType: "local_model",
            modelId,
            verified: true,
          },
        },
      });
    });

    expect(await screen.findAllByText("Preparing OOMU…")).toHaveLength(2);
    const later = screen.getByRole("button", { name: "Set up later" });
    expect(later).toBeEnabled();
    fireEvent.click(later);
    expect(onDefer).toHaveBeenCalledOnce();
  });

  it("allows any failed installer-owned partial to be removed before relocation", async () => {
    renderSetup();
    await waitFor(() => expect(listeners.has("recommended-model-install-progress")).toBe(true));

    await act(async () => {
      listeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-network-failure",
          state: "failed",
          downloadedBytes: 1_000_000,
          totalBytes,
          canCancel: false,
          canResume: true,
          publicErrorCode: "model_install_download_failed",
        },
      });
    });

    expect(screen.getByRole("button", { name: "Remove downloaded files" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Change location…" })).toBeEnabled();
  });

  it("offers a real retry when verified setup progression fails", async () => {
    const onVerified = vi.fn()
      .mockRejectedValueOnce(new Error("progress save failed"))
      .mockResolvedValueOnce(undefined);
    renderSetup({ onVerified });
    await waitFor(() => expect(listeners.has("recommended-model-install-progress")).toBe(true));

    await act(async () => {
      listeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-retry",
          state: "ready",
          downloadedBytes: totalBytes,
          totalBytes,
          canCancel: false,
          canResume: false,
          completedProvider: {
            providerId: "local-model",
            providerType: "local_model",
            modelId,
            verified: true,
          },
        },
      });
    });

    expect(await screen.findByRole("button", { name: "Use this model and continue" }))
      .toBeEnabled();
    expect(onVerified).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Use this model and continue" }));
    await waitFor(() => expect(onVerified).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(
      screen.queryByRole("button", { name: "Use this model and continue" }),
    ).toBeNull());
  });
});
