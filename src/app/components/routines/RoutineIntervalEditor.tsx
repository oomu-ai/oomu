import type { RoutineCadenceUnit } from "./routineDraft";
import type { RoutineTranslate } from "./routineLabels";
import { INTERVAL_UNITS } from "./routineScheduleCadence";

export function RoutineIntervalEditor({
  count,
  disabled,
  onCountChange,
  onUnitChange,
  t,
  unit,
}: {
  count: number;
  disabled: boolean;
  onCountChange: (count: number) => void;
  onUnitChange: (unit: RoutineCadenceUnit) => void;
  t: RoutineTranslate;
  unit: RoutineCadenceUnit;
}) {
  return (
    <div
      aria-label={t("routines.interval")}
      className="flex max-w-lg items-end gap-3"
      role="group"
    >
      <span className="pb-2 text-sm font-semibold">{t("routines.every")}</span>
      <label className="grid gap-2 text-sm font-semibold">
        <span className="sr-only">{t("routines.interval_count")}</span>
        <input
          aria-label={t("routines.interval_count")}
          className="w-24 rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
          disabled={disabled}
          min={1}
          onChange={(event) =>
            onCountChange(Math.max(1, Number(event.target.value) || 1))
          }
          type="number"
          value={count}
        />
      </label>
      <label className="grid min-w-40 gap-2 text-sm font-semibold">
        <span className="sr-only">{t("routines.interval_unit")}</span>
        <select
          aria-label={t("routines.interval_unit")}
          className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
          disabled={disabled}
          onChange={(event) => onUnitChange(event.target.value as RoutineCadenceUnit)}
          value={unit}
        >
          {INTERVAL_UNITS.map((option) => (
            <option key={option} value={option}>
              {t(`routines.interval_unit_${option}`)}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}
