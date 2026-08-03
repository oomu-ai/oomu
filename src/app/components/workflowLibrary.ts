import {
  MEDIUM_TIMEOUT_MS,
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  workflowIrSchema,
  type WorkflowIr,
  type WorkflowIrEdge,
  type WorkflowIrNode,
} from "./workflowIr";

export type WorkflowIrTemplateExample = {
  description: string;
  id: string;
  name: string;
  seedPrompt: string;
  workflowIr: WorkflowIr;
};

type TemplateTranslate = (
  key: string,
  values?: Record<string, string | number>,
) => string;


const LOCAL_FILESYSTEM_SERVER_NAME = "local_filesystem";
const MACOS_APPLESCRIPT_SERVER_NAME = "macos_applescript";
const TASKFLOW_NATIVE_SERVER_NAME = "taskflow_native";

const LOCAL_SANDBOX_INSTRUCTION_PATH = "instruction_input.txt";
const LOCAL_SANDBOX_REPORT_PATH = "executive_summary.txt";
const LOCAL_SANDBOX_DAILY_BRIEF_PATH = "daily_mail_reminders_brief.md";
const TASKFLOW_SOURCE_REQUIRED_PATH = "workspace/selections/source-required";
const TASKFLOW_REPORT_PATH = "workspace/report.md";
const CALENDAR_WORKFLOW_TIMEOUT_MS = 75_000;

const OBJECT_SCHEMA = { type: "object", additionalProperties: true } as const;

const LOCAL_FILESYSTEM_TOOL_SCHEMAS = {
  readFile: {
    type: "object",
    properties: {
      path: {
        type: "string",
        description: "Relative sandbox path, or an absolute path inside the sandbox.",
      },
    },
    required: ["path"],
    additionalProperties: false,
  },
  writeFile: {
    type: "object",
    properties: {
      path: {
        type: "string",
        description: "Relative sandbox path, or an absolute path inside the sandbox.",
      },
      content: {
        type: "string",
        description: "Text content to write.",
      },
    },
    required: ["path", "content"],
    additionalProperties: false,
  },
} as const;

