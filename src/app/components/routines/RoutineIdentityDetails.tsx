"use client";

import { useI18n } from "@/context/I18nContext";
import type { RoutineRecord } from "./routineClient";

export function RoutineIdentityDetails({ routine }: { routine: RoutineRecord }) {
  const { t } = useI18n();

  return (
    <details className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--fill-subtle)] px-4 py-3 text-sm">
      <summary className="cursor-pointer font-semibold">
        {t("routines.identity_details")}
      </summary>
      <dl className="mt-3 grid gap-3 sm:grid-cols-2">
        <div>
          <dt className="text-xs text-[var(--foreground-muted)]">
            {t("routines.routine_id")}
          </dt>
          <dd className="mt-1 select-all break-all font-mono text-xs">
            {routine.routineId}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-[var(--foreground-muted)]">
            {t("routines.workflow_version")}
          </dt>
          <dd className="mt-1 font-mono text-xs">
            {routine.workflowVersion ?? "—"}
          </dd>
        </div>
      </dl>
    </details>
  );
}
