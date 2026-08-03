import { useEffect, useRef, useState } from "react";

export type MacPermissionRecoveryState =
  | "not_requested"
  | "denied"
  | "limited"
  | "restricted"
  | "stale"
  | "timeout"
  | "unsupported";

const supportedCapabilities = new Set([
  "accessibility",
  "screen_control",
  "screen_capture",
  "microphone",
  "camera",
  "speech_recognition",
  "music",
  "photos",
  "contacts",
  "calendar",
  "reminders",
  "mail",
  "notes",
  "messages",
  "finder",
  "system_events",
  "files_and_folders",
  "full_disk_access",
  "local_network",
  "notifications",
]);

export type MacPermissionRecoveryDescriptor = {
  capabilityId: string;
  state: MacPermissionRecoveryState;
};

export function macPermissionRecoveryDescriptor(
  codeValue: string,
  capabilityValue: string | null,
): MacPermissionRecoveryDescriptor | null {
  const code = codeValue.trim().toLowerCase();
  const capabilityId = capabilityValue?.trim().toLowerCase() ?? "";
  if (!supportedCapabilities.has(capabilityId)) return null;
  if (!/(?:permission|authorization|access)/.test(code)) return null;
  const state: MacPermissionRecoveryState = code.includes("restricted")
    ? "restricted"
    : code.includes("unsupported")
      ? "unsupported"
      : code.includes("limited") || code.includes("write_only")
        ? "limited"
        : code.includes("stale")
          ? "stale"
          : code.includes("timeout")
            ? "timeout"
            : code.includes("denied") || code.includes("required")
              || code.includes("requires_settings") || code.includes("revoked")
              || code.includes("unavailable")
              ? "denied"
              : "not_requested";
  return { capabilityId, state };
}

type MacPermissionRecoveryCardProps = {
  boundary: string;
  code: string;
  descriptor: MacPermissionRecoveryDescriptor;
  recoveryId: string;
  onCancel?: (recoveryId: string) => Promise<void>;
  onCheck?: (recoveryId: string, capabilityId: string) => Promise<void>;
  onOpenSettings?: (recoveryId: string, capabilityId: string) => Promise<void>;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

function recoveryAvailability(state: MacPermissionRecoveryState) {
  const canCheck = state !== "restricted" && state !== "unsupported";
  return {
    canCheck,
    canOpenSettings: state === "denied" || state === "limited",
    savedCopyKey: canCheck ? "sprint_301.permission_recovery.saved_recoverable"
      : "sprint_301.permission_recovery.saved_terminal",
  };
}

export function MacPermissionRecoveryCard({
  boundary,
  code,
  descriptor,
  recoveryId,
  onCancel,
  onCheck,
  onOpenSettings,
  t,
}: MacPermissionRecoveryCardProps) {
  const cardRef = useRef<HTMLElement>(null);
  const [action, setAction] = useState<"open" | "check" | "cancel" | null>(null);
  const [failed, setFailed] = useState(false);
  const capabilityName = t(
    `sprint_299.permissions.capabilities.${descriptor.capabilityId}.name`,
  );
  const { canCheck, canOpenSettings, savedCopyKey } = recoveryAvailability(descriptor.state);

  useEffect(() => {
    cardRef.current?.focus({ preventScroll: true });
  }, [code, recoveryId]);

  async function perform(next: "open" | "check" | "cancel") {
    if (action) return;
    setFailed(false);
    setAction(next);
    try {
      if (next === "cancel") {
        if (!onCancel) return;
        await onCancel(recoveryId);
      } else if (next === "open") {
        if (!onOpenSettings) return;
        await onOpenSettings(recoveryId, descriptor.capabilityId);
      } else {
        if (!onCheck) return;
        await onCheck(recoveryId, descriptor.capabilityId);
      }
    } catch {
      setFailed(true);
    } finally {
      setAction(null);
    }
  }

  return (
    <section
      aria-labelledby={`mac-permission-recovery-${recoveryId}`}
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--warning)] bg-[var(--warning-background)] px-5 py-4 text-[var(--foreground)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
      data-mac-permission-capability={descriptor.capabilityId}
      data-mac-permission-state={descriptor.state}
      ref={cardRef}
      role="alert"
      tabIndex={-1}
    >
      <h3 className="text-sm font-semibold" id={`mac-permission-recovery-${recoveryId}`}>
        {t("sprint_301.permission_recovery.title", { capability: capabilityName })}
      </h3>
      <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
        {t(`sprint_301.permission_recovery.${descriptor.state}_body`, {
          capability: capabilityName,
        })}
      </p>
      <p className="mt-2 text-xs font-medium text-[var(--foreground)]">
        {t(savedCopyKey)}
      </p>
      <div className="mt-4 flex flex-wrap gap-2">
        {canOpenSettings ? (
          <button
            className="min-h-10 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50"
            disabled={Boolean(action) || !onOpenSettings}
            onClick={() => void perform("open")}
            type="button"
          >
            {action === "open"
              ? t("sprint_301.permission_recovery.opening")
              : t("sprint_301.permission_recovery.open_settings")}
          </button>
        ) : null}
        {canCheck ? (
          <button
            className="min-h-10 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50"
            disabled={Boolean(action) || !onCheck}
            onClick={() => void perform("check")}
            type="button"
          >
            {action === "check"
              ? t("sprint_301.permission_recovery.checking")
              : t("sprint_301.permission_recovery.check_again")}
          </button>
        ) : null}
        <button
          className="min-h-10 rounded-[var(--radius-sm)] px-3 py-2 text-sm font-medium text-[var(--foreground-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50"
          disabled={Boolean(action) || !onCancel}
          onClick={() => void perform("cancel")}
          type="button"
        >
          {t("sprint_301.permission_recovery.cancel")}
        </button>
      </div>
      {failed ? (
        <p className="mt-3 text-xs font-medium text-[var(--destructive)]" role="alert">
          {t("sprint_301.permission_recovery.action_failed")}
        </p>
      ) : null}
      <details className="mt-3 text-xs text-[var(--foreground-muted)]">
        <summary className="cursor-pointer font-semibold outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]">
          {t("sprint_301.auto_route_recovery.technical_details")}
        </summary>
        <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3">
          <dt>{t("sprint_301.auto_route_recovery.error_code")}</dt>
          <dd className="min-w-0 break-all font-mono text-[var(--foreground)]">{code}</dd>
          <dt>{t("sprint_301.auto_route_recovery.stopped_at")}</dt>
          <dd className="min-w-0 break-all font-mono text-[var(--foreground)]">{boundary}</dd>
        </dl>
      </details>
    </section>
  );
}