// Keep these structurally equivalent to `mcp/taskflow.rs::tool_list`. The
// workflow compiler publishes its native capabilities from that same list.
const TASKFLOW_NATIVE_TOOL_SCHEMAS = {
  folderRead: {
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
  writeMarkdownReport: {
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
  previewReport: {
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

const MACOS_APPLESCRIPT_TOOL_SCHEMAS = {
  readSystemCalendar: {
    type: "object",
    properties: {
      calendar_name: {
        type: "string",
        description: "Optional Calendar name. Leave blank to use Calendar's default calendar.",
      },
      hours_ahead: {
        type: "number",
        description: "Window size when end_date is omitted.",
        minimum: 0.25,
        maximum: 720,
      },
      start_date: {
        type: "string",
        description: "Optional local ISO 8601 start, for example 2026-06-16T09:00:00.",
      },
      end_date: {
        type: "string",
        description: "Optional local ISO 8601 end, for example 2026-06-16T17:00:00.",
      },
    },
    additionalProperties: false,
  },
  triggerSystemNotification: {
    type: "object",
    properties: {
      title_text: {
        type: "string",
        description: "Notification title.",
      },
      subtitle_text: {
        type: "string",
        description: "Optional notification subtitle.",
      },
      body_text: {
        type: "string",
        description: "Notification body text.",
      },
    },
    required: ["body_text"],
    additionalProperties: false,
  },
  draftSystemEmail: {
    type: "object",
    properties: {
      to: {
        type: "string",
        description: "Comma-separated recipient email addresses.",
      },
      subject: {
        type: "string",
        description: "Draft subject line.",
      },
      body: {
        type: "string",
        description: "Draft message body.",
      },
      cc: {
        type: "string",
        description: "Optional comma-separated CC recipients.",
      },
      bcc: {
        type: "string",
        description: "Optional comma-separated BCC recipients.",
      },
    },
    required: ["subject", "body"],
    additionalProperties: false,
  },
  readSystemEmails: {
    type: "object",
    properties: {
      max_messages: {
        type: "number",
        description: "Maximum number of recent messages to retrieve.",
        minimum: 1,
        maximum: 50,
      },
      unread_only: {
        type: "boolean",
        description: "If true, retrieve only unread messages.",
      },
    },
    additionalProperties: false,
  },
  readSystemReminders: {
    type: "object",
    properties: {
      list_name: {
        type: "string",
        description: "Optional Reminder list name. Leave blank to read all reminder lists.",
      },
      completed_only: {
        type: "boolean",
        description: "If true, retrieve completed tasks instead of uncompleted ones.",
      },
    },
    additionalProperties: false,
  },
} as const;

function structuredCollectionOutputSchema(collectionName: string) {
  return {
    type: "object",
    "x-oomu-result-contract": {
      kind: "collection",
      path: `/structuredContent/${collectionName}`,
      emptyIsSuccess: true,
    },
    properties: {
      structuredContent: {
        type: "object",
        properties: {
          [collectionName]: { type: "array", items: {} },
        },
        required: [collectionName],
        additionalProperties: true,
      },
    },
    required: ["structuredContent"],
    additionalProperties: true,
  } as const;
}

const COLLECTION_OUTPUT_SCHEMAS = {
  events: structuredCollectionOutputSchema("events"),
  files: structuredCollectionOutputSchema("files"),
  emails: structuredCollectionOutputSchema("emails"),
  reminders: structuredCollectionOutputSchema("reminders"),
} as const;

export const workflowTemplates = [
  createWorkflowIrTemplate({
    id: "directory-summarizer",
    name: "workflows.templates.directory-summarizer.name",
    description: "workflows.templates.directory-summarizer.description",
    seedPrompt: "workflows.templates.directory-summarizer.seed_prompt",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "read-approved-folder",
        label: "workflows.templates.directory-summarizer.read_label",
        serverName: TASKFLOW_NATIVE_SERVER_NAME,
        toolName: "folder_read",
        inputSchema: TASKFLOW_NATIVE_TOOL_SCHEMAS.folderRead,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.files,
        args: { folderPath: TASKFLOW_SOURCE_REQUIRED_PATH },
      }),
      conditionalNode({
        id: "folder-has-files",
        label: "workflows.templates.empty_result.check_files",
        condition: "$ != []",
        inputMapping:
          "{{nodes.read-approved-folder.output.data.structuredContent.files}}",
      }),
      agentNode({
        id: "summarize-folder",
        label: "workflows.templates.directory-summarizer.summarize_label",
        objective: "workflows.templates.directory-summarizer.objective",
        context: "{{nodes.read-approved-folder.output}}",
      }),
      permissionNode({
        id: "approve-directory-report",
        label: "workflows.templates.directory-summarizer.review_label",
        permission: "file_write",
        reason: "workflows.templates.directory-summarizer.review_reason",
      }),
      mcpToolNode({
        id: "write-directory-report",
        label: "workflows.templates.directory-summarizer.write_label",
        serverName: TASKFLOW_NATIVE_SERVER_NAME,
        toolName: "write_markdown_report",
        inputSchema: TASKFLOW_NATIVE_TOOL_SCHEMAS.writeMarkdownReport,
        args: {
          reportPath: TASKFLOW_REPORT_PATH,
          content: "{{nodes.summarize-folder.output.data}}",
        },
      }),
      mcpToolNode({
        id: "preview-directory-report",
        label: "workflows.templates.directory-summarizer.preview_label",
        serverName: TASKFLOW_NATIVE_SERVER_NAME,
        toolName: "preview_report",
        inputSchema: TASKFLOW_NATIVE_TOOL_SCHEMAS.previewReport,
        args: { reportPath: TASKFLOW_REPORT_PATH },
      }),
      outputNode("{{nodes.preview-directory-report.output}}"),
      outputNode(
        "{{nodes.read-approved-folder.output.data.structuredContent.files}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges(["input", "read-approved-folder", "folder-has-files"]),
      edge("folder-has-files", "true", "summarize-folder"),
      edge("folder-has-files", "false", "empty-output"),
      edge("summarize-folder", "out", "approve-directory-report"),
      edge("approve-directory-report", "approved", "write-directory-report"),
      edge("write-directory-report", "out", "preview-directory-report"),
      edge("preview-directory-report", "out", "output"),
    ],
  }),
  createWorkflowIrTemplate({
    id: "local-sandbox-log-summarizer",
    name: "Local Sandbox Log Summarizer",
    description:
      "Reads files from the secure local sandbox, summarizes them locally using Gemma 4, and writes a unified analytical report back to disk.",
    seedPrompt:
      "Read instruction_input.txt from the secure local sandbox, summarize it locally, ask before writing, then write executive_summary.txt.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "read-sandbox-instructions",
        label: "Read Sandbox Instructions",
        serverName: LOCAL_FILESYSTEM_SERVER_NAME,
        toolName: "read_file",
        inputSchema: LOCAL_FILESYSTEM_TOOL_SCHEMAS.readFile,
        args: { path: LOCAL_SANDBOX_INSTRUCTION_PATH },
      }),
      agentNode({
        id: "sandbox-summary",
        label: "Gemma 4 Summary",
        objective:
          "Read the MCP payload, identify the core instructions, risks, variables, and next action, then write a concise executive summary grounded only in the sandbox content.",
        context: "{{nodes.read-sandbox-instructions.output}}",
      }),
      permissionNode({
        id: "approve-report",
        label: "Approve Report",
        permission: "file_write",
        reason: "Write executive_summary.txt after human approval.",
      }),
      mcpToolNode({
        id: "write-executive-summary",
        label: "Write Executive Summary",
        serverName: LOCAL_FILESYSTEM_SERVER_NAME,
        toolName: "write_file",
        inputSchema: LOCAL_FILESYSTEM_TOOL_SCHEMAS.writeFile,
        args: {
          path: LOCAL_SANDBOX_REPORT_PATH,
          content: "{{nodes.sandbox-summary.output.data}}",
        },
      }),
      outputNode("{{nodes.write-executive-summary.output}}"),
    ],
    edges: [
      ...linearEdges(["input", "read-sandbox-instructions", "sandbox-summary", "approve-report"]),
      edge("approve-report", "approved", "write-executive-summary"),
      edge("write-executive-summary", "out", "output"),
    ],
  }),
  createWorkflowIrTemplate({
    id: "daily-briefing-system-setup",
    name: "Daily Briefing and System Setup",
    description:
      "Reads the local macOS Calendar, asks Gemma 4 for a daily briefing, routes on calendar state, sends a native notification, and opens a Mail draft for review.",
    seedPrompt:
      "Read my local macOS Calendar, produce a daily briefing, notify me when there is useful work, and prepare a Mail draft for review.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "read-calendar",
        label: "Read macOS Calendar",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_calendar",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemCalendar,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.events,
        args: {
          calendar_name: "",
          hours_ahead: 12,
          start_date: "",
          end_date: "",
        },
        timeoutMs: CALENDAR_WORKFLOW_TIMEOUT_MS,
      }),
      conditionalNode({
        id: "calendar-has-events",
        label: "workflows.templates.empty_result.check_events",
        condition: "$ != []",
        inputMapping: "{{nodes.read-calendar.output.data.structuredContent.events}}",
      }),
      agentNode({
        id: "daily-setup",
        label: "Gemma 4 Daily Setup",
        objective:
          "Use the Calendar MCP payload to produce a concise daily briefing. Include one notification-ready sentence and a short email draft body. Do not invent events.",
        context: "{{nodes.read-calendar.output}}",
      }),
      mcpToolNode({
        id: "send-notification",
        label: "Send System Notification",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "trigger_system_notification",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.triggerSystemNotification,
        args: {
          title_text: "OOMU Daily Briefing",
          subtitle_text: "System Setup",
          body_text: "{{nodes.daily-setup.output.data}}",
        },
      }),
      mcpToolNode({
        id: "draft-briefing-email",
        label: "Draft Briefing Email",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "draft_system_email",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.draftSystemEmail,
        args: {
          to: "",
          subject: "Daily briefing",
          body: "{{nodes.daily-setup.output.data}}",
          cc: "",
          bcc: "",
        },
      }),
      outputNode("{{nodes.draft-briefing-email.output}}"),
      outputNode(
        "{{nodes.read-calendar.output.data.structuredContent.events}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges(["input", "read-calendar", "calendar-has-events"]),
      edge("calendar-has-events", "true", "daily-setup"),
      edge("calendar-has-events", "false", "empty-output"),
      edge("daily-setup", "out", "send-notification"),
      edge("send-notification", "out", "draft-briefing-email"),
      edge("draft-briefing-email", "out", "output"),
    ],
  }),
  createWorkflowIrTemplate({
    id: "unread-mail-check",
    name: "Unread Mail Check",
    description:
      "Reads unread messages from macOS Mail and returns only the receipt-backed Mail result.",
    seedPrompt:
      "Read unread messages from macOS Mail and return the verified result without changing any message.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "read-unread-emails",
        label: "Read unread Mail",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_emails",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemEmails,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.emails,
        args: { max_messages: 20, unread_only: true },
      }),
      conditionalNode({
        id: "mail-has-messages",
        label: "Check for unread Mail",
        condition: "$ != []",
        inputMapping:
          "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
      }),
      outputNode(
        "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
      ),
      outputNode(
        "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges(["input", "read-unread-emails", "mail-has-messages"]),
      edge("mail-has-messages", "true", "output"),
      edge("mail-has-messages", "false", "empty-output"),
    ],
  }),
  createWorkflowIrTemplate({
    id: "email-responder",
    name: "Email Responder",
    description:
      "Reads new unread emails from macOS Mail, drafts professional replies using Gemma 4, and prepares the draft for review.",
    seedPrompt:
      "Read unread macOS Mail messages, draft professional replies, ask me to approve the content, then open a visible Mail draft.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "read-unread-emails",
        label: "Read macOS Emails",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_emails",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemEmails,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.emails,
        args: {
          max_messages: 5,
          unread_only: true,
        },
      }),
      conditionalNode({
        id: "mail-has-messages",
        label: "workflows.templates.empty_result.check_mail",
        condition: "$ != []",
        inputMapping:
          "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
      }),
      agentNode({
        id: "draft-reply",
        label: "Gemma 4 Draft Reply",
        objective:
          "Analyze the email subject and content payload, write a highly professional, polite reply to the sender, and format it clearly.",
        context: "{{nodes.read-unread-emails.output}}",
      }),
      permissionNode({
        id: "approve-email-reply",
        label: "Approve Email Reply",
        permission: "mcp_tool",
        reason: "Verify generated reply contents before opening the Mail draft.",
      }),
      mcpToolNode({
        id: "draft-outgoing-email",
        label: "Draft Outgoing Email",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "draft_system_email",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.draftSystemEmail,
        args: {
          to: "{{nodes.read-unread-emails.output.data.structuredContent.emails.0.sender}}",
          subject: "Re: {{nodes.read-unread-emails.output.data.structuredContent.emails.0.subject}}",
          body: "{{nodes.draft-reply.output.data}}",
          cc: "",
          bcc: "",
        },
      }),
      outputNode("{{nodes.draft-outgoing-email.output}}"),
      outputNode(
        "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges(["input", "read-unread-emails", "mail-has-messages"]),
      edge("mail-has-messages", "true", "draft-reply"),
      edge("mail-has-messages", "false", "empty-output"),
      edge("draft-reply", "out", "approve-email-reply"),
      edge("approve-email-reply", "approved", "draft-outgoing-email"),
      edge("draft-outgoing-email", "out", "output"),
    ],
  }),
  createWorkflowIrTemplate({
    id: "calendar-assistant",
    name: "Calendar Assistant",
    description:
      "Scans upcoming local calendar events, asks Gemma 4 for high-priority briefings, and displays native macOS notifications.",
    seedPrompt:
      "Scan my upcoming local calendar events, summarize high-priority meetings and deadlines, and show a native notification.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "calendar-assistant-read",
        label: "Read macOS Calendar",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_calendar",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemCalendar,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.events,
        args: {
          calendar_name: "",
          hours_ahead: 24,
          start_date: "",
          end_date: "",
        },
        timeoutMs: CALENDAR_WORKFLOW_TIMEOUT_MS,
      }),
      conditionalNode({
        id: "calendar-assistant-has-events",
        label: "workflows.templates.empty_result.check_events",
        condition: "$ != []",
        inputMapping:
          "{{nodes.calendar-assistant-read.output.data.structuredContent.events}}",
      }),
      agentNode({
        id: "meeting-audit",
        label: "Gemma 4 Meeting Audit",
        objective:
          "Read the upcoming meetings and deadlines from the macOS calendar payload and outline key priorities.",
        context: "{{nodes.calendar-assistant-read.output}}",
      }),
      mcpToolNode({
        id: "calendar-notification",
        label: "Trigger System Notification",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "trigger_system_notification",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.triggerSystemNotification,
        args: {
          title_text: "OOMU Calendar Assistant",
          subtitle_text: "Upcoming Meetings",
          body_text: "{{nodes.meeting-audit.output.data}}",
        },
      }),
      outputNode("{{nodes.calendar-notification.output}}"),
      outputNode(
        "{{nodes.calendar-assistant-read.output.data.structuredContent.events}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges(["input", "calendar-assistant-read", "calendar-assistant-has-events"]),
      edge("calendar-assistant-has-events", "true", "meeting-audit"),
      edge("calendar-assistant-has-events", "false", "empty-output"),
      ...linearEdges(["meeting-audit", "calendar-notification", "output"]),
    ],
  }),
  createWorkflowIrTemplate({
    id: "daily-mail-reminders-scraper",
    name: "Daily Mail and Reminders Scraper",
    description:
      "Reads recent Mail and Reminders through local AppleScript, summarizes the day with Gemma 4, and writes the brief to the secure sandbox.",
    seedPrompt:
      "Read recent macOS Mail and Reminders, summarize urgent messages and open tasks, then write the daily brief to the secure sandbox.",
    nodes: [
      inputNode(),
      mcpToolNode({
        id: "daily-scraper-mail",
        label: "Read macOS Mail",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_emails",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemEmails,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.emails,
        args: {
          max_messages: 10,
          unread_only: false,
        },
      }),
      mcpToolNode({
        id: "daily-scraper-reminders",
        label: "Read Reminders",
        serverName: MACOS_APPLESCRIPT_SERVER_NAME,
        toolName: "read_system_reminders",
        inputSchema: MACOS_APPLESCRIPT_TOOL_SCHEMAS.readSystemReminders,
        outputSchema: COLLECTION_OUTPUT_SCHEMAS.reminders,
        args: {
          list_name: "",
          completed_only: false,
        },
      }),
      conditionalNode({
        id: "daily-mail-has-messages",
        label: "workflows.templates.empty_result.check_mail",
        condition: "$ != []",
        inputMapping:
          "{{nodes.daily-scraper-mail.output.data.structuredContent.emails}}",
      }),
      conditionalNode({
        id: "daily-reminders-have-items",
        label: "workflows.templates.empty_result.check_reminders",
        condition: "$ != []",
        inputMapping:
          "{{nodes.daily-scraper-reminders.output.data.structuredContent.reminders}}",
      }),
      agentNode({
        id: "daily-capture",
        label: "Gemma 4 Daily Capture",
        objective:
          "Use only the Mail and Reminders payloads. Summarize urgent messages, open tasks, and one recommended next action. Do not invent missing details.",
        inputMappings: {
          mail: "{{nodes.daily-scraper-mail.output}}",
          reminders: "{{nodes.daily-scraper-reminders.output}}",
        },
      }),
      mcpToolNode({
        id: "write-daily-brief",
        label: "Write Daily Brief",
        serverName: LOCAL_FILESYSTEM_SERVER_NAME,
        toolName: "write_file",
        inputSchema: LOCAL_FILESYSTEM_TOOL_SCHEMAS.writeFile,
        args: {
          path: LOCAL_SANDBOX_DAILY_BRIEF_PATH,
          content: "{{nodes.daily-capture.output.data}}",
        },
      }),
      outputNode("{{nodes.write-daily-brief.output}}"),
      outputNode(
        "{{nodes.daily-scraper-reminders.output.data.structuredContent.reminders}}",
        { id: "empty-output", completionKind: "empty_collection" },
      ),
    ],
    edges: [
      ...linearEdges([
        "input",
        "daily-scraper-mail",
        "daily-scraper-reminders",
        "daily-mail-has-messages",
      ]),
      edge("daily-mail-has-messages", "true", "daily-capture"),
      edge("daily-mail-has-messages", "false", "daily-reminders-have-items"),
      edge("daily-reminders-have-items", "true", "daily-capture"),
      edge("daily-reminders-have-items", "false", "empty-output"),
      ...linearEdges(["daily-capture", "write-daily-brief", "output"]),
    ],
  }),
];

