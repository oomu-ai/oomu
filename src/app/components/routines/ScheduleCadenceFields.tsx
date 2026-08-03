"use client";

import { useEffect, useState } from "react";
import type { RoutineTranslate } from "./routineLabels";
import {
  nextOnceSchedule,
  onceScheduleIsPast,
  todayDate,
} from "./onceSchedule";
import { RoutineIntervalEditor } from "./RoutineIntervalEditor";
import type { RoutineCadenceUnit } from "./routineDraft";
import {
  composeScheduleText,
  defaultsFromScheduleSeed,
  type ScheduleFrequency,
} from "./routineScheduleCadence";

const FREQUENCIES: ScheduleFrequency[] = [
  "interval",
  "daily",
  "weekly",
  "once",
  "custom",
];

const DAYS = [
  { value: 1, key: "mon" },
  { value: 2, key: "tue" },
  { value: 3, key: "wed" },
  { value: 4, key: "thu" },
  { value: 5, key: "fri" },
  { value: 6, key: "sat" },
  { value: 0, key: "sun" },
] as const;

const WEEKDAYS = [1, 2, 3, 4, 5];

function tomorrowDate() {
  const date = new Date();
  date.setDate(date.getDate() + 1);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function ScheduleBuilder({
  disabled,
  initialScheduleText = "",
  onScheduleChange,
  t,
  timezone,
}: {
  disabled: boolean;
  initialScheduleText?: string;
  onScheduleChange: (schedule: string, cadence: string) => void;
  t: RoutineTranslate;
  timezone: string;
}) {
  const normalizedInitialSchedule = initialScheduleText.trim().toLowerCase();
  const handedOffSchedule = defaultsFromScheduleSeed(normalizedInitialSchedule);
  const [frequency, setFrequency] = useState<ScheduleFrequency>(
    handedOffSchedule
      ? handedOffSchedule.frequency
      : initialScheduleText.trim()
        ? "custom"
        : "daily",
  );
  const [time, setTime] = useState(handedOffSchedule?.time ?? "09:00");
  const [date, setDate] = useState(tomorrowDate);
  const [intervalCount, setIntervalCount] = useState(
    handedOffSchedule?.cadence?.interval ?? 1,
  );
  const [intervalUnit, setIntervalUnit] = useState<RoutineCadenceUnit>(
    handedOffSchedule?.cadence?.unit ?? "hour",
  );
  const [weekDays, setWeekDays] = useState<number[]>(
    handedOffSchedule?.weekDays.length ? handedOffSchedule.weekDays : WEEKDAYS,
  );
  const [customText, setCustomText] = useState(() =>
    initialScheduleText.trim() || t("routines.default_schedule"),
  );
  const onceIsPast =
    frequency === "once" && onceScheduleIsPast(date, time, timezone);

  useEffect(() => {
    onScheduleChange(
      onceIsPast
        ? ""
        : composeScheduleText({
            customText,
            date,
            frequency,
            intervalCount,
            intervalUnit,
            time,
            weekDays,
          }),
      t(`routines.cadence_${frequency}`),
    );
  }, [
    customText,
    date,
    frequency,
    intervalCount,
    intervalUnit,
    onScheduleChange,
    onceIsPast,
    t,
    time,
    weekDays,
  ]);

  function selectFrequency(next: ScheduleFrequency) {
    if (next === "once") {
      const initial = nextOnceSchedule(new Date(), timezone);
      setDate(initial.date);
      setTime(initial.time);
    }
    setFrequency(next);
  }

  function toggleDay(day: number) {
    setWeekDays((current) => {
      if (current.includes(day)) {
        return current.length === 1
          ? current
          : current.filter((value) => value !== day);
      }
      return DAYS.filter(({ value }) => [...current, day].includes(value)).map(
        ({ value }) => value,
      );
    });
  }

  const isWeekdays =
    weekDays.length === WEEKDAYS.length &&
    WEEKDAYS.every((day) => weekDays.includes(day));

  return (
    <fieldset className="mt-7 grid gap-4">
      <legend className="text-sm font-semibold">{t("routines.when")}</legend>
      <div
        aria-label={t("routines.frequency")}
        className="grid grid-cols-5 rounded-[var(--radius-sm)] bg-[var(--fill-hover)] p-1"
        role="group"
      >
        {FREQUENCIES.map((item) => (
          <button
            aria-pressed={frequency === item}
            className={`rounded-[var(--radius-sm)] px-2 py-2 text-xs font-semibold transition-colors disabled:opacity-50 ${
              frequency === item
                ? "bg-[var(--background)] shadow-sm"
                : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"
            }`}
            disabled={disabled}
            key={item}
            onClick={() => selectFrequency(item)}
            type="button"
          >
            {t(`routines.frequency_${item}`)}
          </button>
        ))}
      </div>

      {frequency === "interval" ? (
        <RoutineIntervalEditor
          count={intervalCount}
          disabled={disabled}
          onCountChange={setIntervalCount}
          onUnitChange={setIntervalUnit}
          t={t}
          unit={intervalUnit}
        />
      ) : null}

      {frequency === "daily" || frequency === "weekly" ? (
        <label className="grid max-w-xs gap-2 text-sm font-semibold">
          {t("routines.time")}
          <input
            className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
            disabled={disabled}
            onChange={(event) => setTime(event.target.value)}
            type="time"
            value={time}
          />
        </label>
      ) : null}

      {frequency === "weekly" ? (
        <div className="grid gap-2">
          <span className="text-sm font-semibold">{t("routines.days")}</span>
          <div className="flex flex-wrap gap-2">
            <button
              aria-pressed={isWeekdays}
              className="rounded-full border px-3 py-1.5 text-xs font-semibold aria-pressed:border-[var(--foreground)] aria-pressed:bg-[var(--fill-selected)]"
              disabled={disabled}
              onClick={() => setWeekDays(WEEKDAYS)}
              type="button"
            >
              {t("routines.weekdays")}
            </button>
            {DAYS.map(({ key, value }) => (
              <button
                aria-pressed={weekDays.includes(value)}
                className="rounded-full border px-3 py-1.5 text-xs font-semibold aria-pressed:border-[var(--foreground)] aria-pressed:bg-[var(--fill-selected)]"
                disabled={disabled}
                key={key}
                onClick={() => toggleDay(value)}
                type="button"
              >
                {t(`routines.day_${key}`)}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {frequency === "once" ? (
        <div className="grid max-w-lg grid-cols-2 gap-3">
          <label className="grid gap-2 text-sm font-semibold">
            {t("routines.date")}
            <input
              className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
              aria-describedby={onceIsPast ? "routine-once-error" : undefined}
              aria-invalid={onceIsPast}
              disabled={disabled}
              min={todayDate(new Date(), timezone)}
              onChange={(event) => setDate(event.target.value)}
              type="date"
              value={date}
            />
          </label>
          <label className="grid gap-2 text-sm font-semibold">
            {t("routines.time")}
            <input
              className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
              aria-describedby={onceIsPast ? "routine-once-error" : undefined}
              aria-invalid={onceIsPast}
              disabled={disabled}
              onChange={(event) => setTime(event.target.value)}
              type="time"
              value={time}
            />
          </label>
          {onceIsPast ? (
            <p
              aria-live="polite"
              className="col-span-2 text-sm text-[var(--danger)]"
              id="routine-once-error"
            >
              {t("routines.once_past")}
            </p>
          ) : null}
        </div>
      ) : null}

      {frequency === "custom" ? (
        <label className="grid gap-2 text-sm font-semibold">
          {t("routines.custom_schedule")}
          <input
            className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
            disabled={disabled}
            onChange={(event) => setCustomText(event.target.value)}
            value={customText}
          />
          <span className="text-xs font-normal text-[var(--foreground-muted)]">
            {t("routines.custom_help")}
          </span>
        </label>
      ) : null}
    </fieldset>
  );
}
