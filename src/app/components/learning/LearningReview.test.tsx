import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApprovalProvider, useApprovalDialogTurn } from "@/context/ApprovalContext";
import { I18nProvider } from "@/context/I18nContext";
import { LearningReview, sanitizePermissionPromptText } from "./LearningReview";

const invokeMock = vi.hoisted(() => vi.fn());
const offersMock = vi.hoisted(() => vi.fn());
const methodsMock = vi.hoisted(() => vi.fn());
const reviewMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock, isTauriRuntime: false }));
vi.mock("./learningClient", () => ({
  learningApi: {
    edit: vi.fn(),
    forget: vi.fn(),
    goBack: vi.fn(),
    methods: methodsMock,
    offers: offersMock,
    prepare: vi.fn(),
    review: reviewMock,
    setEnabled: vi.fn(),
    undo: vi.fn(),
  },
}));

const offer = {
  conflictSummary: "",
  createdAtMs: 1,
  evidenceCount: 2,
  exposureSummary: "Seen twice",
  offerId: "offer-1",
  projectId: "project-1",
  sourceTaskCount: 2,
  status: "proposed" as const,
  summary: "# Weekly method\n- Review [the report](https://example.test)\n```private code```",
  taskRunId: "task-1",
};

function Wrapper({ children }: { children: React.ReactNode }) {
  return (
    <I18nProvider>
      <ApprovalProvider>{children}</ApprovalProvider>
    </I18nProvider>
  );
}

function QueueBlocker({ onRelease }: { onRelease: () => void }) {
  useApprovalDialogTurn(true, "earlier-permission");
  return <button onClick={onRelease} type="button">Release queued permission</button>;
}

describe("LearningReview permission confirmation", () => {
  afterEach(cleanup);

  beforeEach(() => {
    invokeMock.mockReset();
    offersMock.mockReset();
    methodsMock.mockReset();
    reviewMock.mockReset();
    invokeMock.mockResolvedValue({
      activeLocale: "en-US",
      availableLocales: [],
      translations: {},
    });
    offersMock.mockResolvedValue([offer]);
    methodsMock.mockResolvedValue([]);
    reviewMock.mockResolvedValue(null);
  });

  it("uses the shared approval dialog with clean, plain-language copy", async () => {
    const confirmSpy = vi.spyOn(window, "confirm");
    render(
      <LearningReview completed projectId="project-1" taskRunId="task-1" />,
      { wrapper: Wrapper },
    );

    const trigger = await screen.findByRole("button", { name: "Use Everywhere" });
    trigger.focus();
    fireEvent.click(trigger);

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: "Use this method in every project?" })).toBeVisible();
    expect(within(dialog).getByText("Weekly method Review the report")).toBeVisible();
    expect(dialog.textContent).not.toContain("private code");
    expect(dialog.textContent).not.toContain("```");
    expect(dialog.textContent).not.toContain("https://");
    expect(within(dialog).getByRole("button", { name: "Not Now" })).toHaveFocus();
    expect(confirmSpy).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: "Use Everywhere" }));
    await waitFor(() =>
      expect(reviewMock).toHaveBeenCalledWith(
        "offer-1",
        "remember_everywhere",
        undefined,
        true,
      ),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    confirmSpy.mockRestore();
  });

  it("cancels on Escape and restores focus without saving", async () => {
    render(
      <LearningReview completed projectId="project-1" taskRunId="task-1" />,
      { wrapper: Wrapper },
    );
    const trigger = await screen.findByRole("button", { name: "Use Everywhere" });
    trigger.focus();
    fireEvent.click(trigger);
    fireEvent.keyDown(await screen.findByRole("dialog"), { key: "Escape" });

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(reviewMock).not.toHaveBeenCalled();
  });

  it("waits its turn behind an earlier permission prompt", async () => {
    function Harness() {
      const [blocked, setBlocked] = useState(true);
      return (
        <>
          {blocked ? <QueueBlocker onRelease={() => setBlocked(false)} /> : null}
          <LearningReview completed projectId="project-1" taskRunId="task-1" />
        </>
      );
    }
    render(<Harness />, { wrapper: Wrapper });

    fireEvent.click(await screen.findByRole("button", { name: "Use Everywhere" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Release queued permission" }));
    expect(await screen.findByRole("dialog")).toBeVisible();
  });

  it("bounds and sanitizes prompt-only method text", () => {
    const value = sanitizePermissionPromptText(
      `> **Plan**\u0000\n1. Use [the result](https://example.test) ${"x".repeat(300)}`,
      80,
    );
    expect(value).toHaveLength(80);
    expect(value).toMatch(/^Plan Use the result/);
    expect(value.endsWith("…")).toBe(true);
    expect(value).not.toMatch(/[>*\u0000]|https:\/\//);
  });
});
