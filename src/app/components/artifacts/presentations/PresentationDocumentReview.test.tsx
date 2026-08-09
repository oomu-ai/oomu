import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { createTaskSummaryPresentation } from "@/lib/artifacts/presentations/schema";
import { PresentationDocumentReview } from "./PresentationDocumentReview";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const presentation = createTaskSummaryPresentation({
  title: "Quarterly review",
  summary: "Revenue grew while support volume fell.",
  locale: "en-US",
  coverLabel: "Project brief",
  findingsTitle: "What OOMU found",
  sources: [{ sourceRef: "Quarterly actuals", evidenceRef: "task-event:taskrun_55555555-5555-4555-8555-555555555555:4" }],
});

const summary = {
  presentationId: "presentation_44444444-4444-4444-8444-444444444444",
  projectId: "project_22222222-2222-4222-8222-222222222222",
  taskId: "task_33333333-3333-4333-8333-333333333333",
  taskRunId: "taskrun_55555555-5555-4555-8555-555555555555",
  artifactId: "artifact_66666666-6666-4666-8666-666666666666",
  title: "Quarterly review",
  currentRevision: 1,
  status: "check_required" as const,
  slideCount: 2,
  issueCount: 1,
  blockerCount: 1,
  structurallyVerified: true,
  visuallyVerified: false,
  exportable: false,
  updatedAtMs: 1,
};

const issue = {
  issueId: "issue-1", revision: 1, slideId: "cover", code: "text_overflow", severity: "blocker",
  message: "BACKEND TECHNICAL CANARY", objectId: "cover_title", evidenceRef: null,
};

