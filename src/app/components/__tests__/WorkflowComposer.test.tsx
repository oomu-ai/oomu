import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { WorkflowComposer } from "../WorkflowComposer";

const appShellMock = vi.hoisted(() => ({
  setActiveItem: vi.fn(),
  setWorkflowDraft: vi.fn(),
  setWorkflowsView: vi.fn(),
  workflowProjectScope: null as {
    projectId: string;
    projectName: string;
  } | null,
}));

const invokeMock = vi.hoisted(() => vi.fn());

const capabilityCatalog = {
  authoringEnabled: true,
  generatedAtMs: 1,
  actions: [],
  templates: [],
  version: "2026-06-29.p2",
};

const localeState = {
  activeLocale: "en-US",
  availableLocales: [
    {
      fileName: "en-US.json",
      id: "en-US",
      isDefault: true,
      label: "English (US)",
      verified: true,
    },
  ],
  translations: {},
};

function catalogOrLocale(command: string) {
  return command === "get_workflow_capability_catalog" ? capabilityCatalog : localeState;
}

const composedWorkflowIr = {
  schemaVersion: "1.0.0",
  workflowId: "wf-test-daily-brief",
  workflowVersion: 1,
  name: "Daily Brief",
  description: "Read my calendar and draft a daily brief.",
  compiler: { model: "gemma-4-e2b-qat" },
  nodes: [
    {
      kind: "input",
      id: "input",
      label: "Workflow Input",
      outputKey: "workflow.input",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      kind: "agent",
      id: "summarize",
      label: "Summarize Calendar",
      objective: "Draft a concise daily brief from calendar context.",
      inputMappings: { context: "{{workflow.input}}" },
      outputKey: "nodes.summarize.output",
      systemTimeoutMs: 30000,
    },
    {
      kind: "output",
      id: "output",
      label: "Workflow Output",
      inputMapping: "{{nodes.summarize.output}}",
      outputSchema: { type: "object", additionalProperties: true },
    },
  ],
  edges: [
    {
      id: "edge-input-summarize",
      sourceNodeId: "input",
      sourcePort: "out",
      targetNodeId: "summarize",
    },
    {
      id: "edge-summarize-output",
      sourceNodeId: "summarize",
      sourcePort: "out",
      targetNodeId: "output",
    },
  ],
};

const previewOnlyWorkflowIr = {
  schemaVersion: "1.0.0",
  workflowId: "wf-preview-gap",
  workflowVersion: 1,
  name: "Preview Gap",
  description: "Summarize a folder and open the report.",
  compiler: { model: "gemma-4-e2b-qat" },
  nodes: [
    {
      kind: "input",
      id: "input",
      label: "Workflow Input",
      outputKey: "workflow.input",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      kind: "agent",
      id: "summary",
      label: "Summary",
      objective: "Summarize the scanned folder.",
      inputMappings: { context: "{{workflow.input}}" },
      outputKey: "nodes.summary.output",
      systemTimeoutMs: 30000,
    },
    {
      kind: "mcp_tool",
      id: "preview-report",
      label: "Preview Report",
      serverName: "taskflow_native",
      toolName: "preview_report",
      arguments: { reportPath: "workspace/report.md" },
      inputSchema: {
        type: "object",
        properties: { reportPath: { type: "string" } },
        required: ["reportPath"],
        additionalProperties: false,
      },
      systemTimeoutMs: 10000,
    },
    {
      kind: "output",
      id: "output",
      label: "Workflow Output",
      inputMapping: "{{nodes.preview-report.output}}",
      outputSchema: { type: "object", additionalProperties: true },
    },
  ],
  edges: [
    {
      id: "edge-input-summary",
      sourceNodeId: "input",
      sourcePort: "out",
      targetNodeId: "summary",
    },
    {
      id: "edge-summary-preview",
      sourceNodeId: "summary",
      sourcePort: "out",
      targetNodeId: "preview-report",
    },
    {
      id: "edge-preview-output",
      sourceNodeId: "preview-report",
      sourcePort: "out",
      targetNodeId: "output",
    },
  ],
};

