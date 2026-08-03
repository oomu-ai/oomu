import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ShieldApprovalDialog, ShieldApprovalStatusDialog, type ShieldApprovalRequest } from "./ShieldApprovalDialog";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("ShieldApprovalDialog connector labels", () => {
  afterEach(cleanup);

  it("never prints a raw connector operation on the approval glass", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "approval",
      actionType: "connector_write",
      actionLabel: "outlook.mail.draft",
      riskTier: "high",
      reason: "BACKEND REASON outlook.mail.draft",
      requestedAtMs: Date.now(),
      preview: "",
      semanticSummary: "BACKEND SUMMARY outlook.mail.draft",
      semanticDetail: "BACKEND DETAIL outlook.mail.draft",
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    expect(screen.getAllByText("Create an Outlook draft").length).toBeGreaterThan(0);
    expect(screen.queryByText(/outlook\.mail\.draft/)).toBeNull();
    expect(screen.queryByText(/BACKEND (SUMMARY|DETAIL|REASON)/)).toBeNull();
  });

  it("localizes calendar creation in both active and pending approval views", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "calendar-create",
      actionType: "create_system_calendar",
      actionLabel: "RAW_CREATE_CALENDAR",
      riskTier: "consequential",
      reason: "RAW_BACKEND_REASON",
      requestedAtMs: Date.now(),
      preview: '{"calendarName":"OOMU Test"}',
      semanticSummary: "RAW_BACKEND_SUMMARY",
    };
    const { rerender } = render(
      <ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />,
      { wrapper: I18nProvider },
    );
    expect(screen.getAllByText("Create “OOMU Test”").length).toBeGreaterThan(0);
    expect(screen.queryByText(/RAW_/)).toBeNull();
    rerender(<ShieldApprovalStatusDialog onDismiss={vi.fn()} request={request} />);
    expect(screen.getByText("Create “OOMU Test”")).toBeVisible();
    expect(screen.queryByText(/RAW_/)).toBeNull();
  });

  it("presents external-folder access as a simple scoped choice", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const onApprove = vi.fn();
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={onApprove}
        onDeny={vi.fn()}
        request={{
          approvalToken: "folder-access",
          actionType: "file_list",
          actionLabel: "RAW_FILE_LIST",
          targetPath: "/Users/example/Documents/report.txt",
          scopeTrustPrefix: "/Users/example/Documents",
          riskTier: "file_read",
          reason: "RAW_BACKEND_REASON",
          requestedAtMs: Date.now(),
          preview: '{"raw":"json"}',
          scopeTrustAvailable: true,
          approvalScopeKinds: ["once", "app_session", "persistent"],
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("heading", { name: "Let OOMU use “report.txt”?" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeVisible();
    expect(screen.queryByRole("menuitem")).toBeNull();
    expect(screen.queryByText(/RAW_|\{"raw"/)).toBeNull();
    expect(screen.getByText("/Users/example/Documents")).not.toBeVisible();
    expect(screen.getByText("/Users/example/Documents/report.txt")).not.toBeVisible();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("/Users/example/Documents/report.txt")).toBeVisible();
    expect(screen.getByText("/Users/example/Documents")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "More access options" }));
    expect(screen.getByRole("menuitem", { name: /Until OOMU Quits.*Documents/ })).toBeVisible();
    fireEvent.click(screen.getByRole("menuitem", { name: /Always Allow This Folder.*Documents/ }));
    expect(onApprove).toHaveBeenCalledWith({ trustScope: true, trustScopeKind: "persistent" });
  });

  it.each([
    "prepare_background_agent_comparison",
    "prepare_milestone_constraint_recovery_plan",
  ])("presents %s as a localized, bounded file write", (actionType) => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: `evidence-${actionType}`,
          actionType,
          actionLabel: "RAW_EVIDENCE_ACTION",
          targetPath: "/Users/example/Project/output/report.md",
          riskTier: "file_write",
          reason: "RAW_BACKEND_REASON",
          requestedAtMs: Date.now(),
          preview: '{"raw":"preview"}',
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("heading", { name: "Let OOMU use “report.md”?" })).toBeVisible();
    expect(screen.getByText("OOMU needs to save changes here to continue.")).toBeVisible();
    expect(screen.getByText("This choice only adds access to this location.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeEnabled();
    expect(screen.queryByText(/RAW_|\{"raw"/)).toBeNull();
  });

  it("supports keyboard scope selection and restores focus when the menu closes", async () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const onDeny = vi.fn();
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={onDeny}
        request={{
          approvalToken: "folder-keyboard",
          actionType: "file_list",
          actionLabel: "List files",
          preview: "",
          targetPath: "/Users/example/Documents",
          scopeTrustPrefix: "/Users/example/Documents",
          riskTier: "file_read",
          reason: "raw",
          requestedAtMs: Date.now(),
          scopeTrustAvailable: true,
          approvalScopeKinds: ["once", "app_session", "persistent"],
        }}
      />,
      { wrapper: I18nProvider },
    );

    const trigger = screen.getByRole("button", { name: "More access options" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    const sessionChoice = screen.getByRole("menuitem", { name: /Until OOMU Quits/ });
    const persistentChoice = screen.getByRole("menuitem", { name: /Always Allow This Folder/ });
    expect(sessionChoice).toHaveFocus();

    fireEvent.keyDown(sessionChoice, { key: "ArrowDown" });
    expect(persistentChoice).toHaveFocus();
    fireEvent.keyDown(persistentChoice, { key: "Home" });
    expect(sessionChoice).toHaveFocus();
    fireEvent.keyDown(sessionChoice, { key: "End" });
    expect(persistentChoice).toHaveFocus();

    fireEvent.keyDown(persistentChoice, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(onDeny).not.toHaveBeenCalled();
  });

  it("treats Escape as a denial", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const onDeny = vi.fn();
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={onDeny} request={{ approvalToken: "escape", actionType: "mcp_tool_call", actionLabel: "tool", riskTier: "high", reason: "raw", requestedAtMs: Date.now(), preview: "" }} />, { wrapper: I18nProvider });
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onDeny).toHaveBeenCalledOnce();
  });

  it("announces resolution and ignores dismissal while the decision is saving", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const onDeny = vi.fn();
    render(
      <ShieldApprovalDialog
        isResolving
        onApprove={vi.fn()}
        onDeny={onDeny}
        request={{
          approvalToken: "busy-delete",
          actionType: "delete_file",
          actionLabel: "Delete file",
          preview: "",
          canonicalResource: "/Users/example/Documents/old.pdf",
          riskTier: "high",
          reason: "raw",
          requestedAtMs: Date.now(),
        }}
      />,
      { wrapper: I18nProvider },
    );

    const action = screen.getByRole("button", { name: "Resolving" });
    expect(action).toBeDisabled();
    expect(action).toHaveAttribute("aria-busy", "true");
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onDeny).not.toHaveBeenCalled();
  });

  it("localizes bounded connector field labels while preserving their literal values", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "approval-fields",
      actionType: "connector_write",
      actionLabel: "onedrive.file.write",
      riskTier: "high",
      reason: "backend reason",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({ path: "/Reports/Q3.xlsx", contentBytes: 420, expectedETag: "etag-7", backendFieldCanary: "kept-value" }),
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    fireEvent.click(screen.getByText("Exact destination and values"));
    expect(screen.getByText("File")).toBeVisible();
    expect(screen.getByText("/Reports/Q3.xlsx")).toBeVisible();
    expect(screen.getByText("Content size")).toBeVisible();
    expect(screen.getByText("420")).toBeVisible();
    expect(screen.queryByText(/etag-7|kept-value|contentBytes|expectedETag|backendFieldCanary/)).toBeNull();
    expect(document.querySelector("pre")).toBeNull();
  });

  it("explains a generic delete request using only the safe action and file name", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: "delete-file",
          actionType: "delete_file",
          actionLabel: "RAW_ACTION_CANARY",
          canonicalResource: "/Users/example/Documents/old-report.pdf",
          riskTier: "high",
          reason: "RAW_REASON_CANARY",
          requestedAtMs: Date.now(),
          preview: '{"secret":"RAW_PREVIEW_CANARY"}',
          semanticSummary: "RAW_SUMMARY_CANARY",
          semanticDetail: "RAW_DETAIL_CANARY",
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("Delete files")).toBeVisible();
    expect(screen.getByText("old-report.pdf")).toBeVisible();
    expect(screen.queryByText(/RAW_/)).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.getByText("/Users/example/Documents/old-report.pdf")).not.toBeVisible();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("/Users/example/Documents/old-report.pdf")).toBeVisible();
  });

  it.each(["future_system_mutation", "browser_future"])(
    "fails closed when %s has no verified identity",
    (actionType) => {
      invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
      render(
        <ShieldApprovalDialog
          isResolving={false}
          onApprove={vi.fn()}
          onDeny={vi.fn()}
          request={{
            approvalToken: "unknown-action",
            actionType,
            actionLabel: "RAW_UNKNOWN_ACTION",
            riskTier: "high",
            reason: "RAW_UNKNOWN_REASON",
            requestedAtMs: Date.now(),
            preview: "RAW_UNKNOWN_PREVIEW",
          }}
        />,
        { wrapper: I18nProvider },
      );

      expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
      expect(screen.getByText("OOMU couldn’t verify this action, so it can’t be approved. Cancel and try again.")).toBeVisible();
      expect(screen.queryByText(/RAW_UNKNOWN/)).toBeNull();
    },
  );

  it.each([
    {
      actionType: "draft_system_email",
      actionLabel: "Save a Mail draft",
      preview: {
        to: "owner@example.com",
        cc: "reviewer@example.com",
        bcc: "audit@example.com",
        subject: "Supplier Decision Review",
        body: "The verified decision pack is ready.",
      },
      visibleValues: ["audit@example.com", "Supplier Decision Review"],
    },
    {
      actionType: "create_system_calendar_event",
      actionLabel: "Add a Calendar event",
      preview: {
        calendarName: "OOMU Test",
        title: "Supplier Decision Review",
        startDate: "2026-07-20T14:00:00-04:00",
        endDate: "2026-07-20T15:00:00-04:00",
        location: "Video conference",
        notes: "Review the verified decision pack.",
        availability: "tentative",
      },
      visibleValues: ["OOMU Test", "Supplier Decision Review"],
    },
  ])("enables the bounded $actionType approval with exact visible values", ({ actionType, actionLabel, preview, visibleValues }) => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{
      approvalToken: `approval-${actionType}`,
      actionType,
      actionClass: actionType,
      actionLabel,
      riskTier: "consequential",
      reason: "raw",
      requestedAtMs: Date.now(),
      preview: JSON.stringify(preview),
    }} />, { wrapper: I18nProvider });

    expect(screen.getByText(actionLabel)).toBeVisible();
    for (const value of visibleValues) {
      expect(screen.getByText(value)).toBeVisible();
    }
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it.each([
    {
      actionType: "prepare_release_recovery_agenda",
      actionClass: "filesystem_write",
      preview: {
        inputPath: "/Users/example/testing/mock_data/project_milestones.json",
        outputPath: "/Users/example/testing/ship_test_02/release_recovery_agenda.md",
        day: "next_weekday",
        windowStartLocal: "13:00",
        windowEndLocal: "16:00",
        durationMinutes: 30,
        agendaItemCount: 5,
        locale: "en-US",
      },
      visibleValues: [
        "/Users/example/testing/mock_data/project_milestones.json",
        "/Users/example/testing/ship_test_02/release_recovery_agenda.md",
        "Stop instead of replacing an existing file",
      ],
    },
    {
      actionType: "create_release_recovery_calendar_event",
      actionClass: "external_app_write",
      preview: {
        calendarName: "OOMU Test Denial",
        title: "OOMU Release Readiness",
        startDate: "2026-07-21T13:30:00-04:00",
        endDate: "2026-07-21T14:00:00-04:00",
        location: "",
        notes: "1. Confirm owners\n2. Confirm decisions",
        availability: "tentative",
        agendaStep: 0,
        agendaSha256: "a".repeat(64),
        outputPath: "/Users/example/testing/ship_test_02/release_recovery_agenda.md",
        outputSha256: "a".repeat(64),
        byteLength: 1_024,
      },
      visibleValues: [
        "OOMU Test Denial",
        "OOMU Release Readiness",
        "One tentative event",
      ],
    },
    {
      actionType: "draft_release_recovery_email",
      actionClass: "external_app_write",
      preview: {
        to: "recipient@example.com",
        subject: "OOMU Release Readiness — Recovery Meeting",
        body: "Recovery agenda and proposed meeting time.",
        startDate: "2026-07-21T13:30:00-04:00",
        endDate: "2026-07-21T14:00:00-04:00",
        agendaItems: ["One", "Two", "Three", "Four", "Five"],
        agendaStep: 0,
        calendarStep: 1,
        agendaSha256: "b".repeat(64),
        outputPath: "/Users/example/testing/ship_test_02/release_recovery_agenda.md",
        outputSha256: "b".repeat(64),
        byteLength: 1_024,
      },
      visibleValues: [
        "recipient@example.com",
        "OOMU Release Readiness — Recovery Meeting",
        "Save one unsent draft",
        "The email will not be sent",
      ],
    },
  ])("renders a fail-closed exact Scenario 2 preview for $actionType", ({ actionType, actionClass, preview, visibleValues }) => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{
      approvalToken: `approval-${actionType}`,
      actionType,
      actionClass,
      actionLabel: "RAW_SPECIALIZED_OPERATION",
      riskTier: "consequential",
      reason: "raw",
      requestedAtMs: Date.now(),
      preview: JSON.stringify(preview),
    }} />, { wrapper: I18nProvider });

    for (const value of visibleValues) {
      expect(screen.getByText(value)).toBeVisible();
    }
    expect(screen.queryByText("RAW_SPECIALIZED_OPERATION")).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("blocks a release-recovery Mail preview that tries to add sending authority", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{
      approvalToken: "approval-release-mail-send",
      actionType: "draft_release_recovery_email",
      actionClass: "external_app_write",
      actionLabel: "RAW_SPECIALIZED_OPERATION",
      riskTier: "consequential",
      reason: "raw",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({
        to: "recipient@example.com",
        subject: "OOMU Release Readiness — Recovery Meeting",
        body: "Recovery agenda.",
        startDate: "2026-07-21T13:30:00-04:00",
        endDate: "2026-07-21T14:00:00-04:00",
        agendaItems: ["One", "Two", "Three", "Four", "Five"],
        agendaStep: 0,
        calendarStep: 1,
        agendaSha256: "b".repeat(64),
        outputPath: "/Users/example/testing/ship_test_02/release_recovery_agenda.md",
        outputSha256: "b".repeat(64),
        byteLength: 1_024,
        send: true,
      }),
    }} />, { wrapper: I18nProvider });

    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByText(/couldn’t verify this action/i)).toBeVisible();
  });

  it("preserves a complete Mail approval when the exact body exceeds the generic preview limit", () => {
    const body = "Verified supplier evidence. ".repeat(40).trim();
    expect(body.length).toBeGreaterThan(700);
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{
      approvalToken: "approval-long-mail",
      actionType: "draft_system_email",
      actionClass: "draft_system_email",
      actionLabel: "Save a Mail draft",
      riskTier: "consequential",
      reason: "raw",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({
        to: "owner@example.com",
        cc: "",
        bcc: "",
        subject: "Supplier Decision Review",
        body,
      }),
    }} />, { wrapper: I18nProvider });

    expect(screen.getByText(body)).toBeVisible();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("requires a known connector operation and verified nonempty preview", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const { rerender } = render(
      <I18nProvider>
        <ShieldApprovalDialog
          isResolving={false}
          onApprove={vi.fn()}
          onDeny={vi.fn()}
          request={{
            approvalToken: "unknown-connector",
            actionType: "connector_write",
            actionLabel: "future.connector.destroy",
            riskTier: "high",
            reason: "raw",
            requestedAtMs: Date.now(),
            preview: JSON.stringify({ subject: "Quarterly update" }),
          }}
        />
      </I18nProvider>,
    );

    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByText(/couldn’t verify this action/)).toBeVisible();

    rerender(
      <I18nProvider>
        <ShieldApprovalDialog
          isResolving={false}
          onApprove={vi.fn()}
          onDeny={vi.fn()}
          request={{
            approvalToken: "known-connector",
            actionType: "connector_write",
            actionLabel: "outlook.mail.draft",
            riskTier: "high",
            reason: "raw",
            requestedAtMs: Date.now(),
            preview: JSON.stringify({ subject: "Quarterly update" }),
          }}
        />
      </I18nProvider>,
    );

    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.queryByText(/couldn’t verify this action/)).toBeNull();
  });

  it.each([
    ["gmail.draft", { to: ["person@example.com"], subject: "Quarterly update", body: "Ready for review" }, "Create Gmail drafts"],
    ["calendar.create", { summary: "Planning review", start: { dateTime: "2026-07-15T09:00:00-04:00", timeZone: "America/New_York" }, end: { dateTime: "2026-07-15T09:30:00-04:00", timeZone: "America/New_York" } }, "Create a Google Calendar event"],
    ["calendar.update", { eventId: "opaque-id", event: { summary: "Updated review", start: { dateTime: "2026-07-15T10:00:00-04:00" }, end: { dateTime: "2026-07-15T10:30:00-04:00" } } }, "Update a Google Calendar event"],
    ["drive.export", { fileId: "opaque-id", defaultFileName: "Quarterly report.pdf" }, "Export a Google Drive file"],
    ["slack.post", { channel: "#finance", text: "The report is ready", threadTs: "opaque-id" }, "send this Slack message"],
  ])("keeps the established %s connector approval available", (actionLabel, preview, visibleLabel) => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: `known-${actionLabel}`,
          actionType: "connector_write",
          actionLabel,
          riskTier: "high",
          reason: "raw",
          requestedAtMs: Date.now(),
          preview: JSON.stringify(preview),
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getAllByText(visibleLabel).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.queryByText(/couldn’t verify this action/)).toBeNull();
  });

  it("shows only a hostname for a browser request", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: "browser",
          actionType: "browser_click",
          actionLabel: "RAW_BROWSER_ACTION",
          canonicalResource: "https://bank.example/private?prompt=secret",
          riskTier: "high",
          reason: "RAW_BROWSER_REASON",
          requestedAtMs: Date.now(),
          preview: "RAW_BROWSER_PREVIEW",
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("Use the browser")).toBeVisible();
    expect(screen.getAllByText("bank.example").some((node) => node.offsetParent !== null || node.closest("details") === null)).toBe(true);
    expect(screen.queryByText("https://bank.example/private?prompt=secret")).toBeNull();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.queryByText(/private\?prompt=secret/)).toBeNull();
    expect(screen.queryByText(/RAW_BROWSER/)).toBeNull();
  });

  it("names a remote connected tool and destination without exposing raw identifiers", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: "remote-tool",
          actionType: "mcp_execute_remote_tool",
          actionLabel: "RAW_TOOL_ACTION",
          principal: "work_tools/send_report",
          canonicalResource: "https://api.example/v1/private?token=secret",
          riskTier: "high",
          reason: "RAW_TOOL_REASON",
          requestedAtMs: Date.now(),
          preview: "RAW_TOOL_PREVIEW",
          scopeTrustAvailable: true,
          approvalScopeKinds: ["once", "persistent"],
        }}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("Use a connected tool")).toBeVisible();
    expect(screen.getByText("work tools · send report · api.example")).toBeVisible();
    expect(screen.getByRole("option", { name: "Reviewed persistent trust" })).toBeVisible();
    expect(screen.queryByRole("option", { name: "Always Allow This Folder" })).toBeNull();
    expect(screen.queryByText("https://api.example/v1/private?token=secret")).toBeNull();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("api.example")).toBeVisible();
    expect(screen.queryByText(/RAW_TOOL/)).toBeNull();
  });

  it.each([
    ["artifact_export", "Export this document?", "Export document", "Word document, PDF, and a verification receipt"],
    ["workbook_export", "Export this spreadsheet?", "Export spreadsheet", "checked copy"],
    ["presentation_export", "Export this presentation?", "Export presentation", "checked PowerPoint presentation"],
  ])("keeps %s backend jargon and digests off the approval glass", (actionType, title, action, detail) => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: `approval-${actionType}`,
      actionType,
      actionLabel: "BACKEND artifact DOCX XLSX",
      targetPath: "/Users/example/Documents/report",
      riskTier: "consequential",
      reason: "BACKEND private staging",
      requestedAtMs: Date.now(),
      preview: "BACKEND OOXML preview",
      semanticSummary: "BACKEND artifact summary",
      semanticDetail: "digest deadbeef builderIdentity rendererIdentity",
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    expect(screen.getByRole("heading", { name: title })).toBeVisible();
    expect(screen.getByText(action)).toBeVisible();
    expect(screen.getByText(new RegExp(detail))).toBeVisible();
    expect(screen.queryByText(/BACKEND|artifact|OOXML|digest|builderIdentity|rendererIdentity|private staging/)).toBeNull();
  });

  it("maps app-control approval context without exposing backend prose or bundle identifiers", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "approval-app-control",
      actionType: "app_control",
      actionLabel: "BACKEND ACTION CANARY",
      riskTier: "consequential",
      reason: "BACKEND REASON CANARY",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({ appName: "Mail", actionKind: "type_text" }),
      semanticSummary: "BACKEND SUMMARY CANARY",
      semanticDetail: "BACKEND DETAIL CANARY",
      canonicalResource: "com.apple.mail/type_text",
      scopeTrustAvailable: true,
      approvalScopeKinds: ["once", "task"],
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    expect(screen.getByRole("heading", { name: "Allow this step in Mail?" })).toBeVisible();
    expect(screen.getAllByText("enter this text in Mail").length).toBeGreaterThan(0);
    expect(screen.queryByText(/BACKEND/)).toBeNull();
    expect(screen.queryByText("com.apple.mail/type_text")).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("shows a simple channel approval without exposing credentials", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{
      approvalToken: "approval-configure-channel",
      actionType: "configure_channel",
      actionClass: "filesystem_write",
      actionLabel: "configure_channel",
      riskTier: "consequential",
      reason: "BACKEND REASON CANARY",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({ platform: "telegram", ownerId: "42", isActive: true, credentialsProvided: true, token: "secret-token-canary" }),
    }} />, { wrapper: I18nProvider });

    expect(screen.getByRole("heading", { name: "Connect Telegram for 42?" })).toBeVisible();
    expect(screen.getByText("Connect Telegram")).toBeVisible();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.queryByText(/secret-token-canary|credentialsProvided|BACKEND/)).toBeNull();
  });

  it("keeps app approval paused when its structured display context is invalid", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={{ approvalToken: "approval-invalid-app", actionType: "app_control", actionLabel: "raw", riskTier: "consequential", reason: "raw", requestedAtMs: Date.now(), preview: JSON.stringify({ appName: "com.evil.App", actionKind: "raw_script" }) }} />, { wrapper: I18nProvider });
    expect(screen.getByText(/could not verify which app and step/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.queryByText(/com\.evil\.App|raw_script/)).toBeNull();
  });

  it("localizes connected-data transmission consent and keeps policy codes off the glass", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "approval-transmission",
      actionType: "connector_transmission",
      actionLabel: "BACKEND OPERATION CANARY",
      principal: "https://graph.microsoft.com",
      canonicalResource: "https://graph.microsoft.com",
      riskTier: "consequential",
      reason: "BACKEND POLICY CANARY",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({ destination: "https://graph.microsoft.com", dataClasses: ["search_query", "message_metadata"], policyPreview: "BACKEND PREVIEW CANARY" }),
      semanticSummary: "BACKEND SUMMARY CANARY",
      semanticDetail: "BACKEND DETAIL CANARY",
      mandatoryReconfirm: true,
      approvalScopeKinds: ["once"],
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    expect(screen.getByRole("heading", { name: "Allow this connected request?" })).toBeVisible();
    expect(screen.getByText("Use connected work data")).toBeVisible();
    expect(screen.queryByText(/BACKEND|search_query|message_metadata|policyPreview/)).toBeNull();
    const includedDetails = screen.getByText("Data included").closest("details");
    expect(includedDetails).not.toBeNull();
    fireEvent.click(screen.getByText("Data included"));
    expect(within(includedDetails!).getByText(/Search terms/)).toBeVisible();
    expect(within(includedDetails!).getByText(/Email details/)).toBeVisible();
    expect(within(includedDetails!).getByText("Destination")).toBeVisible();
    expect(within(includedDetails!).getByText("graph.microsoft.com")).toBeVisible();
    expect(screen.queryByText("https://graph.microsoft.com")).toBeNull();
    expect(document.querySelector("pre")).toBeNull();
  });

  it("never shows opaque resources, URL secrets, markdown, or code on the approval glass", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={{
          approvalToken: "approval-content-canaries",
          actionType: "connector_write",
          actionLabel: "outlook.mail.draft",
          canonicalResource: "https://mail.example/draft?token=URL_SECRET_CANARY",
          targetPath: "0123456789abcdef0123456789abcdef",
          scopeTrustPrefix: "com.example.private/{TOKEN_CANARY}",
          riskTier: "high",
          reason: "RAW_REASON_CANARY",
          requestedAtMs: Date.now(),
          preview: JSON.stringify({
            subject: "**Quarterly update**",
            body: "```js const SECRET_CODE_CANARY = true; ```",
          }),
        }}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByText("Exact destination and values"));
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("Quarterly update")).toBeVisible();
    expect(screen.queryByText(/URL_SECRET_CANARY|TOKEN_CANARY|SECRET_CODE_CANARY/)).toBeNull();
    expect(screen.queryByText(/```|\*\*/)).toBeNull();
    expect(screen.getAllByText("Unknown").length).toBeGreaterThan(0);
  });

  it("blocks transmission approval when the structured preview destination does not match", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} });
    const request: ShieldApprovalRequest = {
      approvalToken: "approval-transmission-mismatch",
      actionType: "connector_transmission",
      actionLabel: "ignored",
      principal: "https://graph.microsoft.com",
      canonicalResource: "https://graph.microsoft.com",
      riskTier: "consequential",
      reason: "ignored",
      requestedAtMs: Date.now(),
      preview: JSON.stringify({ destination: "https://evil.invalid", dataClasses: ["message_content"] }),
      mandatoryReconfirm: true,
      approvalScopeKinds: ["once"],
    };
    render(<ShieldApprovalDialog isResolving={false} onApprove={vi.fn()} onDeny={vi.fn()} request={request} />, { wrapper: I18nProvider });
    expect(screen.getByText("The data list could not be verified. Cancel and try again.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.queryByText(/evil\.invalid/)).toBeNull();
  });
});
