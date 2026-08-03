import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ArtifactStudio } from "./ArtifactStudio";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const rawReview = {
  artifactId: "artifact_44444444-4444-4444-8444-444444444444",
  projectId: "project_22222222-2222-4222-8222-222222222222",
  taskId: "task_33333333-3333-4333-8333-333333333333",
  taskRunId: "taskrun_55555555-5555-4555-8555-555555555555",
  title: "Quarterly operating review",
  currentRevision: 3,
  selectedSheetId: "summary",
  previewAvailable: true,
  safePriorRevision: 2,
  createdAtMs: 1,
  updatedAtMs: 2,
  revisions: [{
    revision: 3,
    statusCode: "needs_recalculation",
    createdAtMs: 2,
    completedAtMs: 3,
    sheets: [{ sheetId: "summary", name: "Summary", previewAvailable: true }],
    formulaCells: [{ sheetId: "summary", address: "B2", expression: "=SUM(B3:B8)", displayValue: "$12", statusCode: "needs_recalculation" }],
    lineage: [{ sheetId: "summary", address: "B2", sourceRef: "Quarterly actuals", evidenceRef: "evidence-1" }],
    warnings: [{ code: "needs_recalculation", location: { sheetId: "summary", range: "B2" }, technicalDetail: "BACKEND WARNING CANARY" }],
    numbersStatusCode: "needs_recalculation",
    exportable: false,
    evidenceSummary: [{ code: "formula_check", passed: false, evidence: "BACKEND EVIDENCE CANARY" }],
    technicalEvidenceAvailable: true,
    recoverable: true,
    lastErrorCode: null,
  }, {
    revision: 2,
    statusCode: "ready",
    createdAtMs: 1,
    completedAtMs: 2,
    sheets: [{ sheetId: "summary", name: "Summary", previewAvailable: true }],
    formulaCells: [{ sheetId: "summary", address: "B2", expression: "=SUM(B3:B8)", displayValue: "$12", statusCode: "up_to_date" }],
    lineage: [],
    warnings: [],
    numbersStatusCode: "up_to_date",
    exportable: true,
    evidenceSummary: [{ code: "formula_check", passed: true, evidence: "checked" }],
    technicalEvidenceAvailable: true,
    recoverable: true,
    lastErrorCode: null,
  }],
};

let reviseFailure: unknown = null;
let activeRawReview = rawReview;
let documentRecords: unknown[] = [];
let presentationRecords: unknown[] = [];

const otherDocument = {
  artifactId: "artifact_99999999-9999-4999-8999-999999999999",
  projectId: rawReview.projectId,
  taskRunId: rawReview.taskRunId,
  title: "Z different document",
  currentVersion: 1,
  createdAtMs: 1,
  updatedAtMs: 1,
  versions: [{
    version: 1,
    revisionInstruction: null,
    status: "verified",
    document: { schemaVersion: 1, title: "Z different document", sections: [{ heading: "Summary", paragraphs: ["Other"] }] },
    previewPages: [],
    verification: { structurallyVerifiedDocx: true, structurallyVerifiedPdf: true, visuallyVerifiedPdf: true, pageCount: 0, warnings: [], rendererProbe: "verified" },
    provenance: null,
    docxBytes: 1,
    pdfBytes: 1,
    docxSha256: "b".repeat(64),
    pdfSha256: "c".repeat(64),
    builderIdentity: "builder",
    rendererIdentity: "renderer",
    createdAtMs: 1,
    completedAtMs: 1,
    lastError: null,
  }],
};

function localeState() {
  return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} };
}

