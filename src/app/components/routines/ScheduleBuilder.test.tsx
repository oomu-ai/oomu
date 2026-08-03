import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoutineCreateForm, ScheduleBuilder } from "./ScheduleBuilder";

const noop = vi.fn();

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function routineForm(overrides: Record<string, unknown> = {}) {
  return (
    <RoutineCreateForm
      connectedChannels={[]}
      currentProposal={{
        nextRunsMs: [Date.UTC(2026, 6, 22, 13)],
        normalizedSummary: "Daily",
        scheduleExpression: "0 9 * * *",
        scheduleKind: "recurring",
        timezone: "America/New_York",
      }}
      deliveryDestination=""
      deliveryPlatform=""
      disabled={false}
      isCreating={false}
      label=""
      missedPolicy="skip"
      onCreate={noop}
      onDeliveryChange={noop}
      onDeliveryDestinationChange={noop}
      onLabelChange={noop}
      onMissedPolicyChange={noop}
      onOpenConnections={noop}
      onOpenProjectWorkflows={noop}
      onProjectChange={noop}
      onScheduleChange={noop}
      onTimezoneChange={noop}
      onWorkflowChange={noop}
      projectId="project-alpha"
      projects={[
        { projectId: "project-alpha", name: "Alpha" },
        { projectId: "project-beta", name: "Beta" },
      ] as never[]}
      proposalBusy={false}
      scheduleError=""
      t={(key) => key}
      timezone="America/New_York"
      timezoneOptions={["America/New_York"]}
      workflowId=""
      workflows={[
        { id: "alpha-workflow", name: "Alpha brief", projectId: "project-alpha" },
        { id: "beta-workflow", name: "Beta brief", projectId: "project-beta" },
        { id: "global-workflow", name: "Global brief", projectId: null },
      ]}
      {...overrides}
    />
  );
}

describe("ScheduleBuilder one-time UX", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-21T02:02:30.000Z"));
  });

  it("starts Once today in the future and clearly blocks a past time", () => {
    const onScheduleChange = vi.fn();
    render(
      <ScheduleBuilder
        disabled={false}
        onScheduleChange={onScheduleChange}
        t={(key) => key}
        timezone="America/Los_Angeles"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "routines.frequency_once" }));
    const date = screen.getByLabelText("routines.date") as HTMLInputElement;
    const time = screen.getByLabelText("routines.time") as HTMLInputElement;
    expect(date.min).toBe("2026-07-20");
    expect(date.value).toBe("2026-07-20");
    expect(time.value).toBe("19:12");

    fireEvent.change(time, { target: { value: "18:55" } });
    expect(screen.getByText("routines.once_past")).toBeVisible();
    expect(time).toHaveAttribute("aria-invalid", "true");
    expect(onScheduleChange).toHaveBeenLastCalledWith(
      "",
      "routines.cadence_once",
    );
  });
});

