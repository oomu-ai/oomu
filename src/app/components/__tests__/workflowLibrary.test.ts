import { describe, expect, it } from "vitest";
import enUS from "@/locales/en-US.json";
import {
  bindWorkflowSourceFolder,
  instantiateWorkflowIrTemplate,
  localizeWorkflowIrTemplate,
  workflowTemplateById,
  workflowTemplates,
  type WorkflowTemplateId,
} from "../workflowLibrary";
import type { WorkflowIrNode } from "../workflowIr";

const DIRECTORY_SUMMARIZER_ID: WorkflowTemplateId = "directory-summarizer";

function t(key: string) {
  let value: unknown = enUS;
  for (const part of key.split(".")) {
    value = value && typeof value === "object"
      ? (value as Record<string, unknown>)[part]
      : undefined;
  }
  return typeof value === "string" ? value : key;
}

// Intentional contract pin for `src-tauri/src/mcp/taskflow.rs::tool_list`.
const EXPECTED_TOOL_SCHEMAS = {
  folder_read: {
    type: "object",
    properties: {
      folderPath: {
        type: "string",
        description: "The approved project folder path.",
      },
    },
    required: ["folderPath"],
    additionalProperties: false,
  },
  write_markdown_report: {
    type: "object",
    properties: {
      reportPath: {
        type: "string",
        description: "Markdown report file path.",
      },
      content: {
        type: "string",
        description: "Markdown report content to write.",
      },
    },
    required: ["reportPath", "content"],
    additionalProperties: false,
  },
  preview_report: {
    type: "object",
    properties: {
      reportPath: {
        type: "string",
        description: "The path to the generated report to preview.",
      },
    },
    required: ["reportPath"],
    additionalProperties: false,
  },
} as const;