export type WorkflowTemplateId = (typeof workflowTemplates)[number]["id"];

export function workflowTemplateById(id: WorkflowTemplateId) {
  return workflowTemplates.find((template) => template.id === id);
}

export function localizeWorkflowIrTemplate(
  template: WorkflowIrTemplateExample,
  t: TemplateTranslate,
  { sourceTruncated = false }: { sourceTruncated?: boolean } = {},
) {
  const localized = localizeTemplateCopy(clone(template), t) as WorkflowIrTemplateExample;
  if (localized.id === "directory-summarizer" && sourceTruncated) {
    localized.workflowIr.nodes = localized.workflowIr.nodes.map((node) =>
      node.kind === "agent" && node.id === "summarize-folder"
        ? {
            ...node,
            objective: `${node.objective}\n\n${t(
              "workflows.templates.directory-summarizer.truncation_objective",
            )}`,
          }
        : node,
    );
  }
  return localized;
}

function localizeTemplateCopy(value: unknown, t: TemplateTranslate): unknown {
  if (typeof value === "string") {
    return value.startsWith("workflows.templates.") ? t(value) : value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => localizeTemplateCopy(item, t));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, localizeTemplateCopy(item, t)]),
    );
  }
  return value;
}

export function instantiateWorkflowIrTemplate(
  template: WorkflowIrTemplateExample,
  workflowId: string,
) {
  const workflowIr = clone(template.workflowIr);
  return workflowIrSchema.parse({
    ...workflowIr,
    workflowId,
    workflowVersion: 1,
    metadata: {
      ...(asRecord(workflowIr.metadata)),
      templateId: template.id,
      seedPrompt: template.seedPrompt,
    },
  });
}

