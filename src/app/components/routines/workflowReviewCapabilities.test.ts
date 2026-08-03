import { describe, expect, it } from "vitest";
import { composeScheduleText } from "./ScheduleBuilder";
import { humanScheduleSummary } from "./routineLabels";
import { workflowReviewCapabilities } from "./workflowReviewCapabilities";
import {
  scenarioFiveWorkflowSteps,
  scenarioSixWorkflowSteps,
  simpleWorkflowSteps,
} from "./workflowReviewFixtures";

describe("Routine Workflow review", () => {
  it("composes tap-first schedule choices into accepted expressions", () => {
    expect(
      composeScheduleText({
        customText: "",
        date: "2026-08-01",
        frequency: "hourly",
        hourlyInterval: 2,
        time: "09:00",
        weekDays: [1, 2, 3, 4, 5],
      }),
    ).toBe("0 */2 * * *");
    expect(
      composeScheduleText({
        customText: "",
        date: "2026-08-01",
        frequency: "weekly",
        hourlyInterval: 1,
        time: "08:30",
        weekDays: [1, 3, 5],
      }),
    ).toBe("30 08 * * 1,3,5");
    expect(
      composeScheduleText({
        customText: "",
        date: "2026-08-01",
        frequency: "once",
        hourlyInterval: 1,
        time: "15:00",
        weekDays: [1],
      }),
    ).toBe("on 2026-08-01 at 15:00");
  });

  it("recognizes the Project reader and typed analysis without weakening review", () => {
    expect(workflowReviewCapabilities(simpleWorkflowSteps)).toEqual({
      status: "ready",
      calendarCreate: false,
      calendarRead: false,
      emailDraft: false,
      emailRead: false,
      emailSend: false,
      officialWeb: false,
      projectFileRead: false,
      projectFileWrite: false,
    });
    expect(workflowReviewCapabilities(scenarioFiveWorkflowSteps)).toEqual({
      status: "ready",
      calendarCreate: false,
      calendarRead: false,
      emailDraft: false,
      emailRead: false,
      emailSend: false,
      officialWeb: true,
      projectFileRead: true,
      projectFileWrite: true,
    });
    expect(workflowReviewCapabilities(scenarioSixWorkflowSteps)).toEqual({
      status: "ready",
      calendarCreate: true,
      calendarRead: false,
      emailDraft: false,
      emailRead: false,
      emailSend: true,
      officialWeb: true,
      projectFileRead: true,
      projectFileWrite: true,
    });
    expect(
      workflowReviewCapabilities(
        JSON.stringify({
          workflowIr: {
            nodes: [
              {
                kind: "mcp_tool",
                id: "milestones",
                serverName: "oomu_task_tools",
                toolName: "analyze_project_milestones",
              },
            ],
            edges: [],
          },
        }),
      ),
    ).toEqual({
      status: "ready",
      calendarCreate: false,
      calendarRead: false,
      emailDraft: false,
      emailRead: false,
      emailSend: false,
      officialWeb: false,
      projectFileRead: false,
      projectFileWrite: false,
    });
  });

  it("keeps bounded Apple Mail and Calendar reads schedulable", () => {
    expect(
      workflowReviewCapabilities(
        JSON.stringify({
          workflowIr: {
            nodes: [
              {
                kind: "mcp_tool",
                id: "mail",
                serverName: "macos_applescript",
                toolName: "read_system_emails",
                arguments: { max_messages: 5, unread_only: true },
              },
              {
                kind: "mcp_tool",
                id: "calendar",
                serverName: "macos_applescript",
                toolName: "read_system_calendar",
                arguments: { hours_ahead: 24 },
              },
            ],
            edges: [],
          },
        }),
      ),
    ).toEqual({
      status: "ready",
      calendarCreate: false,
      calendarRead: true,
      emailDraft: false,
      emailRead: true,
      emailSend: false,
      officialWeb: false,
      projectFileRead: false,
      projectFileWrite: false,
    });
  });

  it.each(["fail", "branch"])(
    "accepts an exactly approved Mail draft with onDenied=%s",
    (onDenied) => {
      const deniedEdge =
        onDenied === "branch"
          ? [
              {
                sourceNodeId: "approve-draft",
                sourcePort: "denied",
                targetNodeId: "declined",
              },
            ]
          : [];
      expect(
        workflowReviewCapabilities(
          JSON.stringify({
            workflowIr: {
              nodes: [
                {
                  kind: "permission",
                  id: "approve-draft",
                  permission: "mcp_tool",
                  onDenied,
                },
                {
                  kind: "mcp_tool",
                  id: "draft",
                  serverName: "macos_applescript",
                  toolName: "draft_system_email",
                  arguments: { subject: "Review", body: "Ready" },
                },
                { kind: "output", id: "done" },
                { kind: "output", id: "declined" },
              ],
              edges: [
                {
                  sourceNodeId: "approve-draft",
                  sourcePort: "approved",
                  targetNodeId: "draft",
                },
                ...deniedEdge,
                {
                  sourceNodeId: "draft",
                  sourcePort: "out",
                  targetNodeId: "done",
                },
              ],
            },
          }),
        ),
      ).toMatchObject({ status: "ready", emailDraft: true });
    },
  );

  it("rejects a Mail draft without its exact permission predecessor", () => {
    expect(
      workflowReviewCapabilities(
        JSON.stringify({
          workflowIr: {
            nodes: [
              {
                kind: "mcp_tool",
                id: "draft",
                serverName: "macos_applescript",
                toolName: "draft_system_email",
              },
            ],
            edges: [],
          },
        }),
      ),
    ).toMatchObject({ status: "unavailable" });
  });

  it("fails closed for malformed, unknown, or unguarded capabilities", () => {
    expect(workflowReviewCapabilities("{")).toMatchObject({ status: "unavailable" });
    expect(
      workflowReviewCapabilities(
        JSON.stringify({
          workflowIr: {
            nodes: [
              {
                kind: "mcp_tool",
                id: "unknown",
                serverName: "unknown_server",
                toolName: "unknown_tool",
              },
            ],
            edges: [],
          },
        }),
      ),
    ).toMatchObject({ status: "unavailable" });
    expect(
      workflowReviewCapabilities(
        JSON.stringify({
          workflowIr: {
            nodes: [
              {
                kind: "mcp_tool",
                id: "calendar",
                serverName: "oomu_task_tools",
                toolName: "create_conflict_free_calendar_event",
              },
            ],
            edges: [],
          },
        }),
      ),
    ).toMatchObject({ status: "unavailable" });
  });

  it("never exposes stored scheduler expressions", () => {
    const t = (key: string, values?: Record<string, string | number>) => {
      const template =
        key === "routines.schedule_daily_at"
          ? "Every day at {time}"
          : key === "routines.schedule_weekly_at"
            ? "{days} at {time}"
            : key === "routines.day_mon"
              ? "Mon"
            : key;
      return Object.entries(values ?? {}).reduce(
        (value, [name, replacement]) =>
          value.replaceAll(`{${name}}`, String(replacement)),
        template,
      );
    };
    const daily = humanScheduleSummary("0 9 * * *", "America/New_York", t);
    const custom = humanScheduleSummary("15 9 * * 1", "America/New_York", t);
    expect(daily).toMatch(/^Every day at /);
    expect(daily).not.toContain("0 9 * * *");
    expect(custom).toMatch(/^Mon at /);
    expect(custom).not.toContain("15 9 * * 1");
  });
});
