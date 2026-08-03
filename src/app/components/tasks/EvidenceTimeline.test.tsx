import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { parseProjectId, parseTaskId, parseTaskRunId, type P0EventEnvelope } from "@/lib/p0Contracts";
import { EvidenceTimeline } from "./EvidenceTimeline";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("EvidenceTimeline connected-work details", () => {
  afterEach(cleanup);

  it("shows a plain summary and reveals bound source, freshness, route, and postcondition only in Details", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const event: P0EventEnvelope = {
      schemaVersion: 1,
      eventType: "connector.tool.completed",
      projectId: parseProjectId("project_22222222-2222-4222-8222-222222222222"),
      taskId: parseTaskId("task_33333333-3333-4333-8333-333333333333"),
      taskRunId: parseTaskRunId("taskrun_55555555-5555-4555-8555-555555555555"),
      correlationId: "correlation",
      sequence: 4,
      timestamp: "2026-07-11T18:00:00Z",
      evidenceClass: "verified_postcondition",
      payload: { capability: "read_email", source: { origin: "https://graph.microsoft.com", citation: "graph://outlook/mail/message/abc", freshness: "live", observedAtMs: 1 }, partial: false, accountBindingHash: "a".repeat(64), tenantBindingHash: "b".repeat(64), postcondition: { mutationPostcondition: "observed" } },
    };
    render(<EvidenceTimeline emptyLabel="Empty" events={[event]} />, { wrapper: I18nProvider });
    expect(screen.getByText("Email read")).toBeVisible();
    expect(screen.getByText("The requested result is complete.")).toBeVisible();
    expect(screen.getByText("graph://outlook/mail/message/abc")).not.toBeVisible();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("graph://outlook/mail/message/abc")).toBeVisible();
    expect(screen.getByText("Read live from the connected service")).toBeVisible();
    expect(screen.getByText("Microsoft 365")).toBeVisible();
    expect(screen.getAllByText("Identity confirmed for this Task")).toHaveLength(2);
    expect(screen.getByText("OOMU checked the result after the action")).toBeVisible();
    expect(screen.queryByText(/microsoft_graph|read_email|accountBindingHash|tenantBindingHash/)).toBeNull();
  });
});