export function bindWorkflowSourceFolder(workflowIr: WorkflowIr, folderPath: string) {
  if (!/^workspace\/selections\/[A-Za-z0-9_-]+$/.test(folderPath)) {
    throw new Error("workflow_source_folder_path_invalid");
  }
  return {
    ...workflowIr,
    nodes: workflowIr.nodes.map((node) =>
      node.kind === "mcp_tool" && node.id === "read-approved-folder"
        ? { ...node, arguments: { folderPath } }
        : node,
    ),
  };
}

export function humanizeToolName(toolName: string) {
  const cleaned = toolName.replace(/[_-]+/g, " ").replace(/\s+/g, " ").trim();
  if (!cleaned) {
    return toolName;
  }
  return cleaned.replace(/\b\w/g, (character) => character.toUpperCase());
}

function createWorkflowIrTemplate<const TemplateId extends string>({
  description,
  edges,
  id,
  name,
  nodes,
  seedPrompt,
}: {
  description: string;
  edges: WorkflowIrEdge[];
  id: TemplateId;
  name: string;
  nodes: WorkflowIrNode[];
  seedPrompt: string;
}) {
  const workflowIr = workflowIrSchema.parse({
    schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
    workflowId: `template:${id}`,
    workflowVersion: 1,
    name,
    description,
    compiler: { model: WORKFLOW_COMPILER_MODEL },
    metadata: { templateId: id, seedPrompt },
    nodes,
    edges,
  });
  return { description, id, name, seedPrompt, workflowIr };
}

