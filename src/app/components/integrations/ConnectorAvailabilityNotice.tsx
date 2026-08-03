"use client";

import { useI18n } from "@/context/I18nContext";

const AVAILABILITY_REASONS = new Set([
  "build_missing_oauth_client",
  "build_missing_oauth_broker",
  "unsupported_platform",
]);

export function ConnectorAvailabilityNotice({
  reasonCode,
  service,
}: {
  reasonCode?: string | null;
  service: string;
}) {
  const { t } = useI18n();
  const reason = reasonCode && AVAILABILITY_REASONS.has(reasonCode)
    ? reasonCode
    : "unknown";

  return <div className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4" role="status">
    <p className="text-sm font-semibold">{t(`connector_availability.${reason}.title`, { service })}</p>
    <p className="mt-1 text-sm text-[var(--foreground-muted)]">{t(`connector_availability.${reason}.reason`, { service })}</p>
    <p className="mt-2 text-sm">{t(`connector_availability.${reason}.next`, { service })}</p>
  </div>;
}
