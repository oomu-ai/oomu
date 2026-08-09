import { describe, expect, it, vi } from "vitest";
import {
  completeOneTimeRoutineHandoff,
  oneTimeRoutineRequestFromDecision,
  routineRequestFromDecision,
  shouldDeferFileShortcutForRoutine,
} from "./chatRoutineHandoff";

const prompt = "At 4:35 PM today, check whether /Users/example/report.md still exists and tell me in this task. Do not change the file.";

describe("oneTimeRoutineRequest", () => {
  it("preserves the exact request for a verified one-time Routine handoff", () => {
    expect(oneTimeRoutineRequestFromDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: ["future one-time routine"],
    }, prompt)).toBe(prompt);
  });
});

describe("recurring Routine request decoding", () => {
  it("separates an hourly schedule seed from the exact action and pending run-now request", () => {
    const recurringPrompt =
      "Set up an hourly task to check my email and report back on any unread emails. If there are no unread emails, let me know too. Once you create it and schedule it, I want you to test run it once.";

    expect(routineRequestFromDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: [
        "recurring routine",
        "routine cadence:v1:1:hour",
        "routine schedule seed: every 1 hour",
        "routine target private app:v1:mail",
        "explicit run once requested",
      ],
    }, recurringPrompt)).toEqual({
      requestText: recurringPrompt,
      scheduleText: "every 1 hour",
      scheduleKind: "recurring",
      cadence: { interval: 1, unit: "hour" },
      scheduleSupported: true,
      timingDefaulted: false,
      cadenceBoundaryConflict: false,
      runOnceRequested: true,
      endBoundary: null,
      targetAction: { kind: "read_unread_mail" },
    });
  });

  it("derives a valid one-time seed instead of putting the full request in the schedule field", () => {
    expect(routineRequestFromDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: ["future one-time routine"],
    }, prompt, new Date(2026, 7, 2, 12, 0))).toMatchObject({
      requestText: prompt,
      scheduleText: "on 2026-08-02 at 4:35 PM",
      scheduleKind: "one_shot",
      cadence: null,
      scheduleSupported: true,
      timingDefaulted: false,
    });
  });

  it("seeds every-week review without claiming the user supplied a day or time", () => {
    const weeklyPrompt =
      "Check my unread email every week from now until midnight today. Once you set it up, run it once to ensure it’s working properly.";
    expect(routineRequestFromDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: [
        "recurring routine",
        "routine cadence:v1:1:week",
        "routine schedule seed: every week",
        "routine timing defaulted",
        "explicit run once requested",
        "end at midnight requested",
      ],
    }, weeklyPrompt)).toEqual({
      requestText: weeklyPrompt,
      scheduleText: "every week",
      scheduleKind: "recurring",
      cadence: { interval: 1, unit: "week" },
      scheduleSupported: true,
      timingDefaulted: true,
      cadenceBoundaryConflict: true,
      runOnceRequested: true,
      endBoundary: "midnight",
    });
  });

  it.each([
    ["Check unread email every 5 minutes.", 5, "minute", "every 5 minutes", true, false],
    ["Check unread email once per hour.", 1, "hour", "every 1 hour", true, false],
    ["Schedule a daily unread email check.", 1, "day", "every day", true, true],
    ["Schedule a monthly unread email check.", 1, "month", "every month", true, true],
    ["Schedule a quarterly unread email check.", 1, "quarter", "every quarter", true, true],
    ["Schedule an annual unread email check.", 1, "year", "every year", true, true],
    ["Check unread email every 2 weeks.", 2, "week", "every 2 weeks", true, true],
  ] as const)(
    "preserves typed recurrence for %s",
    (requestText, interval, unit, scheduleText, scheduleSupported, timingDefaulted) => {
      const signals = [
        "recurring routine",
        `routine cadence:v1:${interval}:${unit}`,
        `routine schedule seed: ${scheduleText}`,
      ];
      if (timingDefaulted) signals.push("routine timing defaulted");
      if (!scheduleSupported) signals.push("routine schedule unsupported");
      expect(routineRequestFromDecision({
        decision_source: "routine_scheduler_filter",
        matched_signals: signals,
      }, requestText)).toMatchObject({
        requestText,
        scheduleText,
        cadence: { interval, unit },
        scheduleKind: "recurring",
        scheduleSupported,
        timingDefaulted,
      });
    },
  );

  it("opens unsupported cadence review with the exact request and editable seed", () => {
    const requestText = "Check unread email every 30 seconds.";
    expect(routineRequestFromDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: [
        "recurring routine",
        "routine schedule seed: every 30 seconds",
        "routine schedule unsupported",
        "routine schedule clarification required",
      ],
    }, requestText)).toMatchObject({
      requestText,
      scheduleText: "every 30 seconds",
      scheduleKind: "recurring",
      cadence: null,
      scheduleSupported: false,
    });
  });
});

describe("Routine handoff safety", () => {
  it("does not open the Routine flow for an ordinary file request", () => {
    expect(oneTimeRoutineRequestFromDecision({
      decision_source: "filesystem_action_filter",
      matched_signals: ["direct file read"],
    }, prompt)).toBeNull();
  });

  it("defers future file work to the classifier instead of reading immediately", () => {
    expect(shouldDeferFileShortcutForRoutine(prompt)).toBe(true);
    expect(shouldDeferFileShortcutForRoutine("Read /Users/example/report.md now.")).toBe(false);
  });

  it("completes the accepted turn before opening the genuine review flow", async () => {
    const complete = vi.fn(async () => undefined);
    const onOpenRoutine = vi.fn();
    await expect(completeOneTimeRoutineHandoff({
      decision: { decision_source: "routine_scheduler_filter", matched_signals: ["future one-time routine"] },
      prompt, onOpenRoutine, complete, content: "Review it.", status: "Review the schedule",
    })).resolves.toBe(true);
    expect(complete).toHaveBeenCalledWith("Review it.", "Review the schedule", true);
    expect(onOpenRoutine).toHaveBeenCalledWith(expect.objectContaining({
      requestText: prompt,
      scheduleKind: "one_shot",
    }));
  });

  it("opens recurring review without claiming the immediate run or midnight stop happened", async () => {
    const complete = vi.fn(async () => undefined);
    const onOpenRoutine = vi.fn();
    const recurringPrompt =
      "Check my unread email every hour until midnight. Once you set it up, run it once to ensure it’s working properly.";
    await expect(completeOneTimeRoutineHandoff({
      decision: {
        decision_source: "routine_scheduler_filter",
        matched_signals: [
          "recurring routine",
          "routine cadence:v1:1:hour",
          "routine schedule seed: every 1 hour",
          "routine target private app:v1:mail",
          "explicit run once requested",
          "end at midnight requested",
        ],
      },
      prompt: recurringPrompt,
      onOpenRoutine,
      complete,
      content: "Review it.",
      status: "Review the schedule",
    })).resolves.toBe(true);
    expect(onOpenRoutine).toHaveBeenCalledWith({
      requestText: recurringPrompt,
      scheduleText: "every 1 hour",
      scheduleKind: "recurring",
      cadence: { interval: 1, unit: "hour" },
      scheduleSupported: true,
      timingDefaulted: false,
      cadenceBoundaryConflict: false,
      runOnceRequested: true,
      endBoundary: "midnight",
      targetAction: { kind: "read_unread_mail" },
    });
  });
});