function inputNode(): WorkflowIrNode {
  return {
    kind: "input",
    id: "input",
    label: "Workflow Input",
    outputKey: "workflow.input",
    inputSchema: OBJECT_SCHEMA,
  };
}

function outputNode(
  inputMapping: string,
  {
    completionKind = "result",
    id = "output",
  }: {
    completionKind?: "result" | "empty_collection";
    id?: string;
  } = {},
): WorkflowIrNode {
  return {
    kind: "output",
    id,
    label:
      completionKind === "empty_collection"
        ? "workflows.templates.empty_result.label"
        : "Workflow Output",
    inputMapping,
    outputSchema:
      completionKind === "empty_collection"
        ? { type: "array", items: {} }
        : OBJECT_SCHEMA,
    completionKind,
  };
}

function agentNode({
  context,
  id,
  inputMappings,
  label,
  objective,
}: {
  context?: string;
  id: string;
  inputMappings?: Record<string, string>;
  label: string;
  objective: string;
}): WorkflowIrNode {
  const mappings = inputMappings ?? (context ? { context } : {});
  return {
    kind: "agent",
    id,
    label,
    objective,
    inputMappings: mappings,
    outputKey: `nodes.${id}.output`,
    systemTimeoutMs: MEDIUM_TIMEOUT_MS,
  };
}

