const TASK_STATES = new Set([
  "queued",
  "planning",
  "awaiting_approval",
  "running",
  "blocked",
  "completed",
  "failed",
  "cancelled",
]);

export type RoutineTranslate = (
  key: string,
  values?: Record<string, string | number>,
) => string;

function clockTime(hour: number, minute: number) {
  const value = new Date(Date.UTC(2020, 0, 1, hour, minute));
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    timeZone: "UTC",
  }).format(value);
}

function parsedClock(hourText: string, minuteText: string) {
  if (!/^\d{1,2}$/.test(hourText) || !/^\d{1,2}$/.test(minuteText)) {
    return null;
  }
  const hour = Number(hourText);
  const minute = Number(minuteText);
  return hour <= 23 && minute <= 59 ? clockTime(hour, minute) : null;
}

function formattedInstant(timestamp: number, timezone: string) {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return null;

  const options: Intl.DateTimeFormatOptions = {
    dateStyle: "medium",
    timeStyle: "short",
  };
  try {
    return new Intl.DateTimeFormat(undefined, {
      ...options,
      timeZone: timezone,
    }).format(date);
  } catch {
    return new Intl.DateTimeFormat(undefined, options).format(date);
  }
}

export function formatRoutineTimestamp(timestamp: number, timezone?: string) {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return "";

  const options: Intl.DateTimeFormatOptions = {
    dateStyle: "medium",
    timeStyle: "short",
  };
  try {
    return new Intl.DateTimeFormat(undefined, {
      ...options,
      ...(timezone ? { timeZone: timezone } : {}),
    }).format(date);
  } catch {
    return new Intl.DateTimeFormat(undefined, options).format(date);
  }
}

export function humanTimezoneLabel(timezone: string) {
  const place = timezone.split("/").at(-1) || timezone;
  return place.replaceAll("_", " ");
}

function weeklyDayLabels(dayOfWeek: string, t: RoutineTranslate) {
  const keys: Record<string, string> = {
    "0": "sun",
    "1": "mon",
    "2": "tue",
    "3": "wed",
    "4": "thu",
    "5": "fri",
    "6": "sat",
  };
  const days = dayOfWeek.split(",");
  if (days.some((day) => !keys[day])) return null;
  return days.map((day) => t(`routines.day_${keys[day]}`)).join(", ");
}

/**
 * Converts the scheduler's stored expression into calm, human copy. Unknown
 * expressions deliberately collapse to a generic label instead of leaking a
 * cron string or another scheduler identifier onto the screen.
 */
export function humanScheduleSummary(
  scheduleExpression: string,
  timezone: string,
  t: RoutineTranslate,
) {
  const expression = scheduleExpression.trim().toLowerCase();

  const everyHours = /^every (\d{1,3}) hours?$/.exec(expression);
  if (everyHours) {
    const count = Number(everyHours[1]);
    if (count === 1) return t("routines.schedule_hourly");
    if (count > 1) return t("routines.schedule_every_hours", { count });
  }

  const daily = /^daily at (\d{1,2}):(\d{2})$/.exec(expression);
  if (daily) {
    const time = parsedClock(daily[1], daily[2]);
    if (time) return t("routines.schedule_daily_at", { time });
  }

  const oneShot = /^once:(\d{1,17})$/.exec(expression);
  if (oneShot) {
    const date = formattedInstant(Number(oneShot[1]), timezone);
    if (date) return t("routines.schedule_once", { date });
  }

  const cron = expression.split(/\s+/);
  if (cron.length === 5) {
    const [minutePart, hourPart, dayOfMonth, month, dayOfWeek] = cron;
    if (dayOfMonth === "*" && month === "*") {
      const everyMinutes = /^\*\/(\d{1,2})$/.exec(minutePart);
      if (everyMinutes && hourPart === "*" && dayOfWeek === "*") {
        const count = Number(everyMinutes[1]);
        if (count > 0 && count <= 59) {
          return t("routines.schedule_every_minutes", { count });
        }
      }

      const cronHours = /^\*\/(\d{1,2})$/.exec(hourPart);
      if (minutePart === "0" && cronHours && dayOfWeek === "*") {
        const count = Number(cronHours[1]);
        if (count === 1) return t("routines.schedule_hourly");
        if (count > 1 && count <= 23) {
          return t("routines.schedule_every_hours", { count });
        }
      }

      const time = parsedClock(hourPart, minutePart);
      if (time && dayOfWeek === "*") {
        return t("routines.schedule_daily_at", { time });
      }
      if (time && ["1-5", "1,2,3,4,5"].includes(dayOfWeek)) {
        return t("routines.schedule_weekdays_at", { time });
      }
      const days = time ? weeklyDayLabels(dayOfWeek, t) : null;
      if (time && days) {
        return t("routines.schedule_weekly_at", { days, time });
      }
    }
  }

  return t("routines.schedule_custom");
}

export function routineHistoryState(t: RoutineTranslate, state: string) {
  const normalized = state.trim().toLowerCase();
  return TASK_STATES.has(normalized)
    ? t(`tasks.state_${normalized}`)
    : t("routines.history_state_unknown");
}

export function backgroundStatusLabel(t: RoutineTranslate, state: string) {
  switch (state.trim().toLowerCase()) {
    case "on_verified":
    case "active":
      return t("routines.background_on_help");
    case "turning_on":
      return t("routines.background_turning_on_help");
    case "turning_off":
      return t("routines.background_turning_off_help");
    case "needs_attention":
      return t("routines.background_needs_attention_help");
    case "off":
      return t("routines.background_off_help");
    case "requires_approval":
      return t("routines.background_approval");
    case "paused":
      return t("routines.background_paused");
    case "unavailable":
      return t("routines.background_unavailable");
    case "requires_signed_install":
      return t("routines.background_signed_install");
    case "degraded":
      return t("routines.background_attention");
    default:
      return t("routines.background_unknown");
  }
}

export function routinePausedReasonLabel(t: RoutineTranslate, reason: string) {
  switch (reason.trim().toLowerCase()) {
    case "paused by user":
      return t("routines.pause_reason_user");
    case "paused by remote owner":
      return t("routines.pause_reason_remote");
    case "paused after repeated failures":
      return t("routines.pause_reason_failures");
    case "one-time routine completed":
      return t("routines.pause_reason_complete");
    case "routine delivery retry pending":
      return t("routines.pause_reason_delivery_retry");
    case "routine delivery needs review":
      return t("routines.pause_reason_delivery_review");
    default:
      return t("routines.pause_reason_other");
  }
}

/**
 * Delivery recovery has its own actionable status card. All other states,
 * including a delivered one-time task, still need the schedule explanation.
 */
export function shouldShowRoutinePausedReason(deliveryState?: string | null) {
  return deliveryState !== "retrying" && deliveryState !== "needs_review";
}

export function routineHistoryTime(t: RoutineTranslate, timestamp: number) {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) {
    return { dateTime: undefined, label: t("common.unknown") };
  }
  return {
    dateTime: date.toISOString(),
    label: new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(date),
  };
}

export function routineDeleteError(t: RoutineTranslate, cause: unknown) {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /routine_delete_(?:running|in_progress)|running routine/i.test(message)
    ? t("routines.delete_running")
    : t("routines.delete_failed");
}
