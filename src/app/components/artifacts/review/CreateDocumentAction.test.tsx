import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { parseProjectId, parseTaskId, parseTaskRunId, type P0EventEnvelope } from "@/lib/p0Contracts";
import type { TaskRun } from "../../tasks/taskClient";
import { CreateDocumentAction } from "./CreateDocumentAction";

const invokeMock = vi.hoisted(() => vi.fn());
const setActiveItem = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/components/AppShell", () => ({ useAppShell: () => ({ setActiveItem }) }));

const completedTask: TaskRun = {
  taskRunId: "taskrun_55555555-5555-4555-8555-555555555555",
  taskId: "task_33333333-3333-4333-8333-333333333333",
  projectId: "project_22222222-2222-4222-8222-222222222222",
  runtimeKind: "workflow",
  runtimeRecordId: "record",
  state: "completed",
  origin: "chat",
  correlationId: "correlation",
  summary: "Quarterly sales summary",
  lastError: null,
  createdAtMs: Date.parse("2026-07-11T18:00:00Z"),
  updatedAtMs: Date.parse("2026-07-11T18:01:00Z"),
  completedAtMs: Date.parse("2026-07-11T18:01:00Z"),
  acknowledgedAtMs: null,
  recoveryState: "not_required",
  effectVerificationRequired: false,
  validControls: [],
};

function localeState() { return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} }; }

describe("CreateDocumentAction", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
    invokeMock.mockReset(); setActiveItem.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "create_artifact") return { artifactId: "artifact_44444444-4444-4444-8444-444444444444" };
      return null;
    });
  });
  afterEach(cleanup);

  it("uses the current completed Task automatically and exposes the obvious next action", async () => {
    render(<CreateDocumentAction events={[]} task={completedTask} />, { wrapper: I18nProvider });
    expect(screen.getByText("OOMU will use this Task and Project automatically.")).toBeVisible();
    expect(screen.queryByRole("combobox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Create document" }));
    expect(screen.getByRole("button", { name: /Word document and PDF/ })).toBeVisible();
    expect(screen.getByRole("button", { name: /Excel spreadsheet/ })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /Word document and PDF/ }));
    expect(await screen.findByRole("button", { name: "Open in Documents" })).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith("create_artifact", expect.objectContaining({ request: expect.objectContaining({ projectId: completedTask.projectId, taskRunId: completedTask.taskRunId }) }));
    fireEvent.click(screen.getByRole("button", { name: "Open in Documents" }));
    expect(setActiveItem).toHaveBeenCalledWith("artifacts");
    expect(window.sessionStorage.getItem("oomu.documents.focus")).toBe("word:artifact_44444444-4444-4444-8444-444444444444");
  });

  it("does not imply a usable result for incomplete Tasks", async () => {
    const { container } = render(<CreateDocumentAction events={[]} task={{ ...completedTask, state: "running" }} />, { wrapper: I18nProvider });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_locale_state"));
    expect(container).toBeEmptyDOMElement();
  });

  it("binds factual Task output to its canonical evidence reference", async () => {
    const event: P0EventEnvelope = {
      schemaVersion: 1,
      eventType: "connector.read_completed",
      projectId: parseProjectId(completedTask.projectId),
      taskId: parseTaskId(completedTask.taskId),
      taskRunId: parseTaskRunId(completedTask.taskRunId),
      correlationId: "correlation",
      sequence: 7,
      timestamp: "2026-07-11T18:00:00Z",
      evidenceClass: "verified_postcondition",
      payload: { userVisibleOutput: "Verified result" },
    };
    render(<CreateDocumentAction events={[event]} task={completedTask} />, { wrapper: I18nProvider });
    fireEvent.click(screen.getByRole("button", { name: "Create document" }));
    fireEvent.click(screen.getByRole("button", { name: /Word document and PDF/ }));
    await screen.findByRole("button", { name: "Open in Documents" });
    expect(invokeMock).toHaveBeenCalledWith("create_artifact", expect.objectContaining({ request: expect.objectContaining({ document: expect.objectContaining({ sections: [expect.objectContaining({ blocks: [expect.objectContaining({ factual: true, sources: [{ sourceRef: "connector.read_completed", evidenceRef: `task-event:${completedTask.taskRunId}:7` }] })] })] }) }) }));
  });

  it("lets the user choose a compatible PowerPoint design before creation", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "inspect_presentation_template") return {
        templateId: "presentation-template-123", name: "Board update", fingerprintSha256: "a".repeat(64),
        masterParts: ["ppt/slideMasters/slideMaster1.xml"],
        layoutParts: ["ppt/slideLayouts/slideLayout1.xml", "ppt/slideLayouts/slideLayout2.xml"],
        slideCount: 2, exactPartPreservationSupported: true, taskSummaryCompatible: true,
      };
      return null;
    });
    render(<CreateDocumentAction events={[]} task={completedTask} />, { wrapper: I18nProvider });
    fireEvent.click(screen.getByRole("button", { name: "Create document" }));
    fireEvent.click(screen.getByRole("button", { name: "Use a PowerPoint design" }));
    expect(await screen.findByText("Using Board update")).toBeVisible();
    expect(screen.getByRole("button", { name: "Choose another design" })).toBeEnabled();
    expect(invokeMock).toHaveBeenCalledWith("inspect_presentation_template", { request: {
      projectId: completedTask.projectId, taskId: completedTask.taskId, taskRunId: completedTask.taskRunId,
    } });
  });

  it("makes an agent-created spreadsheet immediately obvious and openable", () => {
    const event: P0EventEnvelope = {
      schemaVersion: 1,
      eventType: "workbook.review_ready",
      projectId: parseProjectId(completedTask.projectId),
      taskId: parseTaskId(completedTask.taskId),
      taskRunId: parseTaskRunId(completedTask.taskRunId),
      correlationId: "correlation",
      sequence: 8,
      timestamp: "2026-07-11T18:00:00Z",
      evidenceClass: "signed_artifact",
      payload: { artifactId: "artifact_44444444-4444-4444-8444-444444444444", revision: 1 },
    };
    render(<CreateDocumentAction events={[event]} task={completedTask} />, { wrapper: I18nProvider });
    expect(screen.getByText("Your document is ready")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Open in Documents" }));
    expect(setActiveItem).toHaveBeenCalledWith("artifacts");
    expect(window.sessionStorage.getItem("oomu.documents.focus")).toBe("spreadsheet:artifact_44444444-4444-4444-8444-444444444444");
  });
});
