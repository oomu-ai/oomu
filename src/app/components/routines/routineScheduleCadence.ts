import type { RoutineCadence, RoutineCadenceUnit } from "./routineDraft";

export type ScheduleFrequency =
  | "interval"
  | "hourly"
  | "daily"
  | "weekly"
  | "once"
  | "custom";

export const INTERVAL_UNITS: RoutineCadenceUnit[] = [
  "minute",
  "hour",
  "day",
  "week",
  "month",
  "quarter",
  "year",
];

export function cadenceFromScheduleSeed(seed: string): RoutineCadence | null {
  const normalized = seed.trim().toLowerCase();
  const named: Record<string, RoutineCadence> = {
    minutely: { interval: 1, unit: "minute" },
    hourly: { interval: 1, unit: "hour" },
    daily: { interval: 1, unit: "day" },
    weekly: { interval: 1, unit: "week" },
    fortnightly: { interval: 2, unit: "week" },
    monthly: { interval: 1, unit: "month" },
    quarterly: { interval: 1, unit: "quarter" },
    yearly: { interval: 1, unit: "year" },
    annually: { interval: 1, unit: "year" },
  };
  if (named[normalized]) return named[normalized];
  const match = normalized.match(
    /^every\s+(?:(\d+)\s+)?(minutes?|hours?|days?|weeks?|months?|quarters?|years?)$/,
  );
  if (!match) return null;
  const interval = Number(match[1] ?? 1);
  const unit = match[2].replace(/s$/, "") as RoutineCadenceUnit;
  return Number.isSafeInteger(interval) && interval > 0 && INTERVAL_UNITS.includes(unit)
    ? { interval, unit }
    : null;
}

export type ScheduleSeedDefaults = {
  frequency: "interval" | "daily" | "weekly";
  cadence: RoutineCadence | null;
  time: string;
  weekDays: number[];
};

export function defaultsFromScheduleSeed(seed: string): ScheduleSeedDefaults | null {
  const normalized = seed.trim().toLowerCase();
  const cadence = cadenceFromScheduleSeed(normalized);
  if (cadence) {
    return { frequency: "interval", cadence, time: "09:00", weekDays: [] };
  }
  if (normalized === "every weekday") {
    return { frequency: "weekly", cadence: null, time: "09:00", weekDays: [1, 2, 3, 4, 5] };
  }
  if (normalized === "every weekend") {
    return { frequency: "weekly", cadence: null, time: "09:00", weekDays: [0, 6] };
  }
  const weekday = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]
    .findIndex((day) => normalized === `every ${day}`);
  if (weekday >= 0) {
    return { frequency: "weekly", cadence: null, time: "09:00", weekDays: [weekday] };
  }
  const daypartTimes: Record<string, string> = {
    "every morning": "09:00",
    "every afternoon": "13:00",
    "every evening": "18:00",
    "every night": "21:00",
  };
  const time = daypartTimes[normalized];
  return time
    ? { frequency: "daily", cadence: null, time, weekDays: [] }
    : null;
}

export function composeScheduleText({
  customText,
  date,
  frequency,
  hourlyInterval = 1,
  intervalCount = 1,
  intervalUnit = "hour",
  time,
  weekDays,
}: {
  customText: string;
  date: string;
  frequency: ScheduleFrequency;
  hourlyInterval?: number;
  intervalCount?: number;
  intervalUnit?: RoutineCadenceUnit;
  time: string;
  weekDays: number[];
}) {
  if (frequency === "interval") {
    const suffix = intervalCount === 1 ? intervalUnit : `${intervalUnit}s`;
    return `every ${intervalCount} ${suffix}`;
  }
  if (frequency === "hourly") {
    return hourlyInterval === 1
      ? "every hour"
      : `0 */${hourlyInterval} * * *`;
  }
  if (frequency === "daily") return `daily at ${time}`;
  if (frequency === "weekly") {
    const [hour, minute] = time.split(":");
    return `${minute} ${hour} * * ${weekDays.join(",")}`;
  }
  if (frequency === "once") return `on ${date} at ${time}`;
  return customText.trim();
}