describe("workflowLibrary directory summarizer", () => {
  it("publishes the stable directory-summarizer template ID", () => {
    expect(workflowTemplates.map((template) => template.id)).toContain(
      DIRECTORY_SUMMARIZER_ID,
    );
    expect(workflowTemplateById(DIRECTORY_SUMMARIZER_ID)).toMatchObject({
      id: DIRECTORY_SUMMARIZER_ID,
      name: "workflows.templates.directory-summarizer.name",
    });
  });

  it("localizes every user-visible template field and carries truncation truth", () => {
    const template = workflowTemplateById(DIRECTORY_SUMMARIZER_ID);
    expect(template).toBeDefined();
    if (!template) throw new Error("Directory Summarizer template is missing.");

    const localized = localizeWorkflowIrTemplate(template, t, {
      sourceTruncated: true,
    });
    expect(localized.name).toBe("Directory Summarizer");
    expect(localized.description).toContain("notes and text files");
    expect(localized.seedPrompt).toContain("important notes and text files");
    expect(
      localized.workflowIr.nodes.find((node) => node.id === "read-approved-folder")?.label,
    ).toBe("Read chosen folder");
    expect(
      localized.workflowIr.nodes.find((node) => node.id === "summarize-folder"),
    ).toMatchObject({
      kind: "agent",
      label: "Summarize folder",
      objective: expect.stringContaining("larger than OOMU can read in one pass"),
    });
    expect(
      localized.workflowIr.nodes.find((node) => node.id === "approve-directory-report"),
    ).toMatchObject({
      kind: "permission",
      label: "Review report before saving",
      reason: "Review the generated folder summary before saving it.",
    });
  });

  it("uses the executable native taskflow contracts with review before write and preview", () => {
    const template = workflowTemplateById(DIRECTORY_SUMMARIZER_ID);
    expect(template).toBeDefined();
    if (!template) {
      throw new Error("Directory Summarizer template is missing.");
    }

    const workflowIr = instantiateWorkflowIrTemplate(
      template,
      "wf-directory-summarizer-test",
    );
    const toolNodes = workflowIr.nodes.filter(isMcpToolNode);

    expect(
      toolNodes.map(({ serverName, toolName }) => ({ serverName, toolName })),
    ).toEqual([
      { serverName: "taskflow_native", toolName: "folder_read" },
      { serverName: "taskflow_native", toolName: "write_markdown_report" },
      { serverName: "taskflow_native", toolName: "preview_report" },
    ]);
    expect(toolNodes[0]).toMatchObject({
      id: "read-approved-folder",
      arguments: { folderPath: "workspace/selections/source-required" },
      inputSchema: EXPECTED_TOOL_SCHEMAS.folder_read,
    });
    expect(toolNodes[1]).toMatchObject({
      id: "write-directory-report",
      arguments: {
        reportPath: "workspace/report.md",
        content: "{{nodes.summarize-folder.output.data}}",
      },
      inputSchema: EXPECTED_TOOL_SCHEMAS.write_markdown_report,
    });
    expect(toolNodes[2]).toMatchObject({
      id: "preview-directory-report",
      arguments: { reportPath: "workspace/report.md" },
      inputSchema: EXPECTED_TOOL_SCHEMAS.preview_report,
    });

    expect(workflowIr.nodes.map((node) => node.id)).toEqual([
      "input",
      "read-approved-folder",
      "folder-has-files",
      "summarize-folder",
      "approve-directory-report",
      "write-directory-report",
      "preview-directory-report",
      "output",
      "empty-output",
    ]);
    expect(workflowIr.nodes).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: "permission",
          id: "approve-directory-report",
          permission: "file_write",
          onDenied: "fail",
        }),
      ]),
    );
    expect(
      workflowIr.edges.map(({ sourceNodeId, sourcePort, targetNodeId }) => ({
        sourceNodeId,
        sourcePort,
        targetNodeId,
      })),
    ).toEqual([
      {
        sourceNodeId: "input",
        sourcePort: "out",
        targetNodeId: "read-approved-folder",
      },
      {
        sourceNodeId: "read-approved-folder",
        sourcePort: "out",
        targetNodeId: "folder-has-files",
      },
      {
        sourceNodeId: "folder-has-files",
        sourcePort: "true",
        targetNodeId: "summarize-folder",
      },
      {
        sourceNodeId: "folder-has-files",
        sourcePort: "false",
        targetNodeId: "empty-output",
      },
      {
        sourceNodeId: "summarize-folder",
        sourcePort: "out",
        targetNodeId: "approve-directory-report",
      },
      {
        sourceNodeId: "approve-directory-report",
        sourcePort: "approved",
        targetNodeId: "write-directory-report",
      },
      {
        sourceNodeId: "write-directory-report",
        sourcePort: "out",
        targetNodeId: "preview-directory-report",
      },
      {
        sourceNodeId: "preview-directory-report",
        sourcePort: "out",
        targetNodeId: "output",
      },
    ]);
    expect(workflowIr.metadata).toMatchObject({
      templateId: DIRECTORY_SUMMARIZER_ID,
    });
  });

  it("binds each saved template to its own staged source", () => {
    const template = workflowTemplateById(DIRECTORY_SUMMARIZER_ID);
    expect(template).toBeDefined();
    if (!template) throw new Error("Directory Summarizer template is missing.");

    const workflowIr = bindWorkflowSourceFolder(
      instantiateWorkflowIrTemplate(template, "wf-bound-source"),
      "workspace/selections/selection-case-a",
    );
    expect(
      workflowIr.nodes.find((node) => node.id === "read-approved-folder"),
    ).toMatchObject({
      kind: "mcp_tool",
      arguments: { folderPath: "workspace/selections/selection-case-a" },
    });
    expect(() => bindWorkflowSourceFolder(workflowIr, "workspace/input")).toThrow(
      "workflow_source_folder_path_invalid",
    );
  });
});

