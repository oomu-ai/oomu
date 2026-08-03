import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoutinesScreen } from "./RoutinesScreen";
import { routineFixture as routine } from "./workflowReviewFixtures";

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
    "routines.active": "Active",
    "routines.background": "Work in the background",
    "routines.background_active": "Background scheduling is ready.",
    "routines.delete": "Delete Schedule…",
    "routines.delete_confirm": "Delete Schedule",
    "routines.delete_failed": "OOMU couldn’t delete this schedule. Try again.",
    "routines.delete_preserved": "Past tasks and their results stay in Tasks. The workflow is not deleted.",
    "routines.delete_running": "This schedule has unfinished tasks. Finish or stop them, then delete the schedule.",
    "routines.delete_title": "Delete “{name}” schedule?",
    "routines.delete_warning": "This permanently deletes this schedule. It won’t run again. This can’t be undone.",
    "routines.deleted": "Schedule deleted.",
    "routines.deleting": "Deleting…",
    "routines.delivery_retrying_list": "Delivering",
    "routines.delivery_review_list": "Check delivery",
    "routines.disable": "Disable",
    "routines.duplicate": "Duplicate",
    "routines.empty": "No scheduled tasks yet.",
    "routines.enable": "Enable",
    "routines.history": "History",
    "routines.identity_details": "Schedule details",
    "routines.new": "Schedule a task",
    "routines.no_history": "Nothing here yet.",
    "routines.pause": "Pause",
    "routines.paused": "Paused",
    "routines.routine_id": "Routine ID",
    "routines.run_now": "Run now",
    "routines.schedule_custom": "Custom schedule",
    "routines.schedule_daily_at": "Every day at {time}",
    "routines.subtitle": "Recurring tasks, handled for you.",
    "routines.title": "Scheduled",
    "routines.upcoming": "Next few times",
    "routines.workflow_version": "Workflow version",
  };
  return {
    t: (key: string, values?: Record<string, string | number>) => {
      let value = translations[key] ?? key;
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

const background = {
  userEnabled: true,
  state: "active",
  detail: "Ready",
  checkedAtMs: 1,
};

function readOnlyCommand(command: string) {
  if (command === "list_routines") return [routine];
  if (command === "list_projects") {
    return [{ projectId: "project-1", name: "Launch" }];
  }
  if (command === "get_workflows") return [];
  if (command === "get_background_service_status") return background;
  if (command === "get_channel_statuses") return [];
  return null;
}

describe("RoutinesScreen deletion", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.setActiveItem.mockReset();
    mocks.invoke.mockImplementation(async (command: string) =>
      readOnlyCommand(command),
    );
  });

  afterEach(cleanup);

  it("makes schedule deletion explicit and cancellation changes nothing", async () => {
    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });
    const trigger = screen.getByRole("button", { name: "Delete Schedule…" });
    fireEvent.click(trigger);

    const dialog = screen.getByRole("dialog", {
      name: "Delete “Morning brief” schedule?",
    });
    expect(
      within(dialog).getByText(
        "This permanently deletes this schedule. It won’t run again. This can’t be undone.",
      ),
    ).toBeVisible();
    expect(
      within(dialog).getByText(
        "Past tasks and their results stay in Tasks. The workflow is not deleted.",
      ),
    ).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(screen.queryByRole("button", { name: "Disable" })).toBeNull();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "delete_routine"),
    ).toHaveLength(0);

    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "delete_routine"),
    ).toHaveLength(0);
  });

  it("shows deletion progress, confirms once, and selects the next schedule", async () => {
    const second = { ...routine, routineId: "routine-2", label: "Weekly review" };
    let rows = [routine, second];
    let finishDelete!: () => void;
    const pendingDelete = new Promise<void>((resolve) => {
      finishDelete = resolve;
    });
    mocks.invoke.mockImplementation(
      async (command: string, args?: Record<string, unknown>) => {
        if (command === "list_routines") return rows;
        if (command === "delete_routine") {
          await pendingDelete;
          const request = args?.request as { routineId?: string } | undefined;
          rows = rows.filter((item) => item.routineId !== request?.routineId);
          return null;
        }
        return readOnlyCommand(command);
      },
    );
    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });
    fireEvent.click(screen.getByRole("button", { name: "Delete Schedule…" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete Schedule" }));

    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Deleting…" })).toBeDisabled();
    await waitFor(() => expect(dialog).toHaveFocus());
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(dialog).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeVisible();

    await act(async () => finishDelete());
    await screen.findByRole("heading", { name: "Weekly review" });
    expect(screen.getByText("Schedule deleted.")).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "delete_routine"),
    ).toEqual([
      ["delete_routine", { request: { routineId: "routine-1", confirmed: true } }],
    ]);
  });

  it("keeps a running schedule and explains when deletion can be retried", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "delete_routine") throw new Error("routine_delete_running");
      return readOnlyCommand(command);
    });
    render(<RoutinesScreen />);
    await screen.findByRole("heading", { name: "Morning brief" });
    fireEvent.click(screen.getByRole("button", { name: "Delete Schedule…" }));
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Delete Schedule",
      }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This schedule has unfinished tasks. Finish or stop them, then delete the schedule.",
    );
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(screen.queryByText("routine_delete_running")).toBeNull();
  });
});