describe("Documents library", () => {
  beforeEach(() => {
    reviseFailure = null;
    activeRawReview = rawReview;
    documentRecords = [];
    presentationRecords = [];
    window.sessionStorage.clear();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string, payload?: { request?: { revision?: number } }) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return [];
      if (command === "list_artifacts") return documentRecords;
      if (command === "list_workbook_reviews") return [{ artifactId: rawReview.artifactId }];
      if (command === "list_presentation_reviews") return presentationRecords;
      if (command === "get_workbook_review") return activeRawReview;
      if (command === "get_workbook_preview") return { artifactId: rawReview.artifactId, revision: payload?.request?.revision ?? 3, sheetId: "summary", mimeType: "image/png", dataUrl: "data:image/png;base64,iVBORw0KGgo=", width: 1440, height: 900, sha256: "a".repeat(64) };
      if (command === "revise_workbook_range" && reviseFailure) throw reviseFailure;
      return null;
    });
  });
  afterEach(cleanup);

  it("normalizes a spreadsheet into the preview-first shared library without ownership selectors or raw codes", async () => {
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    expect(await screen.findByRole("heading", { name: "Quarterly operating review" })).toBeVisible();
    expect(screen.getByText("The documents OOMU writes for you — briefs, spreadsheets, and decks — ready to review and export.")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Documents" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Documents OOMU made" })).toBeVisible();
    expect(await screen.findByAltText("Preview of Summary, version 3")).toBeVisible();
    expect(screen.getByText("Some numbers need recalculating")).toBeVisible();
    expect(screen.getByRole("button", { name: "Export spreadsheet" })).toBeDisabled();
    expect(screen.queryByText("needs_recalculation")).toBeNull();
    expect(screen.queryByText("BACKEND WARNING CANARY")).toBeNull();
    expect(screen.queryByText("BACKEND EVIDENCE CANARY")).toBeNull();
    expect(screen.queryByText("Quarterly actuals")).toBeNull();
    expect(screen.queryByLabelText("Task")).toBeNull();
    expect(screen.getByText("Numbers and calculations")).not.toBeVisible();

    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("Recorded source")).toBeVisible();
    expect(screen.queryByText("Quarterly actuals")).toBeNull();
    expect(screen.getByText("Numbers and calculations")).toBeVisible();
    expect(screen.getByRole("button", { name: "Choose specific cells" })).toBeVisible();
    expect(screen.queryByLabelText("Which cells?")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Choose specific cells" }));
    expect(screen.getByLabelText("Which cells?")).toBeVisible();
  });

  it("explains where documents come from and opens Chat from the empty state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects" || command === "list_artifacts" || command === "list_workbook_reviews" || command === "list_presentation_reviews") return [];
      return null;
    });
    const onStartInChat = vi.fn();
    render(<ArtifactStudio onStartInChat={onStartInChat} />, { wrapper: I18nProvider });

    expect(await screen.findByText("The documents OOMU writes for you — briefs, spreadsheets, decks — show up here. Ask OOMU in Chat to create one.")).toBeVisible();
    const goToChat = screen.getByRole("button", { name: "Go to Chat" });
    fireEvent.click(goToChat);
    expect(onStartInChat).toHaveBeenCalledTimes(1);
  });

  it("opens the relevant sheet and cell from a plain warning", async () => {
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    const jump = await screen.findByRole("button", { name: "Show Summary, B2" });
    fireEvent.click(jump);
    await waitFor(() => expect(screen.getByLabelText("Which cells?")).toHaveValue("B2"));
    expect(screen.getByText("Numbers and calculations")).toBeVisible();
  });

  it("opens and exports a recoverable prior version without changing the latest version", async () => {
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    await screen.findByAltText("Preview of Summary, version 3");
    fireEvent.click(screen.getByText("Details"));
    fireEvent.click(screen.getByRole("button", { name: "Open this version" }));
    expect(await screen.findByAltText("Preview of Summary, version 2")).toBeVisible();
    expect(screen.getByRole("button", { name: "Export spreadsheet" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Back to latest" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }));
    expect(await screen.findByAltText("Preview of Summary, version 3")).toBeVisible();
  });

  it("reveals and focuses the cell target when the backend needs a less ambiguous change", async () => {
    reviseFailure = { code: "workbook_revision_target_ambiguous", message: "BACKEND ERROR CANARY" };
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    await screen.findByAltText("Preview of Summary, version 3");
    fireEvent.click(screen.getByText("Details"));
    fireEvent.change(screen.getByLabelText("What should change?"), { target: { value: "Make that total larger" } });
    fireEvent.click(screen.getByRole("button", { name: "Save as a new version" }));
    const target = await screen.findByLabelText("Which cells?");
    await waitFor(() => expect(target).toHaveFocus());
    expect(screen.getByRole("alert")).toHaveTextContent("Choose the cells this change should affect, then try again.");
    expect(screen.queryByText("BACKEND ERROR CANARY")).toBeNull();
  });

  it("offers the safe prior version as the obvious next action when the latest version failed", async () => {
    activeRawReview = structuredClone(rawReview);
    activeRawReview.revisions[0].statusCode = "failed";
    activeRawReview.revisions[0].numbersStatusCode = "not_applicable";
    activeRawReview.revisions[0].exportable = false;
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    await screen.findByAltText("Preview of Summary, version 3");
    expect(screen.getByRole("button", { name: "Go back to the last good version" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Go back to the last good version" }));
    expect(await screen.findByAltText("Preview of Summary, version 2")).toBeVisible();
    expect(screen.getByRole("button", { name: "Export spreadsheet" })).toBeEnabled();
  });

  it("consumes the Open handoff and selects the exact newly created record", async () => {
    documentRecords = [otherDocument];
    window.sessionStorage.setItem("oomu.documents.focus", `spreadsheet:${rawReview.artifactId}`);
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    expect(await screen.findByRole("heading", { name: "Quarterly operating review" })).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Z different document" })).toBeNull();
    expect(window.sessionStorage.getItem("oomu.documents.focus")).toBeNull();
  });

  it("includes presentations in the same preview-first Documents library", async () => {
    presentationRecords = [{
      presentationId: "presentation_77777777-7777-4777-8777-777777777777",
      projectId: rawReview.projectId,
      taskId: rawReview.taskId,
      taskRunId: rawReview.taskRunId,
      artifactId: "artifact_88888888-8888-4888-8888-888888888888",
      title: "Z board presentation",
      currentRevision: 1,
      status: "check_required",
      slideCount: 4,
      issueCount: 1,
      blockerCount: 1,
      structurallyVerified: true,
      visuallyVerified: false,
      exportable: false,
      updatedAtMs: 3,
    }];
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    expect(await screen.findByText("Z board presentation")).toBeVisible();
    expect(screen.getByText("PowerPoint presentation")).toBeVisible();
  });

  it("offers a verified Word version as the obvious recovery when the latest version failed", async () => {
    const verified = structuredClone(otherDocument.versions[0]);
    verified.version = 1;
    const failed = structuredClone(verified);
    failed.version = 2;
    failed.status = "failed";
    failed.verification.visuallyVerifiedPdf = false;
    documentRecords = [{ ...otherDocument, currentVersion: 2, versions: [failed, verified] }];
    activeRawReview = { ...rawReview, title: "A workbook" };
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    expect(await screen.findByRole("heading", { name: "Z different document" })).toBeVisible();
    expect(screen.getByText("This document needs attention before export.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Go back to the last good version" })).toBeVisible();
    expect(screen.getByText("Version history")).not.toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Go back to the last good version" }));
    expect(await screen.findByText("Version 1")).toBeVisible();
    expect(screen.getByRole("button", { name: "Export" })).toBeEnabled();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("Version history")).toBeVisible();
    expect(screen.getByText("What OOMU checked")).toBeVisible();
    expect(screen.getByText("OOMU checked that every page looks right.")).toBeVisible();
    expect(screen.getByText("OOMU also saves proof it didn't change the file, so you can verify it later.")).toBeVisible();
  });

  it("warns when one document family fails instead of silently presenting an incomplete library", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return [];
      if (command === "list_artifacts") throw new Error("document list failed");
      if (command === "list_workbook_reviews") return [{ artifactId: rawReview.artifactId }];
      if (command === "list_presentation_reviews") return [];
      if (command === "get_workbook_review") return rawReview;
      if (command === "get_workbook_preview") return { artifactId: rawReview.artifactId, revision: 3, sheetId: "summary", mimeType: "image/png", dataUrl: "data:image/png;base64,iVBORw0KGgo=", width: 1440, height: 900, sha256: "a".repeat(64) };
      return null;
    });
    render(<ArtifactStudio />, { wrapper: I18nProvider });
    expect(await screen.findByRole("alert")).toHaveTextContent("Some documents could not be loaded. Refresh to try again.");
    expect(await screen.findByRole("heading", { name: "Quarterly operating review" })).toBeVisible();
  });
});
