import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { PresentationCheckerSetup } from "./PresentationCheckerSetup";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

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

describe("presentation checker setup", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_checker_readiness") {
        return {
          status: "not_installed",
          requiredVersion: "26.2.5 (build 26.2.5.2)",
        };
      }
      return null;
    });
  });

  afterEach(cleanup);

  it("explains the exact supported build and opens only the fixed native action", async () => {
    render(<PresentationCheckerSetup />, { wrapper: I18nProvider });

    expect(await screen.findByText("One-time setup needed")).toBeVisible();
    const download = screen.getByRole("button", {
      name: "Open LibreOffice 26.2.5 (build 26.2.5.2) download",
    });
    fireEvent.click(download);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "open_presentation_checker_download",
      ),
    );
    expect(invokeMock.mock.calls.some(([, payload]) => payload?.url)).toBe(false);
  });

  it("lets the user recheck readiness after installing the supported build", async () => {
    let checks = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_checker_readiness") {
        checks += 1;
        return checks === 1
          ? { status: "not_qualified", requiredVersion: "26.2.5 (build 26.2.5.2)" }
          : { status: "ready", requiredVersion: "26.2.5 (build 26.2.5.2)" };
      }
      return null;
    });
    render(<PresentationCheckerSetup />, { wrapper: I18nProvider });

    expect(await screen.findByText("LibreOffice update needed")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("You're all set — OOMU can check and export PowerPoint presentations and Excel workbooks on this Mac.")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Open LibreOffice/ })).toBeNull();
  });
});