const emailReviewWorkflowIr = {
  schemaVersion: "1.0.0",
  workflowId: "wf-email-review",
  workflowVersion: 1,
  name: "Email responder",
  description: "Read unread mail, draft replies, ask first, then open a draft.",
  compiler: { model: "gemma-4-e2b-qat" },
  nodes: [
    {
      kind: "input",
      id: "input",
      label: "Workflow Input",
      outputKey: "workflow.input",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      kind: "mcp_tool",
      id: "read-mail",
      label: "Read unread mail",
      serverName: "macos_applescript",
      toolName: "read_system_emails",
      arguments: {},
      inputSchema: { type: "object", additionalProperties: false },
      systemTimeoutMs: 10000,
    },
    {
      kind: "agent",
      id: "draft-reply",
      label: "Draft reply",
      objective: "Draft a concise reply from the unread message.",
      inputMappings: { context: "{{nodes.read-mail.output}}" },
      outputKey: "nodes.draft-reply.output",
      systemTimeoutMs: 30000,
    },
    {
      kind: "permission",
      id: "approve-draft",
      label: "Ask before opening",
      permission: "mcp_tool",
      reason: "opening the Mail draft.",
      onDenied: "fail",
    },
    {
      kind: "mcp_tool",
      id: "open-draft",
      label: "Open Mail draft",
      serverName: "macos_applescript",
      toolName: "draft_system_email",
      arguments: {},
      inputSchema: { type: "object", additionalProperties: false },
      systemTimeoutMs: 10000,
    },
    {
      kind: "output",
      id: "output",
      label: "Workflow Output",
      inputMapping: "{{nodes.open-draft.output}}",
      outputSchema: { type: "object", additionalProperties: true },
    },
  ],
  edges: [
    { id: "edge-input-read", sourceNodeId: "input", sourcePort: "out", targetNodeId: "read-mail" },
    { id: "edge-read-draft", sourceNodeId: "read-mail", sourcePort: "out", targetNodeId: "draft-reply" },
    { id: "edge-draft-approve", sourceNodeId: "draft-reply", sourcePort: "out", targetNodeId: "approve-draft" },
    { id: "edge-approve-open", sourceNodeId: "approve-draft", sourcePort: "approved", targetNodeId: "open-draft" },
    { id: "edge-open-output", sourceNodeId: "open-draft", sourcePort: "out", targetNodeId: "output" },
  ],
};

vi.mock("@/components/AppShell", () => ({
  useAppShell: () => ({
    setActiveItem: appShellMock.setActiveItem,
    setWorkflowDraft: appShellMock.setWorkflowDraft,
    setWorkflowsView: appShellMock.setWorkflowsView,
    workflowProjectScope: appShellMock.workflowProjectScope,
  }),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: false,
}));

