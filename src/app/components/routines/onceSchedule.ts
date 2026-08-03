type ZonedMinute = {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
};

const zonedFormatters = new Map<string, Intl.DateTimeFormat>();

function zonedFormatter(timezone: string) {
  const cached = zonedFormatters.get(timezone);
  if (cached) return cached;
  const formatter = new Intl.DateTimeFormat("en-US-u-ca-gregory-nu-latn", {
    timeZone: timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  zonedFormatters.set(timezone, formatter);
  return formatter;
}

function zonedMinute(date: Date, timezone: string): ZonedMinute | null {
  if (!Number.isFinite(date.getTime())) return null;
  try {
    const values = Object.fromEntries(
      zonedFormatter(timezone)
        .formatToParts(date)
        .filter(({ type }) => type !== "literal")
        .map(({ type, value }) => [type, Number(value)]),
    );
    const result = {
      year: values.year,
      month: values.month,
      day: values.day,
      hour: values.hour,
      minute: values.minute,
    };
    return Object.values(result).every(Number.isFinite) ? result : null;
  } catch {
    return null;
  }
}

function dateValue(value: ZonedMinute) {
  return `${value.year}-${String(value.month).padStart(2, "0")}-${String(value.day).padStart(2, "0")}`;
}

function timeValue(value: ZonedMinute) {
  return `${String(value.hour).padStart(2, "0")}:${String(value.minute).padStart(2, "0")}`;
}

function parsedWallMinute(date: string, time: string): ZonedMinute | null {
  const dateParts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const timeParts = /^(\d{2}):(\d{2})$/.exec(time);
  if (!dateParts || !timeParts) return null;
  const result = {
    year: Number(dateParts[1]),
    month: Number(dateParts[2]),
    day: Number(dateParts[3]),
    hour: Number(timeParts[1]),
    minute: Number(timeParts[2]),
  };
  if (
    result.month < 1 ||
    result.month > 12 ||
    result.day < 1 ||
    result.day > 31 ||
    result.hour > 23 ||
    result.minute > 59
  ) {
    return null;
  }
  return result;
}

function sameWallMinute(left: ZonedMinute, right: ZonedMinute) {
  return (
    left.year === right.year &&
    left.month === right.month &&
    left.day === right.day &&
    left.hour === right.hour &&
    left.minute === right.minute
  );
}

function exactWallInstant(value: ZonedMinute, timezone: string) {
  const wallUtc = Date.UTC(
    value.year,
    value.month - 1,
    value.day,
    value.hour,
    value.minute,
  );
  const normalized = new Date(wallUtc);
  if (
    normalized.getUTCFullYear() !== value.year ||
    normalized.getUTCMonth() + 1 !== value.month ||
    normalized.getUTCDate() !== value.day
  ) {
    return null;
  }

  const offsets = new Set<number>();
  for (const hours of [-36, -12, 0, 12, 36]) {
    const instant = wallUtc + hours * 60 * 60 * 1_000;
    const parts = zonedMinute(new Date(instant), timezone);
    if (!parts) return null;
    offsets.add(
      Date.UTC(
        parts.year,
        parts.month - 1,
        parts.day,
        parts.hour,
        parts.minute,
      ) - Math.floor(instant / 60_000) * 60_000,
    );
  }

  const matches = [...offsets]
    .map((offset) => wallUtc - offset)
    .filter((instant) => {
      const parts = zonedMinute(new Date(instant), timezone);
      return parts !== null && sameWallMinute(parts, value);
    });
  return matches.length > 0 ? Math.min(...matches) : null;
}

function resolvedWallInstant(value: ZonedMinute, timezone: string) {
  const direct = exactWallInstant(value, timezone);
  if (direct !== null) return direct;

  // Match the backend's DST-gap policy: advance to the first real local
  // minute, up to two hours, rather than silently interpreting another zone.
  const wallUtc = Date.UTC(
    value.year,
    value.month - 1,
    value.day,
    value.hour,
    value.minute,
  );
  for (let offset = 1; offset <= 120; offset += 1) {
    const shifted = new Date(wallUtc + offset * 60_000);
    const candidate = {
      year: shifted.getUTCFullYear(),
      month: shifted.getUTCMonth() + 1,
      day: shifted.getUTCDate(),
      hour: shifted.getUTCHours(),
      minute: shifted.getUTCMinutes(),
    };
    const instant = exactWallInstant(candidate, timezone);
    if (instant !== null) return instant;
  }
  return null;
}

function systemTimezone() {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

export function todayDate(now = new Date(), timezone = systemTimezone()) {
  const value = zonedMinute(now, timezone);
  return value ? dateValue(value) : "";
}

export function nextOnceSchedule(now = new Date(), timezone = systemTimezone()) {
  const target = new Date(
    Math.floor((now.getTime() + 10 * 60_000) / 60_000) * 60_000,
  );
  const value = zonedMinute(target, timezone);
  return value
    ? { date: dateValue(value), time: timeValue(value) }
    : { date: "", time: "" };
}

export function onceScheduleIsPast(
  date: string,
  time: string,
  timezoneOrNow: string | Date = systemTimezone(),
  referenceNow = new Date(),
) {
  const timezone =
    typeof timezoneOrNow === "string" ? timezoneOrNow : systemTimezone();
  const now = timezoneOrNow instanceof Date ? timezoneOrNow : referenceNow;
  const value = parsedWallMinute(date, time);
  if (!value) return true;
  const instant = resolvedWallInstant(value, timezone);
  return instant === null || instant <= now.getTime();
}
