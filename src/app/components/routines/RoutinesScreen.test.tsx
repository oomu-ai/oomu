import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  backgroundStatusLabel,
  routineActionErrorKey,
  routinePausedReasonLabel,
  RoutinesScreen,
} from "./RoutinesScreen";
import type { BackgroundStatus, RoutineHistoryItem } from "./routineClient";
import {
  routineFixture as routine,
  scenarioFiveWorkflowSteps,
  scenarioSixWorkflowSteps,
  simpleWorkflowSteps,
} from "./workflowReviewFixtures";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  setActiveItem: vi.fn(),
}));

const i18n = vi.hoisted(() => {
  const translations: Record<string, string> = {
    "common.cancel": "Cancel",
    "common.delete": "Delete",
    "common.loading": "Loading…",
    "common.refresh": "Refresh",
    "common.refreshing": "Refreshing",
    "common.unknown": "Unknown",
    "routines.active": "Active",
    "routines.advanced": "Advanced",
    "routines.background": "Work in the background",
    "routines.background_active": "Background scheduling is ready.",
    "routines.background_approval": "Allow scheduled tasks in System Settings.",
    "routines.background_attention": "Background scheduling needs attention. Try turning it off and on again.",
    "routines.background_paused": "Background scheduling is turned off.",
    "routines.background_unavailable": "Background scheduling isn't available on this Mac.",
    "routines.background_unknown": "OOMU couldn't check background scheduling right now.",
    "routines.background_state_off": "Off",
    "routines.background_state_turning_on": "Turning on…",
    "routines.background_state_on": "On",
    "routines.background_state_needs_attention": "Needs attention",
    "routines.background_state_turning_off": "Turning off…",
    "routines.background_checking_help": "OOMU is checking background work.",
    "routines.background_off_help": "Scheduled tasks run only while OOMU is open.",
    "routines.background_turning_on_help": "OOMU is checking that background work can run.",
    "routines.background_on_help": "OOMU can keep scheduled tasks moving after you close the window.",
    "routines.background_needs_attention_help": "Background work stopped. Try again, or turn it off.",
    "routines.background_turning_off_help": "OOMU is stopping background work.",
    "routines.background_turn_on": "Turn on",
    "routines.background_turn_off": "Turn off",
    "routines.background_try_again": "Try again",
    "routines.background_repair_label": "Background work options",
    "routines.background_check_failed": "OOMU couldn't check background work. Try again.",
    "routines.background_action_failed": "OOMU couldn't change background work. Try again.",
    "routines.choose_workflow": "Choose a workflow",
    "routines.choose_workflow_hint": "Choose a workflow to continue.",
    "routines.choose_project": "Choose a Project",
    "routines.choose_project_hint": "Choose a Project under Advanced to continue.",
    "routines.confirm": "Schedule it",
    "routines.authority_title": "Review what can run automatically",
    "routines.authority_workflow": "Workflow",
    "routines.authority_project": "Project",
    "routines.authority_files": "Project files",
    "routines.authority_files_scope": "Read and create files only in {project}’s approved locations.",
    "routines.authority_files_read_scope": "Read files only from {project}’s approved locations.",
    "routines.authority_files_write_scope": "Create verified files only in {project}’s approved locations.",
    "routines.authority_web": "Official web sources",
    "routines.authority_web_scope": "Retrieve current information from official public websites.",
    "routines.authority_delivery": "Delivery",
    "routines.authority_delivery_local": "Keep results in OOMU.",
    "routines.authority_delivery_channel": "Deliver to {platform}: {destination}.",
    "routines.authority_delivery_pending": "Choose the delivery destination to continue.",
    "routines.authority_exact_approval": "Creating Calendar events and sending email will still pause for your exact approval.",
    "routines.authority_calendar_approval": "Creating Calendar events will still pause for your exact approval.",
    "routines.authority_email_approval": "Sending email will still pause for your exact approval.",
    "routines.authority_unavailable_title": "Review unavailable",
    "routines.authority_unavailable_help": "OOMU couldn’t verify this Workflow’s capabilities. Open and save the Workflow again before scheduling it.",
    "routines.authority_unavailable_hint": "Review the Workflow before scheduling.",
    "routines.create_title": "New schedule",
    "routines.custom_help": "Use any schedule OOMU understands.",
    "routines.custom_schedule": "Describe the timing",
    "routines.cadence_custom": "custom",
    "routines.cadence_daily": "daily",
    "routines.cadence_hourly": "hourly",
    "routines.cadence_once": "once",
    "routines.cadence_weekly": "weekly",
    "routines.change": "Change",
    "routines.connect_messaging": "Connect another messaging service…",
    "routines.date": "Date",
    "routines.day_fri": "Fri",
    "routines.day_mon": "Mon",
    "routines.day_sat": "Sat",
    "routines.day_sun": "Sun",
    "routines.day_thu": "Thu",
    "routines.day_tue": "Tue",
    "routines.day_wed": "Wed",
    "routines.days": "Days",
    "routines.default_schedule": "daily at 9 AM",
    "routines.discord_channel": "Discord channel",
    "routines.discord_channel_hint": "Choose the Discord channel to continue.",
    "routines.discord_channel_placeholder": "Channel ID, for example 1234567890",
    "routines.slack_conversation": "Slack conversation",
    "routines.slack_conversation_hint": "Choose the Slack conversation to continue.",
    "routines.slack_conversation_placeholder": "Channel or direct-message ID",
    "routines.delivery_discord": "Discord",
    "routines.delivery_platform": "Send results to",
    "routines.delivery_slack": "Slack",
    "routines.delivery_telegram": "Telegram",
    "routines.every": "Repeat",
    "routines.frequency": "How often",
    "routines.frequency_custom": "Custom…",
    "routines.frequency_daily": "Daily",
    "routines.frequency_hourly": "Hourly",
    "routines.frequency_once": "Once",
    "routines.frequency_weekly": "Weekly",
    "routines.generated_name": "{workflow} — {cadence}",
    "routines.hour_interval_many": "Every {count} hours",
    "routines.hour_interval_one": "Every hour",
    "routines.disable": "Disable",
    "routines.duplicate": "Duplicate",
    "routines.duplicating": "Duplicating…",
    "routines.empty": "No scheduled tasks yet.",
    "routines.enable": "Enable",
    "routines.updating": "Updating…",
    "routines.history": "History",
    "routines.history_state_unknown": "Status unavailable",
    "routines.local_only": "Just notify me here",
    "routines.missed_each": "Run each one I missed",
    "routines.missed_once": "Run it once when I'm back",
    "routines.missed_policy": "If your computer was off at that time",
    "routines.missed_skip": "Skip it",
    "routines.name_optional": "Name (optional)",
    "routines.new": "Schedule a task",
    "routines.no_history": "Nothing here yet.",
    "routines.open_result": "Open the result",
    "routines.pause": "Pause",
    "routines.pausing": "Pausing…",
    "routines.paused": "Paused",
    "routines.pause_reason_other": "This scheduled task is paused. Review its setup before resuming.",
    "routines.preparing": "Finding the next few times…",
    "routines.preview_waiting": "Your next few times will appear here.",
    "routines.project": "Project",
    "routines.resume": "Resume",
    "routines.resuming": "Resuming…",
    "routines.run_now": "Run now",
    "routines.running": "Starting…",
    "routines.creating": "Creating…",
    "routines.delete": "Delete Schedule…",
    "routines.delete_title": "Delete “{name}” schedule?",
    "routines.delete_warning": "This permanently deletes this schedule. It won’t run again. This can’t be undone.",
    "routines.delete_preserved": "Past tasks and their results stay in Tasks. The workflow is not deleted.",
    "routines.delete_confirm": "Delete Schedule",
    "routines.deleting": "Deleting…",
    "routines.deleted": "Schedule deleted.",
    "routines.delete_failed": "OOMU couldn’t delete this schedule. Try again.",
    "routines.delete_running": "This schedule has unfinished tasks. Finish or stop them, then delete the schedule.",
    "routines.error_load": "Scheduled tasks couldn't be loaded. Try checking again.",
    "routines.schedule_custom": "Custom schedule",
    "routines.schedule_hourly": "Every hour",
    "routines.schedule_hint": "Choose a time to continue.",
    "routines.subtitle": "Recurring tasks, handled for you.",
    "routines.time": "Time",
    "routines.times_shown": "Times shown in {zone}.",
    "routines.timezone": "Time zone",
    "routines.title": "Scheduled",
    "routines.upcoming": "Next few times",
    "routines.weekdays": "Weekdays",
    "routines.when": "When",
    "routines.workflow": "Workflow",
    "tasks.state_blocked": "Blocked",
    "tasks.state_cancelled": "Cancelled",
    "tasks.state_completed": "Completed",
    "tasks.state_failed": "Failed",
    "tasks.state_planning": "Planning",
    "tasks.state_queued": "Queued",
    "tasks.state_running": "Running",
    "tasks.state_awaiting_approval": "Awaiting approval",
  };

  return {
    t: (key: string, values?: Record<string, string | number>) => {
      const templates: Record<string, string> = {
        "routines.schedule_daily_at": "Every day at {time}",
        "routines.schedule_every_hours": "Every {count} hours",
        "routines.schedule_every_minutes": "Every {count} minutes",
        "routines.schedule_once": "Once on {date}",
        "routines.schedule_weekly_at": "{days} at {time}",
        "routines.schedule_weekdays_at": "Weekdays at {time}",
      };
      let value = translations[key] ?? templates[key] ?? key;
      Object.entries(values ?? {}).forEach(([name, replacement]) => {
        value = value.replaceAll(`{${name}}`, String(replacement));
      });
      return value;
    },
  };
});

vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@/components/AppShell", () => ({
  useAppShell: () => ({ setActiveItem: mocks.setActiveItem }),
}));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({ t: i18n.t }),
}));

function backgroundStatus(
  state: BackgroundStatus["state"] = "on_verified",
): BackgroundStatus {
  return {
    userEnabled: state !== "off",
    verifiedActive: state === "on_verified",
    state,
    registrationState: state === "off" ? "unregistered" : "registered",
    registrationBackend: "supervised_process",
    processState: state === "on_verified" ? "running" : "absent",
    registrationGeneration: state === "off" ? null : "registration-1",
    processId: state === "on_verified" ? 42 : null,
    buildNumber: 7,
    buildIdentity: "build-7",
    profileClass: "development",
    profileGenerationSha256: "profile-digest-1",
    heartbeatAtMs: state === "on_verified" ? 1 : null,
    heartbeatAgeMs: state === "on_verified" ? 0 : null,
    menuVisible: state === "on_verified",
    errorCode: null,
    detail: "Ready",
    checkedAtMs: 1,
    recentReceipts: [],
  };
}

let backgroundResponse: BackgroundStatus = backgroundStatus();

describe("RoutinesScreen", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.setActiveItem.mockReset();
    window.sessionStorage.clear();
    backgroundResponse = backgroundStatus();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") {
        return [{ projectId: "project-1", name: "Launch" }];
      }
      if (command === "get_workflows") {
        return [{ id: "workflow-1", name: "Daily brief", projectId: "project-1", workflowVersion: 1, steps: scenarioFiveWorkflowSteps }];
      }
      if (command === "get_background_service_status") {
        return backgroundResponse;
      }
      if (command === "get_channel_statuses") {
        return [
          {
            platform: "slack",
            label: "Slack",
            isActive: true,
            connectionState: "linked",
            ownerId: "U123OWNER",
            workerState: "running",
            lastCheckedAtMs: 1,
            detail: null,
          },
          {
            platform: "telegram",
            label: "Telegram",
            isActive: false,
            connectionState: "unlinked",
            ownerId: null,
            workerState: "idle",
            lastCheckedAtMs: 1,
            detail: null,
          },
        ];
      }
      if (command === "get_routine_history") {
        return [
          {
            taskRunId: "task-run-1",
            state: "completed",
            summary: "Morning brief",
            lastError: null,
            createdAtMs: Date.UTC(2026, 6, 11, 13),
            updatedAtMs: Date.UTC(2026, 6, 11, 13, 1),
          },
        ];
      }
      if (command === "propose_routine") {
        return {
          scheduleExpression: "0 9 * * *",
          scheduleKind: "recurring",
          timezone: "America/New_York",
          normalizedSummary: "Cron 0 9 * * * in America/New_York",
          nextRunsMs: [Date.UTC(2026, 6, 13, 13)],
        };
      }
      return null;
    });
  });

  afterEach(cleanup);

  it("shows honest initial loading and keeps setup actions locked until data is ready", async () => {
    let finishRoutineLoad!: (value: (typeof routine)[]) => void;
    const pendingRoutines = new Promise<(typeof routine)[]>((resolve) => {
      finishRoutineLoad = resolve;
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return pendingRoutines;
      if (command === "list_projects") return [{ projectId: "project-1", name: "Launch" }];
      if (command === "get_workflows") return [{ id: "workflow-1", name: "Daily brief", projectId: "project-1", workflowVersion: 1, steps: scenarioFiveWorkflowSteps }];
      if (command === "get_background_service_status") return backgroundResponse;
      return null;
    });

    render(<RoutinesScreen />);

    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Turn on" })).toBeDisabled();
    expect(screen.getAllByText("Loading…")).not.toHaveLength(0);

    await act(async () => finishRoutineLoad([routine]));
    await screen.findByRole("heading", { name: "Morning brief" });
    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Turn off" })).toBeEnabled();
  });

  it("never exposes backend background prose or repair identifiers", async () => {
    const detailCanary =
      "BACKEND CANARY. Repair code: background_registration_rejected.";
    backgroundResponse = {
      ...backgroundStatus("needs_attention"),
      detail: detailCanary,
      errorCode: "background_registration_rejected",
    };

    render(<RoutinesScreen />);

    expect(
      await screen.findByText(
        "Background work stopped. Try again, or turn it off.",
      ),
    ).toBeVisible();
    expect(screen.queryByText(detailCanary)).toBeNull();
    expect(screen.queryByText(/Repair code|background_registration_rejected/i)).toBeNull();
    expect(backgroundStatusLabel(i18n.t, "future_backend_state")).toBe(
      "OOMU couldn't check background scheduling right now.",
    );
  });

  it("replaces paused and load failures with calm localized guidance", async () => {
    const pausedCanary = "BACKEND CANARY: Paused after scheduler_failure_code_17";
    expect(routinePausedReasonLabel(i18n.t, pausedCanary)).toBe(
      "This scheduled task is paused. Review its setup before resuming.",
    );
    expect(routinePausedReasonLabel(i18n.t, pausedCanary)).not.toContain(pausedCanary);

    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") throw new Error(pausedCanary);
      if (command === "list_projects" || command === "get_workflows") return [];
      if (command === "get_background_service_status") return backgroundResponse;
      return null;
    });
    render(<RoutinesScreen />);

    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent("Scheduled tasks couldn't be loaded. Try checking again.");
    expect(screen.queryByText(/BACKEND CANARY|scheduler_failure_code_17/i)).toBeNull();
  });

  it("renders human history and opens its exact Task", async () => {
    render(<RoutinesScreen />);

    expect(
      await screen.findByRole("heading", { name: "Morning brief" }),
    ).toBeVisible();
    expect(screen.getByText(/Every day at .*New York/)).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    expect(await screen.findByText("Completed")).toBeVisible();
    expect(screen.queryByText(/taskRunId/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open the result" }));

    expect(JSON.parse(window.sessionStorage.getItem("oomu.tasks.focus") ?? "null")).toEqual({
      state: "completed",
      taskRunId: "task-run-1",
    });
    expect(mocks.setActiveItem).toHaveBeenCalledWith("tasks");
  });

  it("shows the complete tap-first form and fills a connected recipient", async () => {
    const view = render(<RoutinesScreen showIntroduction={false} />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled(),
    );
    expect(screen.queryByRole("heading", { name: "Scheduled" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Schedule a task" }));
    expect(screen.getByRole("heading", { name: "New schedule" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Daily" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByLabelText("Workflow")).toBeVisible();
    expect(screen.getByRole("button", { name: "Schedule it" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /Preview next/i }),
    ).toBeNull();
    expect(screen.queryByText("Authorized destination")).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Time zone" })).toBeNull();
    expect(
      screen.getByRole("combobox", { name: "Time zone" }),
    ).not.toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    expect(
      screen.getByRole("combobox", { name: "Time zone" }),
    ).toBeVisible();
    expect(
      screen.getByRole("combobox", {
        name: "If your computer was off at that time",
      }),
    ).toBeVisible();

    const channel = screen.getByRole("combobox", {
      name: "Send results to",
    });
    expect(channel).toContainElement(
      screen.getByRole("option", { name: "Slack" }),
    );
    expect(screen.queryByRole("option", { name: "Telegram" })).toBeNull();
    expect(screen.queryByRole("option", { name: "Discord" })).toBeNull();
    fireEvent.change(screen.getByLabelText("Workflow"), {
      target: { value: "workflow-1" },
    });
    fireEvent.change(channel, { target: { value: "slack" } });
    fireEvent.change(screen.getByLabelText("Slack conversation"), {
      target: { value: "C123ALLOWED" },
    });
    const authorityReview = screen.getByRole("region", {
      name: "Review what can run automatically",
    });
    expect(within(authorityReview).getByText("Daily brief")).toBeVisible();
    expect(within(authorityReview).getByText("Launch")).toBeVisible();
    expect(
      within(authorityReview).getByText(
        "Read and create files only in Launch’s approved locations.",
      ),
    ).toBeVisible();
    expect(
      within(authorityReview).getByText(
        "Retrieve current information from official public websites.",
      ),
    ).toBeVisible();
    expect(
      within(authorityReview).getByText(
        "Deliver to Slack: C123ALLOWED.",
      ),
    ).toBeVisible();
    expect(
      within(authorityReview).queryByText(
        "Creating Calendar events and sending email will still pause for your exact approval.",
      ),
    ).toBeNull();

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Schedule it" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Schedule it" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "create_routine",
        expect.objectContaining({
          request: expect.objectContaining({
            authority: { mode: "reviewed_workflow_scope" },
            deliveryTarget: {
              destination: "C123ALLOWED",
              platform: "slack",
            },
          }),
        }),
      ),
    );

    const createRequest = mocks.invoke.mock.calls.find(
      ([command]) => command === "create_routine",
    )?.[1]?.request;
    expect(createRequest.authority).toEqual({
      mode: "reviewed_workflow_scope",
    });
    expect(createRequest.authority).not.toHaveProperty("actions");

    view.unmount();
  });

  it("keeps a simple Workflow review simple", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") {
        return [{ projectId: "project-1", name: "Launch" }];
      }
      if (command === "get_workflows") {
        return [{
          id: "workflow-1",
          name: "Daily brief",
          projectId: "project-1",
          workflowVersion: 1,
          steps: simpleWorkflowSteps,
        }];
      }
      if (command === "get_background_service_status") return backgroundResponse;
      if (command === "get_channel_statuses") return [];
      if (command === "propose_routine") {
        return {
          scheduleExpression: "0 9 * * *",
          scheduleKind: "recurring",
          timezone: "America/New_York",
          normalizedSummary: "Daily",
          nextRunsMs: [Date.UTC(2026, 6, 13, 13)],
        };
      }
      return null;
    });

    render(<RoutinesScreen showIntroduction={false} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Schedule a task" }));
    fireEvent.change(screen.getByLabelText("Workflow"), {
      target: { value: "workflow-1" },
    });

    const review = screen.getByRole("region", {
      name: "Review what can run automatically",
    });
    expect(within(review).getByText("Daily brief")).toBeVisible();
    expect(within(review).getByText("Launch")).toBeVisible();
    expect(within(review).getByText("Keep results in OOMU.")).toBeVisible();
    expect(within(review).queryByText("Project files")).toBeNull();
    expect(within(review).queryByText("Official web sources")).toBeNull();
    expect(within(review).queryByText(/exact approval/)).toBeNull();
  });

  it("explains the exact approvals still required by a Calendar and email Workflow", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") {
        return [{ projectId: "project-1", name: "Launch" }];
      }
      if (command === "get_workflows") {
        return [{
          id: "workflow-1",
          name: "Daily brief",
          projectId: "project-1",
          workflowVersion: 1,
          steps: scenarioSixWorkflowSteps,
        }];
      }
      if (command === "get_background_service_status") return backgroundResponse;
      if (command === "get_channel_statuses") return [];
      return null;
    });

    render(<RoutinesScreen showIntroduction={false} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Schedule a task" }));
    fireEvent.change(screen.getByLabelText("Workflow"), {
      target: { value: "workflow-1" },
    });

    expect(
      within(screen.getByRole("region", {
        name: "Review what can run automatically",
      })).getByText(
        "Creating Calendar events and sending email will still pause for your exact approval.",
      ),
    ).toBeVisible();
  });

  it("fails closed when a saved Workflow cannot be reviewed", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") {
        return [{ projectId: "project-1", name: "Launch" }];
      }
      if (command === "get_workflows") {
        return [{
          id: "workflow-1",
          name: "Broken Workflow",
          projectId: "project-1",
          workflowVersion: 1,
          steps: "{",
        }];
      }
      if (command === "get_background_service_status") return backgroundResponse;
      if (command === "get_channel_statuses") return [];
      if (command === "propose_routine") {
        return {
          scheduleExpression: "0 9 * * *",
          scheduleKind: "recurring",
          timezone: "America/New_York",
          normalizedSummary: "Daily",
          nextRunsMs: [Date.UTC(2026, 6, 13, 13)],
        };
      }
      return null;
    });

    render(<RoutinesScreen showIntroduction={false} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Schedule a task" }));
    fireEvent.change(screen.getByLabelText("Workflow"), {
      target: { value: "workflow-1" },
    });

    expect(screen.getByRole("region", { name: "Review unavailable" })).toHaveTextContent(
      "OOMU couldn’t verify this Workflow’s capabilities.",
    );
    expect(screen.getByRole("button", { name: "Schedule it" })).toBeDisabled();
    expect(screen.getByText("Review the Workflow before scheduling.")).toBeVisible();
  });

  it("shows immediate progress and locks routine actions while one is running", async () => {
    let finishRun!: () => void;
    const pendingRun = new Promise<void>((resolve) => { finishRun = resolve; });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") return [{ projectId: "project-1", name: "Launch" }];
      if (command === "get_workflows") return [{ id: "workflow-1", name: "Daily brief", projectId: "project-1", workflowVersion: 1, steps: scenarioFiveWorkflowSteps }];
      if (command === "get_background_service_status") return backgroundResponse;
      if (command === "run_routine_now") { await pendingRun; return routine; }
      return null;
    });
    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));

    expect(screen.getByRole("button", { name: "Starting…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Duplicate" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete Schedule…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Turn off" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Morning brief/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDisabled();

    await act(async () => finishRun());
    await waitFor(() => expect(screen.getByRole("button", { name: "Run now" })).toBeEnabled());
  });

  it("keeps a history refresh attached to the selected schedule", async () => {
    let finishHistory!: (value: RoutineHistoryItem[]) => void;
    const pendingHistory = new Promise<RoutineHistoryItem[]>((resolve) => {
      finishHistory = resolve;
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") return [{ projectId: "project-1", name: "Launch" }];
      if (command === "get_workflows") return [{ id: "workflow-1", name: "Daily brief", projectId: "project-1", workflowVersion: 1, steps: scenarioFiveWorkflowSteps }];
      if (command === "get_background_service_status") return backgroundResponse;
      if (command === "get_routine_history") return pendingHistory;
      return null;
    });
    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    expect(screen.getByRole("button", { name: "Refreshing" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Turn off" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Morning brief/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Run now" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete Schedule…" })).toBeDisabled();

    await act(async () => finishHistory([]));
    await waitFor(() => expect(screen.getByRole("button", { name: "Refresh" })).toBeEnabled());
  });
});

describe("RoutinesScreen background status isolation", () => {
  afterEach(cleanup);

  it("keeps schedule controls available while a failed background check is retried", async () => {
    let backgroundChecks = 0;
    let finishRetry!: (value: BackgroundStatus) => void;
    const pendingRetry = new Promise<BackgroundStatus>((resolve) => {
      finishRetry = resolve;
    });
    mocks.invoke.mockReset();
    mocks.setActiveItem.mockReset();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_routines") return [routine];
      if (command === "list_projects") return [{ projectId: "project-1", name: "Launch" }];
      if (command === "get_workflows") return [];
      if (command === "get_channel_statuses") return [];
      if (command === "get_background_service_status") {
        backgroundChecks += 1;
        if (backgroundChecks === 1) throw new Error("PRIVATE BACKGROUND FAILURE");
        return pendingRetry;
      }
      return null;
    });

    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });
    expect(await screen.findByText("OOMU couldn't check background work. Try again."))
      .toBeVisible();
    expect(screen.queryByText("PRIVATE BACKGROUND FAILURE")).toBeNull();
    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(screen.getByRole("button", { name: "Schedule a task" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Updating…" })).toBeDisabled();

    await act(async () => finishRetry(backgroundStatus()));
    expect(await screen.findByText("On")).toBeVisible();
    expect(screen.getByRole("button", { name: "Turn off" })).toBeEnabled();
  });
});

describe("routineActionErrorKey", () => {
  it.each([
    ["routine_workflow_project_binding_required", "routines.error_workflow_project_required"],
    ["routine_workflow_project_mismatch", "routines.error_workflow_project_mismatch"],
    ["routine_workflow_version_unavailable", "routines.error_workflow_version_unavailable"],
  ])("maps %s to an actionable localized message", (code, key) => {
    expect(routineActionErrorKey(new Error(code))).toBe(key);
  });
});
