import { describe, expect, it } from "vitest";
import type { ApprovalRequest } from "./workflowPersistence";
import { approvalPreviewFromRequest } from "./workflowApprovalPreview";

const labels: Record<string, string> = {
  "chat.recovery.approval_save_draft_only": "Translated save one unsent draft",
  "mcp_confirmation.arguments": "Translated arguments",
  "mcp_confirmation.action_create_calendar_event": "Translated create Calendar event",
  "mcp_confirmation.action_send_email": "Translated send email",
  "mcp_confirmation.attachment": "Translated attachment",
  "mcp_confirmation.bcc": "Translated Bcc",
  "mcp_confirmation.calendar": "Translated calendar",
  "mcp_confirmation.cc": "Translated Cc",
  "mcp_confirmation.destination": "Translated destination",
  "mcp_confirmation.duration": "Translated duration",
  "mcp_confirmation.event_title": "Translated event",
  "mcp_confirmation.local_action": "Translated local action",
  "mcp_confirmation.location": "Translated location",
  "mcp_confirmation.minutes": "translated minutes",
  "mcp_confirmation.next_weekday": "Translated next weekday",
  "mcp_confirmation.purpose": "Translated purpose",
  "mcp_confirmation.recipient": "Translated recipient",
  "mcp_confirmation.server": "Translated service",
  "mcp_confirmation.subject": "Translated subject",
  "mcp_confirmation.time_window": "Translated time window",
  "mcp_confirmation.tool": "Translated action",
  "settings.privacy.trust.action_connected_tool": "Translated connected tool",
  "settings.privacy.trust.action_change_files": "Translated change files",
  "settings.privacy.trust.action_check_system": "Translated check Mac",
  "settings.privacy.trust.action_other": "Translated protected action",
  "settings.privacy.trust.action_read_files": "Translated read files",
  "settings.privacy.trust.action_run_command": "Translated command",
  "settings.privacy.trust.action_use_network": "Translated connect service",
  "workflows.trust.touches.local_mac": "Translated this Mac",
  "workflows.library.approve_step": "Translated workflow step",
};

const t = (key: string) => labels[key] ?? `missing:${key}`;

function request(context: unknown): ApprovalRequest {
  return {
    instanceId: "instance",
    workflowId: "workflow",
    nodeId: "node",
    message: "backend prose",
    context,
    approvalToken: "token",
    approveCommand: {},
    rejectCommand: {},
  };
}