describe("ScheduleBuilder recurrence handoff", () => {
  it.each([
    ["every 5 minutes", 5, "minute", "every 5 minutes"],
    ["every 1 hour", 1, "hour", "every 1 hour"],
    ["every day", 1, "day", "every 1 day"],
    ["every 2 weeks", 2, "week", "every 2 weeks"],
    ["every month", 1, "month", "every 1 month"],
    ["every 2 quarters", 2, "quarter", "every 2 quarters"],
    ["every 11 years", 11, "year", "every 11 years"],
  ] as const)(
    "opens %s as one valid, visibly editable interval proposal",
    (initialScheduleText, count, unit, expectedSchedule) => {
      const onScheduleChange = vi.fn();
      render(
        <ScheduleBuilder
          disabled={false}
          initialScheduleText={initialScheduleText}
          onScheduleChange={onScheduleChange}
          t={(key) => key}
          timezone="America/New_York"
        />,
      );

      expect(
        screen.getByRole("button", { name: "routines.frequency_interval" }),
      ).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByLabelText("routines.interval_count")).toHaveValue(count);
      expect(screen.getByLabelText("routines.interval_unit")).toHaveValue(unit);
      expect(onScheduleChange).toHaveBeenLastCalledWith(
        expectedSchedule,
        "routines.cadence_interval",
      );
    },
  );

  it("keeps an unsupported sub-minute seed visible for correction", () => {
    const onScheduleChange = vi.fn();
    render(
      <ScheduleBuilder
        disabled={false}
        initialScheduleText="every 30 seconds"
        onScheduleChange={onScheduleChange}
        t={(key) => key}
        timezone="America/New_York"
      />,
    );

    expect(screen.getByRole("button", { name: "routines.frequency_custom" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("textbox")).toHaveValue("every 30 seconds");
    expect(onScheduleChange).toHaveBeenLastCalledWith(
      "every 30 seconds",
      "routines.cadence_custom",
    );
  });

  it.each([
    ["every weekday", "weekly", "09:00", "00 09 * * 1,2,3,4,5"],
    ["every weekend", "weekly", "09:00", "00 09 * * 0,6"],
    ["every monday", "weekly", "09:00", "00 09 * * 1"],
    ["every morning", "daily", "09:00", "daily at 09:00"],
    ["every afternoon", "daily", "13:00", "daily at 13:00"],
    ["every evening", "daily", "18:00", "daily at 18:00"],
    ["every night", "daily", "21:00", "daily at 21:00"],
  ] as const)(
    "opens %s with a visible deterministic timing default",
    (schedule, frequency, time, expectedSchedule) => {
      const onScheduleChange = vi.fn();
      render(
        <ScheduleBuilder
          disabled={false}
          initialScheduleText={schedule}
          onScheduleChange={onScheduleChange}
          t={(key) => key}
          timezone="America/New_York"
        />,
      );

      expect(
        screen.getByRole("button", { name: `routines.frequency_${frequency}` }),
      ).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByLabelText("routines.time")).toHaveValue(time);
      expect(onScheduleChange).toHaveBeenLastCalledWith(
        expectedSchedule,
        `routines.cadence_${frequency}`,
      );
    },
  );
});

describe("Routine Project scope", () => {
  it("shows human place names instead of raw timezone identifiers", () => {
    render(
      routineForm({
        timezoneOptions: ["America/New_York", "America/Los_Angeles"],
      }),
    );

    fireEvent.click(screen.getByText("routines.advanced"));
    expect(screen.getByRole("option", { name: "New York" })).toHaveValue(
      "America/New_York",
    );
    expect(screen.getByRole("option", { name: "Los Angeles" })).toHaveValue(
      "America/Los_Angeles",
    );
  });

  it("offers only Workflows bound to the selected Project", () => {
    render(routineForm());

    const workflow = screen.getByRole("combobox", {
      name: "routines.workflow",
    });
    expect(screen.getByRole("option", { name: "Alpha brief" })).toBeVisible();
    expect(screen.queryByRole("option", { name: "Beta brief" })).toBeNull();
    expect(screen.queryByRole("option", { name: "Global brief" })).toBeNull();
    expect(workflow).toHaveValue("");
  });

  it("gives an immediate path when a Project has no schedulable Workflow", () => {
    const openProjectWorkflows = vi.fn();
    render(
      routineForm({
        onOpenProjectWorkflows: openProjectWorkflows,
        workflows: [{ id: "global-workflow", name: "Global", projectId: null }],
      }),
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "routines.create_project_workflow",
      }),
    );
    expect(openProjectWorkflows).toHaveBeenCalledWith("composer");
  });

  it("honors an authoritative unavailable review without reinterpreting stored IR", () => {
    const view = render(
      routineForm({
        workflowId: "alpha-workflow",
        workflows: [
          {
            id: "alpha-workflow",
            name: "Alpha brief",
            projectId: "project-alpha",
            steps: JSON.stringify({
              workflowIr: {
                nodes: [{ kind: "input", id: "input" }],
                edges: [],
              },
            }),
            reviewCapabilities: {
              status: "unavailable",
              calendarCreate: false,
              calendarRead: false,
              emailDraft: false,
              emailRead: false,
              emailSend: false,
              officialWeb: false,
              projectFileRead: false,
              projectFileWrite: false,
            },
          },
        ],
      }),
    );

    expect(
      view.getByText("routines.authority_unavailable_title"),
    ).toBeVisible();
    expect(
      view.getByRole("button", { name: "routines.confirm" }),
    ).toBeDisabled();
  });
});