describe("workflowLibrary email responder", () => {
  it("reads Mail fields from the normalized MCP payload envelope", () => {
    const template = workflowTemplateById("email-responder");
    expect(template).toBeDefined();
    if (!template) throw new Error("Email Responder template is missing.");

    const workflowIr = instantiateWorkflowIrTemplate(
      template,
      "wf-email-responder-contract",
    );
    const draftNode = workflowIr.nodes.find(
      (node): node is Extract<WorkflowIrNode, { kind: "mcp_tool" }> =>
        node.kind === "mcp_tool" && node.id === "draft-outgoing-email",
    );

    expect(draftNode?.arguments).toMatchObject({
      to: "{{nodes.read-unread-emails.output.data.structuredContent.emails.0.sender}}",
      subject:
        "Re: {{nodes.read-unread-emails.output.data.structuredContent.emails.0.subject}}",
      body: "{{nodes.draft-reply.output.data}}",
    });
  });
});

describe("workflowLibrary empty-result contract", () => {
  it("gives every collection workflow a deterministic empty completion path", () => {
    const collectionTemplates: WorkflowTemplateId[] = [
      "directory-summarizer",
      "daily-briefing-system-setup",
      "email-responder",
      "calendar-assistant",
      "daily-mail-reminders-scraper",
    ];

    for (const templateId of collectionTemplates) {
      const template = workflowTemplateById(templateId);
      if (!template) throw new Error(`Missing template ${templateId}`);
      const workflowIr = instantiateWorkflowIrTemplate(template, `wf-${templateId}`);
      const emptyOutput = workflowIr.nodes.find(
        (node) => node.kind === "output" && node.completionKind === "empty_collection",
      );
      expect(emptyOutput, templateId).toBeDefined();
      expect(
        workflowIr.nodes.some(
          (node) =>
            node.kind === "conditional" &&
            node.condition === "$ != []" &&
            workflowIr.edges.some(
              (edge) =>
                edge.sourceNodeId === node.id &&
                edge.sourcePort === "false" &&
                emptyOutput &&
                reachesNode(workflowIr.edges, edge.targetNodeId, emptyOutput.id),
            ),
        ),
        templateId,
      ).toBe(true);
    }
  });

  it("passes both Mail and Reminders into the combined brief", () => {
    const template = workflowTemplateById("daily-mail-reminders-scraper");
    if (!template) throw new Error("Mail and Reminders template is missing.");
    const workflowIr = instantiateWorkflowIrTemplate(template, "wf-mail-reminders");
    expect(workflowIr.nodes.find((node) => node.id === "daily-capture")).toMatchObject({
      kind: "agent",
      inputMappings: {
        mail: "{{nodes.daily-scraper-mail.output}}",
        reminders: "{{nodes.daily-scraper-reminders.output}}",
      },
    });
  });
});

describe("workflowLibrary execution timing", () => {
  it("gives every packaged external step enough time for its real transport", () => {
    for (const template of workflowTemplates) {
      const workflowIr = instantiateWorkflowIrTemplate(
        template,
        `wf-timeout-${template.id}`,
      );
      for (const node of workflowIr.nodes.filter(isMcpToolNode)) {
        expect(node.systemTimeoutMs, `${template.id}:${node.id}`).toBeGreaterThanOrEqual(
          60_000,
        );
        if (
          node.serverName === "macos_applescript" &&
          node.toolName === "read_system_calendar"
        ) {
          expect(node.systemTimeoutMs, `${template.id}:${node.id}`).toBeGreaterThanOrEqual(
            75_000,
          );
        }
      }
    }
  });
});

function isMcpToolNode(
  node: WorkflowIrNode,
): node is Extract<WorkflowIrNode, { kind: "mcp_tool" }> {
  return node.kind === "mcp_tool";
}

function reachesNode(
  edges: { sourceNodeId: string; targetNodeId: string }[],
  start: string,
  target: string,
) {
  const pending = [start];
  const seen = new Set<string>();
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current || seen.has(current)) continue;
    if (current === target) return true;
    seen.add(current);
    pending.push(
      ...edges
        .filter((edge) => edge.sourceNodeId === current)
        .map((edge) => edge.targetNodeId),
    );
  }
  return false;
}