describe("approvalPreviewFromRequest", () => {
  it("uses localized labels and turns MCP identifiers into readable names", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "work_tools",
      toolName: "send_report",
      arguments: { destination: "Finance" },
    }), t);

    expect(preview.argumentsLabel).toBe("Translated arguments");
    expect(preview.canApprove).toBe(true);
    expect(preview.serverLabel).toBe("Translated service");
    expect(preview.toolLabel).toBe("Translated action");
    expect(preview.serverName).toBe("work tools");
    expect(preview.toolName).toBe("send report");
    expect(preview.argumentsValue).toEqual([
      "Translated destination: Finance",
    ]);
    expect(preview.reusableForWorkflowVersion).toBe(false);
  });

  it("shows durable reuse only when the backend marks an exact workflow-version scope", () => {
    const reusable = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "fetch_official_page",
      arguments: { destination: "eia.gov" },
      approvalReuse: {
        scope: "workflow_version",
        workflowVersion: 1,
      },
    }), t);
    const unrelated = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "fetch_official_page",
      arguments: { destination: "eia.gov" },
      approvalReuse: {
        scope: "session",
        workflowVersion: 1,
      },
    }), t);

    expect(reusable.reusableForWorkflowVersion).toBe(true);
    expect(unrelated.reusableForWorkflowVersion).toBe(false);
  });

  it("never exposes commands, arguments, or technical execution fields", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "system_action",
      args: ["--private-token", "secret"],
      command: "dangerous-command --private-token",
      mode: "shell",
      capabilityRiskTier: "FILE_WRITE",
      timeoutMs: 30_000,
      workingDirectory: "/Users/example/Quarterly Reports",
      capabilityReason: "Update the verified report",
    }), t);

    expect(preview.toolName).toBe("Translated change files");
    expect(preview.serverName).toBe("Translated this Mac");
    expect(preview.canApprove).toBe(true);
    expect(preview.argumentsValue).toEqual([
      "Translated location: Quarterly Reports",
    ]);
    expect(JSON.stringify(preview)).not.toContain("dangerous-command");
    expect(JSON.stringify(preview)).not.toContain("private-token");
    expect(JSON.stringify(preview)).not.toContain("30000");
    expect(JSON.stringify(preview)).not.toContain("Update the verified report");
  });

  it("fails closed when a system action has no verified semantic category", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "system_action",
      mode: "shell",
      capabilityRiskTier: "UNKNOWN",
      capabilityReason: "Trust this reassuring but unverified sentence",
      command: "opaque-operation",
    }), t);

    expect(preview.canApprove).toBe(false);
    expect(preview.toolName).toBe("Translated local action");
    expect(JSON.stringify(preview)).not.toContain("reassuring");
    expect(JSON.stringify(preview)).not.toContain("opaque-operation");
  });

  it("fails closed when the execution mode is not in the backend contract", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "system_action",
      mode: "future_mode",
      capabilityRiskTier: "FILE_WRITE",
    }), t);

    expect(preview.canApprove).toBe(false);
  });

  it.each([
    ["file_read", "Translated read files"],
    ["file_write", "Translated change files"],
    ["network", "Translated connect service"],
    ["process", "Translated command"],
    ["mcp_tool", "Translated connected tool"],
  ])("approves the backend's typed %s workflow permission", (permissionKind, label) => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "workflow_permission",
      permissionKind,
      actionLabel: "Approve Email Reply",
      capabilityReason: "Review the reply before opening the Mail draft.",
    }), t);

    expect(preview.canApprove).toBe(true);
    expect(preview.toolName).toBe(label);
    expect(preview.serverName).toBe("OOMU");
    expect(preview.argumentsValue).toEqual([
      "Translated purpose: Approve Email Reply",
      "Review the reply before opening the Mail draft.",
    ]);
  });

  it("fails closed for an unclassified custom workflow permission", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "workflow_permission",
      permissionKind: "custom",
      actionLabel: "Trust this action",
      capabilityReason: "A workflow requested it.",
    }), t);

    expect(preview.canApprove).toBe(false);
    expect(preview.toolName).toBe("Translated protected action");
  });

  it("keeps only bounded, allowlisted MCP meaning", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      capabilityReason: "- Send the finished report\u0000",
      capabilityRiskTier: "high",
      serverName: "work_tools `private code`",
      toolName: "send_report --private-token",
      arguments: {
        destination_path: "/Users/example/Board/weekly.md",
        raw_json: "{ secret: true }",
        token: "private-token-value",
        content_sha256: "abcdef0123456789abcdef0123456789",
      },
    }), t);

    expect(preview.argumentsValue).toEqual([
      "Translated purpose: Send the finished report",
      "Translated destination: weekly.md",
    ]);
    const rendered = JSON.stringify(preview);
    expect(rendered).not.toContain("private code");
    expect(rendered).not.toContain("private-token");
    expect(rendered).not.toContain("private-token-value");
    expect(rendered).not.toContain("abcdef0123456789");
    expect(rendered).not.toContain("raw_json");
  });

  it("shows the exact allowlisted Mail recipients and subject without the body or secrets", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "send_system_email",
      arguments: {
        to: "recipient@example.com",
        cc: "reviewer@example.com",
        bcc: "audit@example.com",
        subject: "OOMU Test — Supplier Exception",
        attachmentPath: "/Users/example/Reports/supplier_exception.md",
        body: "Sensitive report body that must never appear in the approval preview.",
        token: "never-render-this-token",
      },
    }), t);

    expect(preview.canApprove).toBe(true);
    expect(preview.serverName).toBe("OOMU");
    expect(preview.toolName).toBe("Translated send email");
    expect(preview.argumentsValue).toEqual([
      "Translated recipient: recipient@example.com",
      "Translated Cc: reviewer@example.com",
      "Translated Bcc: audit@example.com",
      "Translated subject: OOMU Test — Supplier Exception",
      "Translated attachment: supplier_exception.md",
    ]);
    expect(JSON.stringify(preview)).not.toContain("Sensitive report body");
    expect(JSON.stringify(preview)).not.toContain("never-render-this-token");
  });

  it("shows an exact Mail-draft approval without exposing its message body", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "draft_system_email",
      arguments: {
        body: "Sensitive draft body that must never appear.",
        subject: "Supplier decision",
        to: "recipient@example.com",
      },
    }), t);

    expect(preview.canApprove).toBe(true);
    expect(preview.toolName).toBe("Translated save one unsent draft");
    expect(preview.argumentsValue).toEqual([
      "Translated recipient: recipient@example.com",
      "Translated subject: Supplier decision",
    ]);
    expect(JSON.stringify(preview)).not.toContain("Sensitive draft body");
  });

  it("shows the exact Calendar, event, time window, and duration", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "create_conflict_free_calendar_event",
      arguments: {
        calendarName: "OOMU Test",
        title: "Supplier Exception Follow-up",
        day: "next_weekday",
        windowStartLocal: "13:00",
        windowEndLocal: "16:00",
        durationMinutes: 30,
        notes: "Internal context that should not appear.",
        availability: "tentative",
      },
    }), t);

    expect(preview.canApprove).toBe(true);
    expect(preview.serverName).toBe("OOMU");
    expect(preview.toolName).toBe("Translated create Calendar event");
    expect(preview.argumentsValue).toEqual([
      "Translated calendar: OOMU Test",
      "Translated event: Supplier Exception Follow-up",
      "Translated time window: Translated next weekday, 13:00–16:00",
      "Translated duration: 30 translated minutes",
    ]);
    expect(JSON.stringify(preview)).not.toContain("Internal context");
    expect(JSON.stringify(preview)).not.toContain("availability");
  });

  it("fails closed when an exact Mail or Calendar approval is incomplete", () => {
    const mail = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "send_system_email",
      arguments: { to: "recipient@example.com", body: "No subject" },
    }), t);
    const calendar = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "create_conflict_free_calendar_event",
      arguments: {
        calendarName: "OOMU Test",
        title: "Supplier Exception Follow-up",
        day: "next_weekday",
        windowStartLocal: "13:00",
        durationMinutes: 30,
      },
    }), t);

    expect(mail.canApprove).toBe(false);
    expect(calendar.canApprove).toBe(false);
  });

  it("fails closed without exposing an unsafe Mail attachment path", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "mcp_tool",
      serverName: "oomu_task_tools",
      toolName: "send_system_email",
      arguments: {
        to: "owner@example.com",
        subject: "Report",
        body: "Attached.",
        attachmentPath: "/Users/example/private-api-token.txt",
      },
    }), t);

    expect(preview.canApprove).toBe(false);
    expect(JSON.stringify(preview)).not.toContain("private-api-token");
    expect(JSON.stringify(preview)).not.toContain("/Users/example");
  });

  it("does not pass unknown workflow context into the permission prompt", () => {
    const preview = approvalPreviewFromRequest(request({
      actionType: "future_action",
      secret: "never show this",
    }), t);

    expect(preview.toolName).toBe("Translated workflow step");
    expect(preview.canApprove).toBe(false);
    expect(preview.argumentsValue).toEqual([]);
    expect(JSON.stringify(preview)).not.toContain("never show this");
  });
});
