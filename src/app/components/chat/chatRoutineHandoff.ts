import type {
  RoutineCadence,
  RoutineCadenceUnit,
  RoutineHandoffRequest,
} from "../routines/routineDraft";

type RouteDecision = {
  decision_source: string;
  matched_signals: string[];
};

const ONE_TIME_SIGNAL = "future one-time routine";
const RECURRING_SIGNAL = "recurring routine";
const CADENCE_SIGNAL_PREFIX = "routine cadence:v1:";
const SCHEDULE_SEED_PREFIX = "routine schedule seed: ";
const TIMING_DEFAULTED_SIGNAL = "routine timing defaulted";
const UNSUPPORTED_SCHEDULE_SIGNAL = "routine schedule unsupported";
const RUN_ONCE_SIGNAL = "explicit run once requested";
const END_AT_MIDNIGHT_SIGNAL = "end at midnight requested";
const PRIVATE_APP_TARGET_PREFIX = "routine target private app:v1:";
const CADENCE_UNITS = new Set<RoutineCadenceUnit>([
  "minute",
  "hour",
  "day",
  "week",
  "month",
  "quarter",
  "year",
]);

function cadenceFromSignals(signals: string[]): RoutineCadence | null {
  const raw = signals.find((signal) => signal.startsWith(CADENCE_SIGNAL_PREFIX));
  if (!raw) return null;
  const [intervalText, unitText, extra] = raw
    .slice(CADENCE_SIGNAL_PREFIX.length)
    .split(":");
  const interval = Number(intervalText);
  if (
    extra !== undefined ||
    !Number.isSafeInteger(interval) ||
    interval <= 0 ||
    !CADENCE_UNITS.has(unitText as RoutineCadenceUnit)
  ) return null;
  return { interval, unit: unitText as RoutineCadenceUnit };
}

function localDateKey(date: Date) {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

function oneTimeScheduleSeed(prompt: string, now: Date) {
  const normalized = prompt.split(/\s+/).join(" ").trim();
  const dated = normalized.match(
    /\bon\s+(20\d{2}-\d{2}-\d{2})\s+at\s+((?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:a\.?m\.?|p\.?m\.?)?)/i,
  );
  if (dated) return `on ${dated[1]} at ${dated[2].trim()}`;

  const tomorrow = normalized.match(
    /\btomorrow\s+at\s+((?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:a\.?m\.?|p\.?m\.?)?)/i,
  );
  if (tomorrow) return `tomorrow at ${tomorrow[1].trim()}`;

  const today = normalized.match(
    /\bat\s+((?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:a\.?m\.?|p\.?m\.?)?)\s+today\b/i,
  );
  if (today) return `on ${localDateKey(now)} at ${today[1].trim()}`;

  return null;
}

export function routineRequestFromDecision(
  decision: Pick<RouteDecision, "decision_source" | "matched_signals">,
  prompt: string,
  now = new Date(),
): RoutineHandoffRequest | null {
  if (decision.decision_source !== "routine_scheduler_filter") return null;
  const recurring = decision.matched_signals.includes(RECURRING_SIGNAL);
  const cadence = recurring ? cadenceFromSignals(decision.matched_signals) : null;
  const oneTime = decision.matched_signals.includes(ONE_TIME_SIGNAL);
  const recurringSeed = decision.matched_signals
    .find((signal) => signal.startsWith(SCHEDULE_SEED_PREFIX))
    ?.slice(SCHEDULE_SEED_PREFIX.length)
    .trim();
  const unsupportedSchedule = decision.matched_signals.includes(
    UNSUPPORTED_SCHEDULE_SIGNAL,
  );
  const scheduleText = recurring && recurringSeed && (cadence || unsupportedSchedule)
    ? recurringSeed
    : oneTime
      ? oneTimeScheduleSeed(prompt, now)
      : null;
  if (!scheduleText) return null;
  const endsAtMidnight = decision.matched_signals.includes(
    END_AT_MIDNIGHT_SIGNAL,
  );
  const coarseCadence = cadence
    ? ["day", "week", "month", "quarter", "year"].includes(cadence.unit)
    : false;
  const privateAppTarget = decision.matched_signals
    .find((signal) => signal.startsWith(PRIVATE_APP_TARGET_PREFIX))
    ?.slice(PRIVATE_APP_TARGET_PREFIX.length);
  return {
    requestText: prompt,
    scheduleText,
    scheduleKind: recurring ? "recurring" : "one_shot",
    cadence,
    scheduleSupported: !unsupportedSchedule,
    timingDefaulted: decision.matched_signals.includes(TIMING_DEFAULTED_SIGNAL),
    cadenceBoundaryConflict: coarseCadence && endsAtMidnight,
    runOnceRequested: decision.matched_signals.includes(RUN_ONCE_SIGNAL),
    endBoundary: endsAtMidnight ? "midnight" : null,
    ...(privateAppTarget === "mail"
      ? { targetAction: { kind: "read_unread_mail" as const } }
      : {}),
  };
}

export function oneTimeRoutineRequestFromDecision(
  decision: Pick<RouteDecision, "decision_source" | "matched_signals">,
  prompt: string,
) {
  const request = routineRequestFromDecision(decision, prompt);
  return request?.scheduleKind === "one_shot" ? request.requestText : null;
}

export function shouldDeferFileShortcutForRoutine(prompt: string) {
  return /\b(?:at\s+\d{1,2}(?::\d{2})?\s*(?:a\.?m\.?|p\.?m\.?)|tomorrow|later\s+today|next\s+(?:weekday|week|month))\b/i.test(prompt);
}

export async function completeOneTimeRoutineHandoff(options: {
  decision: RouteDecision;
  prompt: string;
  onOpenRoutine?: (request: RoutineHandoffRequest) => void;
  complete: (content: string, status: string, assistantOutcome: boolean) => Promise<void>;
  content: string;
  status: string;
}) {
  const request = routineRequestFromDecision(options.decision, options.prompt);
  if (!request || !options.onOpenRoutine) return false;
  await options.complete(options.content, options.status, true);
  options.onOpenRoutine(request);
  return true;
}