describe("WorkflowComposer", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    appShellMock.setActiveItem.mockReset();
    appShellMock.setWorkflowDraft.mockReset();
    appShellMock.setWorkflowsView.mockReset();
    appShellMock.workflowProjectScope = null;
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: composedWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          attempts: 1,
          latencyMs: 0,
        });
      }
      if (command === "choose_workflow_source_folder") {
        return Promise.resolve({
          fileCount: 3,
          folderName: "Case files",
          folderPath: "workspace/selections/selection-case-files",
          totalBytes: 1200,
          truncated: false,
        });
      }
      return Promise.resolve(catalogOrLocale(command));
    });
  });

  it("loads a requested workflow template by its stable external ID", async () => {
    const onRequestedTemplateLoaded = vi.fn();
    render(
      <I18nProvider>
        <WorkflowComposer
          onRequestedTemplateLoaded={onRequestedTemplateLoaded}
          requestedTemplateId="directory-summarizer"
          requestedTemplateSourceFolder={{
            fileCount: 3,
            folderName: "Case files",
            folderPath: "workspace/selections/selection-case-files",
            totalBytes: 1200,
            truncated: false,
          }}
        />
      </I18nProvider>,
    );

    expect(
      await screen.findByText(
        'Loaded the "Directory Summarizer" workflow for review.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("What will happen")).toBeInTheDocument();
    expect(screen.queryByText("Steps")).not.toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Edit steps" }));
    expect(screen.getByText("Steps")).toBeInTheDocument();
    expect(screen.getByLabelText("What should this workflow do?")).toHaveValue(
      "Read the folder I chose, summarize the important notes and text files without inventing details, ask before saving the report, then open it for review.",
    );
    expect(
      invokeMock.mock.calls.some(([command]) => command === "compose_workflow"),
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some(
        ([command]) => command === "choose_workflow_source_folder",
      ),
    ).toBe(false);
    expect(onRequestedTemplateLoaded).toHaveBeenCalledOnce();
    expect(onRequestedTemplateLoaded).toHaveBeenCalledWith(
      "directory-summarizer",
    );
    expect(appShellMock.setWorkflowDraft).toHaveBeenCalledWith(null);
  });

  it("keeps template-card loading on the same stable ID path", async () => {
    const user = userEvent.setup();
    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.click(
      screen.getByRole("button", {
        name: "Start from the Directory Summarizer template",
      }),
    );

    expect(
      await screen.findByText(
        'Loaded the "Directory Summarizer" workflow for review.',
      ),
    ).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("choose_workflow_source_folder", {
      selectionId: expect.stringMatching(/^selection-[a-z0-9-]+$/i),
      title: "Choose a folder to summarize",
      truncationNotice:
        "This selection was larger than OOMU can read in one pass. The summary must say it covers only the files included in this run.",
    });
  });

  it("discloses a partial folder selection before the workflow can run", async () => {
    render(
      <I18nProvider>
        <WorkflowComposer
          requestedTemplateId="directory-summarizer"
          requestedTemplateSourceFolder={{
            fileCount: 50,
            folderName: "Large case file",
            folderPath: "workspace/selections/selection-large-case-file",
            totalBytes: 524288,
            truncated: true,
          }}
        />
      </I18nProvider>,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      'Loaded the "Directory Summarizer" template. Some files were left out because the folder was too large for one run; the report will say so.',
    );
  });

  it("keeps native folder failures off the review surface", async () => {
    const nativeCanary = "BACKEND CANARY: source_path_escape_code_91";
    invokeMock.mockImplementation((command: string) =>
      command === "choose_workflow_source_folder"
        ? Promise.reject(new Error(nativeCanary))
        : Promise.resolve(catalogOrLocale(command)),
    );
    const user = userEvent.setup();
    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.click(
      screen.getByRole("button", {
        name: "Start from the Directory Summarizer template",
      }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That folder couldn't be prepared. Choose one with readable notes or text files, then try again.",
    );
    expect(screen.queryByText(/BACKEND CANARY|source_path_escape_code_91/i)).toBeNull();
  });

  it("lands on a four-beat story and reveals the full editor only on request", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: emailReviewWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          attempts: 1,
          latencyMs: 0,
        });
      }
      return Promise.resolve(catalogOrLocale(command));
    });
    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Read unread mail, draft replies, ask first, then open a draft.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));

    expect(
      await screen.findByText("OOMU drafted a workflow for review."),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow name")).toHaveValue("Email responder");
    const story = screen.getByRole("list", { name: "Workflow summary" });
    expect(within(story).getAllByRole("listitem")).toHaveLength(4);
    expect(screen.getByText("What will happen")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save and run" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit steps" })).toBeInTheDocument();
    expect(screen.queryByText("Steps")).not.toBeInTheDocument();
    expect(screen.queryByText("Start with your request")).not.toBeInTheDocument();
    expect(screen.queryByText("Return the result")).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue("workflow.input")).not.toBeInTheDocument();
    const approvalBeat = screen
      .getByText("Pauses for your approval before opening the Mail draft.")
      .closest("li");
    expect(approvalBeat).toHaveClass("bg-[var(--accent-background)]");

    await user.click(screen.getByRole("button", { name: "Edit steps" }));
    expect(screen.getByText("Steps")).toBeInTheDocument();
    expect(screen.queryByText("What will happen")).not.toBeInTheDocument();
    expect(screen.queryByText("Writing")).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue("workflow.input")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Done editing" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Inspect" }));
    expect(screen.getByText("Workflow plumbing")).toBeInTheDocument();
    await user.click(screen.getByText("Workflow Input", { selector: "summary" }));
    expect(screen.getByDisplayValue("workflow.input")).toBeInTheDocument();
    expect(
      invokeMock.mock.calls.some(([command]) => command === "compose_workflow"),
    ).toBe(true);
  });

  it("keeps the user's exact workflow name through composition, review, and save", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: composedWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          attempts: 1,
          latencyMs: 0,
        });
      }
      if (command === "save_workflow") {
        return Promise.resolve({
          workflowId: composedWorkflowIr.workflowId,
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: composedWorkflowIr.nodes.length,
        });
      }
      if (command === "get_compiled_instructions") {
        return Promise.resolve([]);
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    const name = screen.getByLabelText("Workflow name");
    expect(name).toHaveValue("");
    expect(
      screen.getByText("Leave this blank and OOMU will suggest a name."),
    ).toBeVisible();

    await user.type(name, "Working title");
    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Read my calendar and draft a daily brief.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));

    await screen.findByText("OOMU drafted a workflow for review.");
    expect(name).toHaveValue("Working title");
    expect(screen.getByRole("heading", { name: "Working title" })).toBeVisible();

    await user.clear(name);
    expect(name).toHaveAttribute("aria-invalid", "true");
    expect(name).toHaveAttribute("aria-required", "true");
    expect(
      screen.getByText("Add a name before saving or running this workflow."),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save and run" })).toBeDisabled();

    const exactName = "Ship Test 05 — Morning Operations Brief";
    await user.type(name, exactName);
    expect(screen.getByRole("heading", { name: exactName })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(([command]) => command === "save_workflow"),
      ).toBe(true);
    });
    const saveCall = invokeMock.mock.calls.find(
      ([command]) => command === "save_workflow",
    )?.[1] as {
      request: {
        workflow: { name: string };
        workflowIr: { name: string };
        visualState: { workflowIr: { name: string } };
      };
    };
    expect(saveCall.request.workflow.name).toBe(exactName);
    expect(saveCall.request.workflowIr.name).toBe(exactName);
    expect(saveCall.request.visualState.workflowIr.name).toBe(exactName);
    expect(
      await screen.findByText(`"${exactName}" is saved and ready to run.`),
    ).toBeVisible();
  });

  it("keeps Project-launched authoring bound through composition and save", async () => {
    const user = userEvent.setup();
    appShellMock.workflowProjectScope = {
      projectId: "project-1",
      projectName: "Launch",
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: composedWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          attempts: 1,
          latencyMs: 0,
        });
      }
      if (command === "save_workflow") {
        return Promise.resolve({
          workflowId: composedWorkflowIr.workflowId,
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: composedWorkflowIr.nodes.length,
        });
      }
      if (command === "get_compiled_instructions") return Promise.resolve([]);
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    expect(await screen.findByText("Project Workflow · Launch")).toBeVisible();
    expect(
      screen.getByText(
        "This Workflow can use only Launch’s approved knowledge and locations.",
      ),
    ).toBeVisible();
    await user.type(screen.getByLabelText("Workflow name"), "Launch brief");
    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Prepare the Launch brief.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));
    await screen.findByText("OOMU drafted a workflow for review.");
    await user.click(screen.getByRole("button", { name: "Save" }));

    const composeCall = invokeMock.mock.calls.find(
      ([command]) => command === "compose_workflow",
    )?.[1] as { request: { projectId: string | null } };
    const saveCall = invokeMock.mock.calls.find(
      ([command]) => command === "save_workflow",
    )?.[1] as {
      request: {
        projectId: string | null;
        visualState: { projectId: string | null };
      };
    };
    expect(composeCall.request.projectId).toBe("project-1");
    expect(saveCall.request.projectId).toBe("project-1");
    expect(saveCall.request.visualState.projectId).toBe("project-1");
  });

  it("shows workflow run failures once in a readable inline notice", async () => {
    const user = userEvent.setup();
    const rawTransportError =
      "MCP server 'macos_applescript' disconnected. A fresh one-use Shield Gate approval is required before reconnecting the non-native transport.";
    invokeMock.mockImplementation((command: string) => {
      if (command === "save_workflow") {
        return Promise.resolve({
          workflowId: "email-responder",
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: 4,
        });
      }
      if (command === "get_compiled_instructions") {
        return Promise.resolve([]);
      }
      if (command === "run_workflow") {
        return Promise.reject(new Error(rawTransportError));
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer requestedTemplateId="email-responder" />
      </I18nProvider>,
    );

    await screen.findByText('Loaded the "Email Responder" workflow for review.');
    await user.click(screen.getByRole("button", { name: "Save and run" }));

    const message =
      'Couldn\'t run "Email Responder". OOMU couldn\'t reach the Apple app this workflow needs. Try again.';
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(message);
    expect(screen.getAllByText(message)).toHaveLength(1);
    expect(screen.queryByText(rawTransportError)).toBeNull();
  });

  it("does not expose collection-contract diagnostics in the authoring UI", async () => {
    const user = userEvent.setup();
    const rawDiagnostic =
      "The Nothing found step must use the exact collection declared by its read step before it can report that nothing was found.";
    invokeMock.mockImplementation((command: string) => {
      if (command === "save_workflow") {
        return Promise.reject(
          Object.assign(new Error(rawDiagnostic), {
            code: "workflow_topological_anomaly_unsafe_collection_access",
          }),
        );
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer requestedTemplateId="email-responder" />
      </I18nProvider>,
    );

    await screen.findByText('Loaded the "Email Responder" workflow for review.');
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByText(
        "Couldn't save the reviewed workflow: OOMU couldn't finish checking this workflow. Try Describe again.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(rawDiagnostic)).toBeNull();
    expect(
      screen.queryByText(/workflow_topological_anomaly_unsafe_collection_access/),
    ).toBeNull();
  });

  it("lets the user approve a typed workflow permission and continues the run", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation((command: string) => {
      if (command === "save_workflow") {
        return Promise.resolve({
          workflowId: "email-responder",
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: 5,
        });
      }
      if (command === "get_compiled_instructions") {
        return Promise.resolve([]);
      }
      if (command === "run_workflow") {
        return Promise.resolve({
          instance: {
            id: "instance-email-review",
            workflowId: "email-responder",
            workflowVersion: 1,
            status: "AwaitingApproval",
            nodePayloads: {},
          },
          executionOrder: ["read-unread-emails", "draft-reply"],
          approvalRequest: {
            instanceId: "instance-email-review",
            workflowId: "email-responder",
            nodeId: "approve-email-reply",
            message: "Verify generated reply contents before opening the Mail draft.",
            context: {
              actionType: "workflow_permission",
              permissionKind: "mcp_tool",
              actionLabel: "Approve Email Reply",
              capabilityReason: "Verify generated reply contents before opening the Mail draft.",
            },
            approvalToken: "approval-token",
            approveCommand: {},
            rejectCommand: {},
          },
        });
      }
      if (command === "resolve_workflow_permission") {
        return Promise.resolve({
          instance: {
            id: "instance-email-review",
            workflowId: "email-responder",
            workflowVersion: 1,
            status: "Completed",
            nodePayloads: {},
          },
          executionOrder: ["approve-email-reply"],
        });
      }
      if (command === "update_workflow_last_run") {
        return Promise.resolve(true);
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer requestedTemplateId="email-responder" />
      </I18nProvider>,
    );

    await screen.findByText('Loaded the "Email Responder" workflow for review.');
    await user.click(screen.getByRole("button", { name: "Save and run" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Use a connected tool")).toBeVisible();
    expect(within(dialog).getByText(/Approve Email Reply/)).toBeVisible();
    const approve = within(dialog).getByRole("button", { name: "Approve" });
    expect(approve).toBeEnabled();

    await user.click(approve);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resolve_workflow_permission",
      {
        request: {
          instanceId: "instance-email-review",
          approvalToken: "approval-token",
          decision: "approve",
        },
      },
    ));
  });

  it("inserts a missing report save step and retries save after topology warning", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: previewOnlyWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          composedBy: "gemma",
          attempts: 1,
          latencyMs: 0,
        });
      }
      if (command === "save_workflow") {
        const saveCalls = invokeMock.mock.calls.filter(
          ([calledCommand]) => calledCommand === "save_workflow",
        );
        if (saveCalls.length === 1) {
          return Promise.reject(
            Object.assign(
              new Error(
                "This workflow wants to open a report for review, but it needs to save the report to disk first.",
              ),
              {
                code: "workflow_topological_anomaly_missing_report_writer",
              },
            ),
          );
        }
        return Promise.resolve({
          workflowId: "wf-preview-gap",
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: 1,
        });
      }
      if (command === "get_compiled_instructions") {
        return Promise.resolve([]);
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Summarize a folder and open the report.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));
    await screen.findByText("OOMU drafted a workflow for review.");

    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText(
        "This workflow wants to open a report for review, but we need to save the report to disk first.",
      ),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Insert Save Step" }));
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "save_workflow"),
      ).toHaveLength(2);
    });

    const repairedSave = invokeMock.mock.calls
      .filter(([command]) => command === "save_workflow")
      .at(-1)?.[1] as {
      request: { workflowIr: typeof previewOnlyWorkflowIr };
    };
    expect(repairedSave.request.workflowIr.nodes).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: "mcp_tool",
          id: "write-report",
          serverName: "taskflow_native",
          toolName: "write_markdown_report",
          arguments: expect.objectContaining({
            reportPath: "workspace/report.md",
            content: "{{nodes.summary.output}}",
          }),
        }),
      ]),
    );
    expect(repairedSave.request.workflowIr.edges).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "edge-summary-preview",
          sourceNodeId: "summary",
          targetNodeId: "write-report",
        }),
        expect.objectContaining({
          sourceNodeId: "write-report",
          targetNodeId: "preview-report",
        }),
      ]),
    );
    expect(
      await screen.findByText("\"Preview Gap\" is saved and ready to run."),
    ).toBeInTheDocument();
    expect(screen.getByText("Workflow saved")).toBeInTheDocument();
    expect(screen.queryByText("Ready for review")).not.toBeInTheDocument();
  });

  it("renders actionable connection guidance for missing capabilities", async () => {
    const user = userEvent.setup();
    let composeCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        composeCount += 1;
        if (composeCount === 1) {
          return Promise.resolve({
            status: "needs_connection",
            reason: "Connect slack to use post_message for Post a message to Slack.",
            workflowIr: null,
            partialDraft: null,
            missingCapabilities: ["Post Slack Message"],
            missingCapabilityDetails: [
              {
                id: "mcp:slack:post_message",
                title: "Post Slack Message",
                outcome: "Post a message to Slack.",
                reason:
                  "Connect slack to use post_message for Post a message to Slack.",
                source: "mcp",
                serverName: "slack",
                toolName: "post_message",
              },
            ],
            composedBy: "gemma",
            attempts: 1,
            latencyMs: 0,
          });
        }
        return Promise.resolve({
          status: "composed",
          reason: "Composed without Slack.",
          workflowIr: composedWorkflowIr,
          partialDraft: null,
          missingCapabilities: [],
          missingCapabilityDetails: [],
          composedBy: "gemma",
          attempts: 1,
          latencyMs: 0,
        });
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Post the daily brief to Slack.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));

    expect(await screen.findByText("Post Slack Message")).toBeInTheDocument();
    expect(screen.getByText("Post a message to Slack.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open Mods" }));
    expect(appShellMock.setActiveItem).toHaveBeenCalledWith("mods");

    await user.click(
      screen.getByRole("button", { name: "Build it without this step" }),
    );
    expect(
      await screen.findByText("OOMU drafted a workflow for review."),
    ).toBeInTheDocument();
    const composeCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "compose_workflow",
    );
    expect(composeCalls).toHaveLength(2);
    expect(composeCalls[1]?.[1]).toMatchObject({
      request: {
        prompt: expect.stringContaining('Build it without the "Post Slack Message" step.'),
      },
    });
  });

  it("shows an honest error instead of an unsaveable draft when the engine returns an invalid IR", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation((command: string) => {
      if (command === "compose_workflow") {
        // Mirrors the desktop engine before the serde fix: a `null` targetPort that
        // passed Rust validation but fails the save-time schema. The composer must
        // refuse it up front rather than show a draft that can never be saved.
        return Promise.resolve({
          status: "composed",
          reason: "Composed",
          workflowIr: {
            ...composedWorkflowIr,
            edges: [
              { ...composedWorkflowIr.edges[0], targetPort: null },
              composedWorkflowIr.edges[1],
            ],
          },
          partialDraft: null,
          missingCapabilities: [],
          composedBy: "gemma",
          attempts: 1,
          latencyMs: 0,
        });
      }
      return Promise.resolve(catalogOrLocale(command));
    });

    render(
      <I18nProvider>
        <WorkflowComposer />
      </I18nProvider>,
    );

    await user.type(
      screen.getByLabelText("What should this workflow do?"),
      "Read my calendar and draft a daily brief.",
    );
    await user.click(screen.getByRole("button", { name: "Describe" }));

    expect(
      await screen.findByText(
        "The steps OOMU returned were incomplete. Try a simpler request, or start from a template and edit the steps.",
      ),
    ).toBeInTheDocument();
    // No dead-end draft: the review storyboard must not render.
    expect(screen.queryByText("Steps")).not.toBeInTheDocument();
    expect(
      screen.queryByText("OOMU drafted a workflow for review."),
    ).not.toBeInTheDocument();
  });
});
