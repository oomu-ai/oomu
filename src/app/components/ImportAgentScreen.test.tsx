import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { ImportAgentScreen } from "./ImportAgentScreen";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_locale_state") {
      return {
        activeLocale: "en-US",
        availableLocales: [],
        translations: {},
      };
    }
    if (command === "choose_agent_import_directory") {
      return { grant_id: "grant-refresh", directory_name: "OOMU" };
    }
    if (command === "scan_agent_import_directory") {
      return {
        success: true,
        directory_name: "OOMU",
        scan_token: "scan-refresh",
        files: [{
          key: "journal:2026-06-03.md",
          filename: "2026-06-03.md",
          relative_path: "memory/2026-06-03.md",
          size_bytes: 42,
          modified_at_ms: 1_783_036_800_000,
          group: "chronological_journals",
          label: "Chronological Journal",
          description: "A dated memory note.",
          selected_by_default: true,
        }],
      };
    }
    if (command === "execute_agent_import") {
      return { id: "imported_oomu", name: "OOMU" };
    }
    return null;
  });
});

afterEach(cleanup);

describe("ImportAgentScreen memory refresh", () => {
  it("refreshes the exact existing agent instead of creating another agent", async () => {
    const onImportComplete = vi.fn();
    render(
      <ImportAgentScreen
        configuredProviders={[{
          id: "provider-local",
          providerId: "local_model",
          providerName: "On this Mac",
          customModelIds: "gemma-4-E4B-it-qat-q4_0-gguf",
        } as ConfiguredProvider]}
        onCancel={vi.fn()}
        onImportComplete={onImportComplete}
        refreshTarget={{
          id: "imported_oomu",
          name: "OOMU",
          description: "",
          providerId: "local_model",
          modelId: "gemma-4-E4B-it-qat-q4_0-gguf",
        }}
        templateOptions={[]}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("heading", { name: "Refresh OOMU’s memory" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Choose" }));
    fireEvent.click(await screen.findByRole("button", { name: "Refresh memory" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "execute_agent_import",
      {
        request: expect.objectContaining({
          grantId: "grant-refresh",
          scanToken: "scan-refresh",
          keysToImport: ["journal:2026-06-03.md"],
          targetAgentId: "imported_oomu",
        }),
      },
    ));
    expect(onImportComplete).toHaveBeenCalledWith({ id: "imported_oomu", name: "OOMU" });
  });
});
