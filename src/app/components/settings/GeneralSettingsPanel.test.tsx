import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GeneralSettingsPanel } from "./GeneralSettingsPanel";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/components/ThemeProvider", () => ({
  useTheme: () => ({ theme: "system", setTheme: vi.fn(), resolvedTheme: "light" }),
}));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    language: "en-US",
    availableLocales: [{ id: "en-US", label: "English (US)" }],
    isLoadingLocales: false,
    isChangingLanguage: false,
    localeError: null,
    setLanguage: vi.fn(),
    t: (key: string) =>
      ({
        "settings.general.model_directory.multiple_models_error":
          "Put each model in its own folder. This folder has more than one model file (.gguf). Create one folder for each model, move one .gguf file into each folder, then check again.",
        "settings.general.model_directory.check_again": "Check again",
        "settings.general.default_prewarmed_model.fix_models_folder":
          "Fix the models folder above to see your models.",
      })[key] ?? key,
  }),
}));
vi.mock("../artifacts/presentations/PresentationCheckerSetup", () => ({
  PresentationCheckerSetup: () => null,
}));

const modelId = "gemma-4-E2B-it-qat-q4_0-gguf";

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(cleanup);

describe("GeneralSettingsPanel model folder recovery", () => {
  it("explains the folder-per-model rule and recovers without claiming the saved model is missing", async () => {
    let folderFixed = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_local_model_directory") {
        return Promise.resolve({ path: "/tmp/models", isDefault: false });
      }
      if (command === "get_default_prewarmed_model") {
        return Promise.resolve({ modelId, isDefault: false });
      }
      if (command === "list_local_models") {
        return folderFixed
          ? Promise.resolve([{ id: modelId, name: "Gemma 4 E2B", compatibility: "ready" }])
          : Promise.reject({
              code: "local_model_primary_gguf_ambiguous",
              message: "technical inventory detail",
            });
      }
      return Promise.resolve(null);
    });

    render(<GeneralSettingsPanel />);

    expect(await screen.findByText(/Put each model in its own folder/)).toBeVisible();
    expect(screen.queryByText(/not found in this directory/i)).toBeNull();
    expect(screen.getByLabelText("settings.general.default_prewarmed_model.select_label"))
      .toBeDisabled();

    folderFixed = true;
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));

    await waitFor(() =>
      expect(screen.queryByText(/Put each model in its own folder/)).toBeNull(),
    );
    expect(screen.getByLabelText("settings.general.default_prewarmed_model.select_label"))
      .toBeEnabled();
    expect(screen.getByRole("option", { name: "Gemma 4 E2B" })).toBeVisible();
  });
});