function conditionalNode({
  condition,
  id,
  inputMapping,
  label,
}: {
  condition: string;
  id: string;
  inputMapping: string;
  label: string;
}): WorkflowIrNode {
  return {
    kind: "conditional",
    id,
    label,
    condition,
    inputMapping,
    systemTimeoutMs: MEDIUM_TIMEOUT_MS,
  };
}

function permissionNode({
  id,
  label,
  permission,
  reason,
}: {
  id: string;
  label: string;
  permission: Extract<WorkflowIrNode, { kind: "permission" }>["permission"];
  reason: string;
}): WorkflowIrNode {
  return {
    kind: "permission",
    id,
    label,
    permission,
    reason,
    onDenied: "fail",
  };
}

function mcpToolNode({
  args,
  id,
  inputSchema,
  label,
  outputSchema,
  serverName,
  timeoutMs = MEDIUM_TIMEOUT_MS,
  toolName,
}: {
  args: Record<string, unknown>;
  id: string;
  inputSchema: unknown;
  label: string;
  outputSchema?: unknown;
  serverName: string;
  timeoutMs?: number;
  toolName: string;
}): WorkflowIrNode {
  return {
    kind: "mcp_tool",
    id,
    label,
    serverName,
    toolName,
    arguments: args,
    inputSchema,
    outputSchema,
    systemTimeoutMs: timeoutMs,
  };
}

function linearEdges(nodeIds: string[]) {
  return nodeIds.slice(0, -1).map((sourceNodeId, index) =>
    edge(sourceNodeId, "out", nodeIds[index + 1]),
  );
}

function edge(
  sourceNodeId: string,
  sourcePort: string,
  targetNodeId: string,
): WorkflowIrEdge {
  return {
    id: `edge-${sourceNodeId}-${sourcePort}-${targetNodeId}`,
    sourceNodeId,
    sourcePort,
    targetNodeId,
  };
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
