import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  instantiateWorkflowIrTemplate,
  workflowTemplateById,
} from "../workflowLibrary";
import { plannedRoutineWorkflowAttachment } from "./routineTargetWorkflow";
import type { RoutineDraft, RoutineHandoffRequest } from "./routineDraft";
import { RoutinesScreen } from "./RoutinesScreen";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  setRoutineDraft: vi.fn(),
  shell: { routineDraft: null as RoutineDraft | null },
  t: (key: string) => ({
    "routines.confirm": "Schedule it",
    "routines.handoff_project_description":
      "Private Project scope for schedules created from Chat.",
    "routines.handoff_project_name": "Scheduled tasks",
    "routines.handoff_workflow_description":
      "Runs the exact task requested in Chat.",
    "routines.handoff_workflow_name": "Scheduled task",
    "routines.project": "Project",
    "routines.workflow": "Workflow",
  } as Record<string, string>)[key] ?? key,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: unknown) => mocks.invoke(command, args),
}));
vi.mock("@/components/AppShell", () => ({
  useAppShell: () => ({
    routineDraft: mocks.shell.routineDraft,
    setRoutineDraft: mocks.setRoutineDraft,
    setActiveItem: vi.fn(),
    setWorkflowProjectScope: vi.fn(),
    setWorkflowsView: vi.fn(),
  }),
}));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: mocks.t,
  }),
}));

function backgroundStatus() {
  return {
    userEnabled: false, verifiedActive: false, state: "off",
    registrationState: "unregistered", registrationBackend: "supervised_process",
    processState: "absent", processId: null, buildNumber: 1,
    buildIdentity: "dev", profileClass: "development",
    profileGenerationSha256: "profile", heartbeatAtMs: null,
    heartbeatAgeMs: null, menuVisible: false, errorCode: null,
    detail: "Off", checkedAtMs: 1, recentReceipts: [],
  };
}

function prepareGenericComposition() {
  const request: RoutineHandoffRequest = {
    requestText: "Cada día, revisa mis mensajes nuevos.",
    scheduleText: "every day",
    scheduleKind: "recurring",
    cadence: { interval: 1, unit: "day" },
    scheduleSupported: true,
    timingDefaulted: true,
    cadenceBoundaryConflict: false,
    runOnceRequested: false,
    endBoundary: null,
  };
  const id = "draft-compose-review";
  const attachment = plannedRoutineWorkflowAttachment(request, id, "project-1");
  mocks.shell.routineDraft = {
    id,
    ...request,
    workflowAttachment: attachment,
  };
  const template = workflowTemplateById("unread-mail-check");
  if (!template) throw new Error("Unread Mail template is missing.");
  const workflowIr = instantiateWorkflowIrTemplate(
    template,
    attachment.workflowId,
  );
  let resolveComposition!: (value: unknown) => void;
  const composition = new Promise((resolve) => {
    resolveComposition = resolve;
  });
  mocks.invoke.mockImplementation(async (command: string) => {
    if (
      command === "list_routines" ||
      command === "get_workflows" ||
      command === "get_channel_statuses"
    ) {
      return [];
    }
    if (command === "list_projects") {
      return [{ projectId: "project-1", name: "Launch" }];
    }
    if (command === "get_background_service_status") return backgroundStatus();
    if (command === "propose_routine") {
      return {
        scheduleExpression: "every 1 day",
        scheduleKind: "recurring",
        timezone: "America/New_York",
        normalizedSummary: "Every day",
        nextRunsMs: [Date.now() + 86_400_000],
      };
    }
    if (command === "get_workflow_capability_catalog") {
      return {
        authoringEnabled: true,
        generatedAtMs: 1,
        actions: [],
        templates: [],
        version: "v1",
      };
    }
    if (command === "compose_workflow") return composition;
    throw new Error(`Unexpected command before confirmation: ${command}`);
  });
  return { composition, resolveComposition, workflowIr };
}

describe("RoutinesScreen Chat handoff", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.setRoutineDraft.mockReset();
    const request: RoutineHandoffRequest = {
      requestText: "Check my unread email every hour until midnight. Once you set it up, run it once.",
      scheduleText: "every 1 hour",
      scheduleKind: "recurring",
      cadence: { interval: 1, unit: "hour" },
      scheduleSupported: true,
      timingDefaulted: false,
      cadenceBoundaryConflict: false,
      runOnceRequested: true,
      endBoundary: "midnight",
      targetAction: { kind: "read_unread_mail" },
    };
    const id = "draft-mail-review";
    mocks.shell.routineDraft = {
      id,
      ...request,
      workflowAttachment: plannedRoutineWorkflowAttachment(request, id, "project-1"),
    };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines" || command === "get_workflows" || command === "get_channel_statuses") return [];
      if (command === "list_projects") return [{ projectId: "project-1", name: "Launch" }];
      if (command === "get_background_service_status") return backgroundStatus();
      if (command === "propose_routine") {
        return {
          scheduleExpression: "every 1 hour", scheduleKind: "recurring",
          timezone: "America/New_York", normalizedSummary: "Every hour",
          nextRunsMs: [Date.now() + 3_600_000],
        };
      }
      throw new Error(`Unexpected command before confirmation: ${command}`);
    });
  });

  afterEach(() => {
    mocks.shell.routineDraft = null;
    cleanup();
  });

  it("opens review with the exact project-bound unread-Mail workflow selected and runs nothing", async () => {
    render(<RoutinesScreen showIntroduction={false} />);

    const project = await screen.findByRole("combobox", { name: /^Project/ });
    const workflow = screen.getByRole("combobox", { name: /^Workflow/ });
    await waitFor(() => {
      expect(project).toHaveValue("project-1");
      expect(workflow).toHaveValue(
        "workflow-chat-schedule-draft-mail-review",
      );
    });
    expect(screen.getAllByText("Unread Mail Check")).toHaveLength(2);
    await waitFor(() => expect(screen.getByRole("button", { name: "Schedule it" })).toBeEnabled());

    const commands = mocks.invoke.mock.calls.map(([command]) => command);
    expect(commands).not.toContain("create_project");
    expect(commands).not.toContain("save_workflow");
    expect(commands).not.toContain("create_routine");
    expect(commands).not.toContain("mcp_execute_tool");
    expect(mocks.setRoutineDraft).toHaveBeenCalledWith(null);
  });

  it("automatically composes an unmatched request before confirmation", async () => {
    const { composition, resolveComposition, workflowIr } =
      prepareGenericComposition();

    render(<RoutinesScreen showIntroduction={false} />);

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "get_workflow_capability_catalog",
        undefined,
      ),
    );
    expect(
      screen.getByText("routines.handoff_prepare_title"),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", {
        name: "routines.handoff_prepare_retry",
      }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: "Schedule it" }),
    ).toBeDisabled();

    await act(async () => {
      resolveComposition({
        status: "composed",
        reason: "",
        workflowIr,
        missingCapabilities: [],
        attempts: 1,
        latencyMs: 1,
      });
      await composition;
    });

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Schedule it" }),
      ).toBeEnabled(),
    );
    const commands = mocks.invoke.mock.calls.map(([command]) => command);
    expect(commands).not.toContain("save_workflow");
    expect(commands).not.toContain("create_routine");
    expect(commands).not.toContain("mcp_execute_tool");
  });
});
