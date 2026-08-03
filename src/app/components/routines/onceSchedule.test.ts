import { describe, expect, it } from "vitest";
import {
  nextOnceSchedule,
  onceScheduleIsPast,
  todayDate,
} from "./onceSchedule";

describe("one-time routine scheduling", () => {
  it("seeds and validates in the selected timezone, not the Mac timezone", () => {
    const now = new Date("2026-07-21T02:02:30.000Z");
    expect(todayDate(now, "America/Los_Angeles")).toBe("2026-07-20");
    expect(nextOnceSchedule(now, "America/Los_Angeles")).toEqual({
      date: "2026-07-20",
      time: "19:12",
    });
    expect(
      onceScheduleIsPast(
        "2026-07-20",
        "19:08",
        "America/Los_Angeles",
        now,
      ),
    ).toBe(false);
    expect(
      onceScheduleIsPast(
        "2026-07-20",
        "19:02",
        "America/Los_Angeles",
        now,
      ),
    ).toBe(true);
  });

  it("rolls the selected timezone's late-night default into tomorrow", () => {
    const now = new Date("2026-07-22T06:54:30.000Z");
    expect(nextOnceSchedule(now, "America/Los_Angeles")).toEqual({
      date: "2026-07-22",
      time: "00:04",
    });
  });

  it("matches the backend at spring gaps and fall overlaps", () => {
    const beforeSpringGap = new Date("2026-03-08T06:50:00.000Z");
    expect(nextOnceSchedule(beforeSpringGap, "America/New_York")).toEqual({
      date: "2026-03-08",
      time: "03:00",
    });
    expect(
      onceScheduleIsPast(
        "2026-03-08",
        "02:30",
        "America/New_York",
        beforeSpringGap,
      ),
    ).toBe(false);
    expect(
      onceScheduleIsPast(
        "2026-03-08",
        "02:30",
        "America/New_York",
        new Date("2026-03-08T07:01:00.000Z"),
      ),
    ).toBe(true);

    expect(
      onceScheduleIsPast(
        "2026-11-01",
        "01:30",
        "America/New_York",
        new Date("2026-11-01T05:15:00.000Z"),
      ),
    ).toBe(false);
    expect(
      onceScheduleIsPast(
        "2026-11-01",
        "01:30",
        "America/New_York",
        new Date("2026-11-01T05:45:00.000Z"),
      ),
    ).toBe(true);
  });

  it("fails closed for invalid dates or timezones", () => {
    const now = new Date("2026-07-21T10:00:00.000Z");
    expect(onceScheduleIsPast("2026-02-30", "10:08", "UTC", now)).toBe(true);
    expect(onceScheduleIsPast("2026-07-21", "10:08", "Not/AZone", now)).toBe(
      true,
    );
  });
});
