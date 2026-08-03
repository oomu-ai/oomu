"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { publishMacPermissionRefresh } from "../chat/macPermissionRefreshSignal";

type PermissionState =
  | "not_requested"
  | "allowed"
  | "limited"
  | "denied"
  | "restricted"
  | "requires_settings"
  | "stale"
  | "when_used"
  | "unsupported";

export type MacPermissionStatus = {
  capabilityId: string;
  state: PermissionState;
  canRequest: boolean;
  settingsPane?: string | null;
  checkedAtMs: number;
};

export type PermissionAction = "request" | "settings" | null;

const CAPABILITY_GROUPS = [
  ["control", ["accessibility", "screen_control", "screen_capture"]],
  ["audio_video", ["microphone", "camera", "speech_recognition", "music"]],
  ["personal", ["photos", "contacts", "calendar", "reminders"]],
  ["apple_apps", ["mail", "notes", "messages", "finder", "system_events"]],
  ["system", ["files_and_folders", "full_disk_access", "local_network", "notifications"]],
] as const;

const KNOWN_CAPABILITIES: ReadonlySet<string> = new Set(
  CAPABILITY_GROUPS.flatMap(([, capabilities]) => [...capabilities]),
);

export function permissionAction(status: MacPermissionStatus): PermissionAction {
  if (status.state === "allowed" || status.state === "when_used" || status.state === "unsupported" || status.state === "restricted") {
    return null;
  }
  if (status.canRequest && status.state === "not_requested") return "request";
  return status.settingsPane ? "settings" : status.canRequest ? "request" : null;
}

function statusTone(state: PermissionState) {
  if (state === "allowed") return "text-[var(--success)]";
  if (state === "denied" || state === "restricted") return "text-[var(--destructive)]";
  return "text-[var(--warning)]";
}

type Translate = (key: string, values?: Record<string, string | number>) => string;

function PermissionGroups({
  activeCapability,
  byCapability,
  performAction,
  successCapability,
  t,
}: {
  activeCapability: string;
  byCapability: Map<string, MacPermissionStatus>;
  performAction: (
    status: MacPermissionStatus,
    action: Exclude<PermissionAction, null>,
  ) => Promise<void>;
  successCapability: string;
  t: Translate;
}) {
  return CAPABILITY_GROUPS.map(([group, capabilities]) => {
    const statuses = capabilities
      .map((capability) => byCapability.get(capability))
      .filter((status): status is MacPermissionStatus => Boolean(status));
    if (statuses.length === 0) return null;
    return (
      <section className="overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)]" key={group}>
        <h3 className="border-b border-[var(--border-soft)] px-5 py-3 text-sm font-semibold text-[var(--foreground)]">
          {t(`sprint_299.permissions.groups.${group}`)}
        </h3>
        <div className="divide-y divide-[var(--border-soft)]">
          {statuses.map((status) => {
            const action = permissionAction(status);
            const isWorking = activeCapability === status.capabilityId;
            return (
              <div className="flex flex-col gap-3 px-5 py-4 sm:flex-row sm:items-center sm:justify-between" key={status.capabilityId}>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <h4 className="text-sm font-semibold text-[var(--foreground)]">
                      {t(`sprint_299.permissions.capabilities.${status.capabilityId}.name`)}
                    </h4>
                    <span className={`text-xs font-semibold ${statusTone(status.state)}`}>
                      {t(`sprint_299.permissions.states.${status.state}`)}
                    </span>
                  </div>
                  <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
                    {t(`sprint_299.permissions.purposes.${status.capabilityId}`)}
                  </p>
                  {successCapability === status.capabilityId ? (
                    <p className="mt-1 text-xs font-medium text-[var(--success)]" role="status">
                      {t("sprint_299.permissions.allowed_success")}
                    </p>
                  ) : null}
                </div>
                {action ? (
                  <button
                    className="min-h-9 shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-semibold text-[var(--foreground)] hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
                    disabled={Boolean(activeCapability)}
                    onClick={() => void performAction(status, action)}
                    type="button"
                  >
                    {isWorking
                      ? t("sprint_299.permissions.working")
                      : t(action === "request" ? "sprint_299.permissions.allow" : "sprint_299.permissions.open_settings")}
                  </button>
                ) : null}
              </div>
            );
          })}
        </div>
      </section>
    );
  });
}

export function MacPermissionsPanel() {
  const { t } = useI18n();
  const [statuses, setStatuses] = useState<MacPermissionStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [activeCapability, setActiveCapability] = useState("");
  const [error, setError] = useState("");
  const [successCapability, setSuccessCapability] = useState("");
  const pendingSettingsCapabilityRef = useRef("");

  const refresh = useCallback(async () => {
    try {
      const response = await invoke<MacPermissionStatus[]>("list_macos_permission_states");
      setStatuses(response.filter((entry) => KNOWN_CAPABILITIES.has(entry.capabilityId)));
      setError("");
    } catch {
      setError(t("sprint_299.permissions.errors.load"));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    const initialRefresh = window.setTimeout(() => void refresh(), 0);
    const handleFocus = () => void refresh().then(() => {
      const capabilityId = pendingSettingsCapabilityRef.current;
      if (!capabilityId) return;
      pendingSettingsCapabilityRef.current = "";
      publishMacPermissionRefresh(capabilityId);
    });
    window.addEventListener("focus", handleFocus);
    return () => {
      window.clearTimeout(initialRefresh);
      window.removeEventListener("focus", handleFocus);
    };
  }, [refresh]);

  const byCapability = useMemo(
    () => new Map(statuses.map((status) => [status.capabilityId, status])),
    [statuses],
  );

  const performAction = async (status: MacPermissionStatus, action: Exclude<PermissionAction, null>) => {
    setActiveCapability(status.capabilityId);
    setSuccessCapability("");
    setError("");
    try {
      if (action === "request") {
        const updated = await invoke<MacPermissionStatus>("request_macos_permission", {
          request: { capabilityId: status.capabilityId },
        });
        setStatuses((current) => current.map((entry) =>
          entry.capabilityId === updated.capabilityId ? updated : entry));
        if (updated.state === "allowed" || updated.state === "limited") {
          setSuccessCapability(updated.capabilityId);
        }
      } else {
        pendingSettingsCapabilityRef.current = status.capabilityId;
        await invoke("open_macos_permission_settings", {
          request: { capabilityId: status.capabilityId },
        });
      }
      if (action === "request") {
        await refresh();
        publishMacPermissionRefresh(status.capabilityId);
      }
    } catch {
      if (action === "settings") pendingSettingsCapabilityRef.current = "";
      setError(t("sprint_299.permissions.errors.action"));
    } finally {
      setActiveCapability("");
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-base font-semibold text-[var(--foreground)]">
          {t("sprint_299.permissions.title")}
        </h2>
        <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--foreground-muted)]">
          {t("sprint_299.permissions.description")}
        </p>
      </header>

      {isLoading ? (
        <p className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5 text-sm text-[var(--foreground-muted)]">
          {t("sprint_299.permissions.loading")}
        </p>
      ) : null}

      {!isLoading && statuses.length === 0 ? (
        <p className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5 text-sm text-[var(--foreground-muted)]">
          {t("sprint_299.permissions.not_available")}
        </p>
      ) : null}

      <PermissionGroups
        activeCapability={activeCapability}
        byCapability={byCapability}
        performAction={performAction}
        successCapability={successCapability}
        t={t}
      />

      {error ? (
        <p className="rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-xs font-medium text-[var(--destructive)]" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
