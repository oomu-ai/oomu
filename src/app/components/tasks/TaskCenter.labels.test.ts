import { describe, expect, it } from "vitest";
import { taskErrorLabel, taskOriginLabel } from "./TaskCenter";

const t = (key: string) => key;

describe("TaskCenter labels", () => {
  it("maps every production origin with a safe fallback", () => {
    expect([
      taskOriginLabel(t, "taskflow"),
      taskOriginLabel(t, "workflow"),
      taskOriginLabel(t, "agent"),
      taskOriginLabel(t, "chat_queue"),
      taskOriginLabel(t, "routine"),
    ]).toEqual([
      "tasks.origin_task",
      "tasks.origin_workflow",
      "tasks.origin_agent",
      "tasks.origin_chat",
      "tasks.origin_scheduled",
    ]);
    expect(taskOriginLabel(t, "future_origin")).toBe("tasks.origin_other");
  });

  it("never returns raw backend error prose or machine identifiers", () => {
    const lostCanary = "BACKEND CANARY: Owning runtime record is missing.";
    const unknownCanary = "backend_failure_code_77";

    expect(taskErrorLabel(t, lostCanary, "lost")).toBe(
      "tasks.error_record_missing",
    );
    expect(taskErrorLabel(t, unknownCanary, "reconciled")).toBe(
      "tasks.error_generic",
    );
    expect(taskErrorLabel(t, lostCanary, "lost")).not.toMatch(
      /BACKEND|Owning runtime/i,
    );
    expect(taskErrorLabel(t, unknownCanary, "reconciled")).not.toContain(
      unknownCanary,
    );
  });

  it("turns an official-source rejection into actionable product copy", () => {
    expect(
      taskErrorLabel(
        t,
        "The official page returned HTTP 403.",
        "reconciled",
      ),
    ).toBe("tasks.error_official_source");
  });
});