const detail = {
  summary,
  selectedRevision: 1,
  presentation,
  revisionHistory: [{ revision: 1, createdAtMs: 1, scope: "whole_presentation", changeSummary: "Created", structurallyVerified: true, visuallyVerified: false, exportable: false }],
  filmstrip: presentation.slides.map((slide, position) => ({ slideId: slide.slideId, position, title: slide.title ?? "", layoutId: slide.layoutId, thumbnail: { mediaType: "image/png" as const, bytesBase64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=", width: 1, height: 1 }, issueCount: position === 0 ? 1 : 0, blockerCount: position === 0 ? 1 : 0 })),
  issues: [issue],
  notes: presentation.slides.map((slide) => ({ slideId: slide.slideId, speakerNotes: slide.notes.speakerNotes, sourceRefs: slide.notes.sourceRefs })),
  citations: presentation.citations.map((citation) => ({ ...citation })),
  provenance: [{ slideId: "findings", objectId: "findings_body", sourceRef: "Quarterly actuals", evidenceRef: presentation.citations[0].evidenceRef, evidenceClass: "verified_postcondition" }],
  templateIdentity: { templateId: null, name: "OOMU Light", imported: false, fingerprintSha256: "", masterIds: ["oomu_master"], layoutIds: ["cover_layout", "content_layout"] },
  verification: { packageSha256: "a".repeat(64), structurallyVerified: true, visuallyVerified: false, exportable: false, checkedAtMs: 1, renderer: null, checks: [{ code: "package_structure_valid", passed: true, detail: "BACKEND CHECK CANARY", slideId: null, objectId: null }, { code: "semantic_checks_completed", passed: true, detail: "RAW SEMANTIC CANARY", slideId: null, objectId: null }, { code: "exact_package_pages_rendered", passed: true, detail: "RAW PACKAGE CANARY", slideId: null, objectId: null }], issues: [issue] },
};

function localeState() {
  return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} };
}

describe("presentation review", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_review") return detail;
      if (command === "get_presentation_preview") return { presentationId: summary.presentationId, revision: 1, filmstrip: detail.filmstrip, issues: detail.issues, rendererUnavailable: false };
      if (command === "revise_presentation_scope") return { ...detail, summary: { ...summary, currentRevision: 2 }, selectedRevision: 2, presentation: { ...presentation, revision: 2 } };
      return null;
    });
  });
  afterEach(cleanup);

  it("keeps export unavailable until every slide passes and maps issues into plain language", async () => {
    render(<PresentationDocumentReview onRefresh={vi.fn()} summary={summary} />, { wrapper: I18nProvider });
    expect(await screen.findByRole("heading", { name: "Quarterly review" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Export presentation" })).toBeDisabled();
    expect(screen.getByText("Some text does not fit in its frame.")).toBeVisible();
    expect(screen.queryByText("BACKEND TECHNICAL CANARY")).toBeNull();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_presentation_preview", { request: { presentationId: summary.presentationId, revision: 1 } }));
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText((_, element) => element?.tagName === "LI" && element.textContent?.includes("The presentation file is complete and safe to open") === true)).toBeVisible();
    expect(screen.getByText((_, element) => element?.tagName === "LI" && element.textContent?.includes("Every slide was checked from the actual PowerPoint file") === true)).toBeVisible();
    expect(screen.getByText((_, element) => element?.tagName === "LI" && element.textContent?.includes("Slide content passed OOMU's layout checks") === true)).toBeVisible();
    expect(screen.getByText(/Used on What OOMU found/)).toBeVisible();
    expect(screen.queryByText("BACKEND CHECK CANARY")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show slide" }));
    expect(screen.getByRole("img", { name: "What needs attention" })).toBeVisible();
  });

  it("opens contextual setup and rechecks the blocked deck as a new verified attempt", async () => {
    const openSetup = vi.fn();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const checkerIssue = {
      issueId: "checker-1",
      revision: 1,
      slideId: null,
      code: "exact_package_preview_unavailable",
      severity: "blocker" as const,
      message: "RAW CHECKER CANARY",
      objectId: null,
      evidenceRef: null,
    };
    const checkerDetail = {
      ...detail,
      issues: [checkerIssue],
      verification: {
        ...detail.verification,
        checks: detail.verification.checks.map((check) =>
          check.code === "exact_package_pages_rendered"
            ? { ...check, passed: false }
            : check,
        ),
        issues: [checkerIssue],
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_review") return checkerDetail;
      if (command === "get_presentation_preview") {
        return {
          presentationId: summary.presentationId,
          revision: 1,
          filmstrip: detail.filmstrip,
          issues: [checkerIssue],
          rendererUnavailable: true,
        };
      }
      if (command === "recheck_presentation_revision") return checkerDetail;
      return null;
    });

    render(
      <PresentationDocumentReview
        onOpenSetup={openSetup}
        onRefresh={refresh}
        summary={summary}
      />,
      { wrapper: I18nProvider },
    );
    expect(await screen.findByText("Office file export needs LibreOffice on this Mac.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Set up Office export" }));
    expect(openSetup).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Check presentation again" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("recheck_presentation_revision", {
        request: {
          presentationId: summary.presentationId,
          expectedRevision: 1,
        },
      }),
    );
    expect(refresh).toHaveBeenCalledOnce();
    expect(screen.queryByText("RAW CHECKER CANARY")).toBeNull();
  });

  it("revises only the selected slide and preserves a new immutable version", async () => {
    const refresh = vi.fn().mockResolvedValue(undefined);
    render(<PresentationDocumentReview onRefresh={refresh} summary={summary} />, { wrapper: I18nProvider });
    await screen.findByRole("heading", { name: "Quarterly review" });
    fireEvent.click(screen.getByRole("button", { name: "Edit slide" }));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Updated quarterly review" } });
    fireEvent.click(screen.getByRole("button", { name: "Save as a new version" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("revise_presentation_scope", expect.objectContaining({ request: expect.objectContaining({ scope: "element", targetSlideIds: ["cover"], targetObjectIds: ["cover_title"], expectedRevision: 1 }) })));
    const call = invokeMock.mock.calls.find(([command]) => command === "revise_presentation_scope");
    expect(call?.[1].request.presentation.revision).toBe(2);
    expect(call?.[1].request.presentation.slides[0].elements[2]).toEqual(presentation.slides[0].elements[2]);
    expect(call?.[1].request.presentation.slides[1]).toEqual(presentation.slides[1]);
    expect(refresh).toHaveBeenCalled();
  });

  it("preserves the changed element's existing rich-text run styles", async () => {
    const richPresentation = structuredClone(presentation);
    const title = richPresentation.slides[0].elements.find((element) => element.objectId === "cover_title");
    if (!title || title.content.kind !== "text_box") throw new Error("cover_title_fixture_missing");
    const baseRun = title.content.text.paragraphs[0].runs[0];
    title.content.text.paragraphs[0].runs = [
      { ...baseRun, text: "Quarterly ", bold: false },
      { ...baseRun, text: "review", bold: true, color: "5856D6" },
    ];
    const richDetail = { ...detail, presentation: richPresentation };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_review") return richDetail;
      if (command === "get_presentation_preview") return { presentationId: summary.presentationId, revision: 1, filmstrip: detail.filmstrip, issues: detail.issues, rendererUnavailable: false };
      if (command === "revise_presentation_scope") return { ...richDetail, summary: { ...summary, currentRevision: 2 }, selectedRevision: 2, presentation: { ...richPresentation, revision: 2 } };
      return null;
    });

    render(<PresentationDocumentReview onRefresh={vi.fn()} summary={summary} />, { wrapper: I18nProvider });
    await screen.findByRole("heading", { name: "Quarterly review" });
    fireEvent.click(screen.getByRole("button", { name: "Edit slide" }));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Quarterly outlook" } });
    fireEvent.click(screen.getByRole("button", { name: "Save as a new version" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("revise_presentation_scope", expect.anything()));
    const call = invokeMock.mock.calls.find(([command]) => command === "revise_presentation_scope");
    const runs = call?.[1].request.presentation.slides[0].elements.find((element: { objectId: string }) => element.objectId === "cover_title").content.text.paragraphs[0].runs;
    expect(runs).toHaveLength(2);
    expect(runs[0]).toMatchObject({ text: "Quarterly ", bold: false });
    expect(runs[1]).toMatchObject({ text: "outlook", bold: true, color: "5856D6" });
  });

  it("keeps a multi-field story edit inside its selected narrative section", async () => {
    render(<PresentationDocumentReview onRefresh={vi.fn()} summary={summary} />, { wrapper: I18nProvider });
    await screen.findByRole("heading", { name: "Quarterly review" });
    fireEvent.click(screen.getByRole("button", { name: "Edit slide" }));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Quarterly outlook" } });
    fireEvent.change(screen.getByLabelText("Text 1"), { target: { value: "Prepared for the board" } });
    fireEvent.click(screen.getByRole("button", { name: "Save as a new version" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("revise_presentation_scope", expect.objectContaining({ request: expect.objectContaining({
      scope: "narrative_section", targetSlideIds: ["cover"], targetObjectIds: [],
    }) })));
  });

  it("exports a verified selected version only through an opaque native grant", async () => {
    const ready = { ...detail, summary: { ...summary, status: "ready", blockerCount: 0, issueCount: 0, visuallyVerified: true, exportable: true }, issues: [], verification: { ...detail.verification, visuallyVerified: true, exportable: true, issues: [] } };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_presentation_review") return ready;
      if (command === "choose_presentation_export_destination") return { grantToken: "grant_token_123456789", displayName: "Quarterly review.pptx", expiresAtMs: Date.now() + 60_000 };
      if (command === "export_presentation_revision") return { presentationId: summary.presentationId, revision: 1, displayName: "Quarterly review.pptx", sha256: "b".repeat(64), receiptId: "receipt-1" };
      return null;
    });
    render(<PresentationDocumentReview onRefresh={vi.fn()} summary={summary} />, { wrapper: I18nProvider });
    const button = await screen.findByRole("button", { name: "Export presentation" });
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_presentation_revision", { request: { presentationId: summary.presentationId, revision: 1, grantToken: "grant_token_123456789" } }));
    expect(invokeMock.mock.calls.some(([command, payload]) => command === "export_presentation_revision" && "destinationPath" in payload.request)).toBe(false);
  });
});
