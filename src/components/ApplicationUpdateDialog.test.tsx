import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApplicationUpdateDialog } from "./ApplicationUpdateDialog";

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string, values?: Record<string, string | number>) => ({
      "application_updates.available_title": "An OOMU Update Is Available",
      "application_updates.available_body": `OOMU ${values?.availableVersion} is ready. You’re using ${values?.currentVersion}.`,
      "application_updates.whats_new": "What’s new",
      "application_updates.actions.full_notes": "Read full release notes",
      "application_updates.actions.install": "Install Update",
      "application_updates.actions.remind": "Remind Me Later",
      "application_updates.actions.skip": "Skip This Update",
      "application_updates.downloading": "Downloading",
      "application_updates.downloading_body": "Downloading safely.",
      "application_updates.progress": `${values?.downloaded} of ${values?.total}`,
      "application_updates.progress_label": "Application update download progress",
    })[key] ?? key,
  }),
}));

const handlers = {
  onCheck: vi.fn(), onDismiss: vi.fn(), onInstall: vi.fn(), onOpenFullNotes: vi.fn(),
  onRemind: vi.fn(), onRestart: vi.fn(), onSkip: vi.fn(),
};

beforeEach(() => vi.clearAllMocks());
afterEach(() => cleanup());

describe("ApplicationUpdateDialog", () => {
  it("offers one dominant install action and renders remote notes as inert text", () => {
    render(<ApplicationUpdateDialog {...handlers} view={{
      status: "update_available",
      currentVersion: "0.1.2",
      availableVersion: "0.1.3",
      notes: "Safer updates. <script>alert(1)</script>",
      fullNotesAvailable: true,
    }} />);
    expect(screen.getByRole("dialog")).toHaveTextContent("Safer updates");
    expect(document.querySelector("script")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Install Update" }));
    expect(handlers.onInstall).toHaveBeenCalledTimes(1);
  });

  it("treats Escape from an available update as remind later", () => {
    render(<ApplicationUpdateDialog {...handlers} view={{
      status: "update_available", currentVersion: "0.1.2", availableVersion: "0.1.3",
    }} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(handlers.onRemind).toHaveBeenCalledTimes(1);
  });

  it("shows measured download progress", () => {
    render(<ApplicationUpdateDialog {...handlers} view={{
      status: "downloading", currentVersion: "0.1.2", downloadedBytes: 25, totalBytes: 100,
    }} />);
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "25");
  });
});
