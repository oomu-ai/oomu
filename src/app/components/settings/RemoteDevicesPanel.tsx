"use client";

import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";

type RemoteDevice = {
  remoteDeviceId: string;
  label: string;
  allowedProjectIds: string[];
  scopes: string[];
  pairedAtMs: number;
  expiresAtMs: number;
  lastUsedAtMs: number | null;
  revokedAtMs: number | null;
};

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

const secondaryButtonClass =
  "inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-sm font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50";

const destructiveButtonClass =
  "inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--destructive)] bg-[var(--background)] px-3 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-not-allowed disabled:opacity-50";

function deviceAbility(scopes: string[], t: TranslateFn) {
  if (scopes.includes("create_task") && scopes.includes("view_task")) {
    return t("remote_devices.abilities.start_and_check");
  }
  if (scopes.includes("view_task")) {
    return t("remote_devices.abilities.check");
  }
  return t("remote_devices.abilities.limited");
}

export function RemoteDevicesPanel() {
  const { t } = useI18n();
  const [devices, setDevices] = useState<RemoteDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyDeviceId, setBusyDeviceId] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [refreshState, setRefreshState] = useState<
    "idle" | "working" | "success" | "error"
  >("idle");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<RemoteDevice | null>(null);
  const removalTriggerRef = useRef<HTMLButtonElement | null>(null);
  const activeDevices = useMemo(
    () => devices.filter((device) => !device.revokedAtMs),
    [devices],
  );

  const load = useCallback(
    async (showLoading: boolean) => {
      if (showLoading) setLoading(true);
      try {
        setDevices(await invoke<RemoteDevice[]>("list_remote_devices"));
        setError("");
        return true;
      } catch {
        setError(t("remote_devices.errors.load"));
        return false;
      } finally {
        if (showLoading) setLoading(false);
      }
    },
    [t],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => void load(true), 0);
    return () => window.clearTimeout(timer);
  }, [load]);

  async function refreshDevices() {
    setRefreshing(true);
    setRefreshState("working");
    setMessage("");
    setError("");
    try {
      if (await load(false)) {
        setMessage(t("remote_devices.refreshed"));
        setRefreshState("success");
      } else {
        setRefreshState("error");
      }
    } finally {
      setRefreshing(false);
    }
  }

  async function removeDevice(device: RemoteDevice) {
    setBusyDeviceId(device.remoteDeviceId);
    setMessage("");
    setError("");
    try {
      await invoke("revoke_remote_device", {
        request: { remoteDeviceId: device.remoteDeviceId },
      });
      setDevices((current) =>
        current.filter((candidate) => candidate.remoteDeviceId !== device.remoteDeviceId),
      );
      setPendingRemoval(null);
      setMessage(t("remote_devices.removed", { name: device.label }));
    } catch {
      setError(t("remote_devices.errors.remove"));
    } finally {
      setBusyDeviceId("");
    }
  }

  function closeRemovalDialog() {
    if (busyDeviceId) return;
    setPendingRemoval(null);
    window.setTimeout(() => removalTriggerRef.current?.focus(), 0);
  }

  return (
    <section className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold text-[var(--foreground)]">
            {t("remote_devices.title")}
          </h2>
          <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
            {t("remote_devices.subtitle")}
          </p>
        </div>
        <button
          aria-busy={refreshing}
          className={secondaryButtonClass}
          data-action-state={refreshState}
          disabled={refreshing || loading || Boolean(busyDeviceId)}
          onClick={() => void refreshDevices()}
          type="button"
        >
          {refreshing ? t("common.refreshing") : t("common.refresh")}
        </button>
      </header>

      <div className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-5">
        <h3 className="font-semibold text-[var(--foreground)]">
          {t("remote_devices.unavailable_title")}
        </h3>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--foreground-muted)]">
          {t("remote_devices.unavailable_help")}
        </p>
      </div>

      {message ? (
        <p
          aria-live="polite"
          className="rounded-[var(--radius-sm)] border border-[var(--success)]/30 bg-[var(--success-background)] px-4 py-3 text-sm text-[var(--success)]"
          role="status"
        >
          {message}
        </p>
      ) : null}
      {error && !pendingRemoval ? (
        <p
          className="rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-4 py-3 text-sm text-[var(--destructive)]"
          role="alert"
        >
          {error}
        </p>
      ) : null}

      <div>
        <h3 className="font-semibold">{t("remote_devices.connected")}</h3>
        {loading ? (
          <p className="mt-2 text-sm text-[var(--foreground-muted)]">
            {t("common.loading")}
          </p>
        ) : activeDevices.length ? (
          <div className="mt-3 divide-y divide-[var(--border-soft)] rounded-[var(--radius-md)] border border-[var(--border-soft)]">
            {activeDevices.map((device) => {
              const removing = busyDeviceId === device.remoteDeviceId;
              return (
                <article
                  className="flex items-center justify-between gap-4 p-4"
                  key={device.remoteDeviceId}
                >
                  <div>
                    <p className="text-sm font-semibold">{device.label}</p>
                    <p className="mt-1 text-xs text-[var(--foreground-muted)]">
                      {device.lastUsedAtMs
                        ? t("remote_devices.last_used", {
                            time: new Date(device.lastUsedAtMs).toLocaleString(),
                          })
                        : t("remote_devices.not_used")} {" · "}
                      {deviceAbility(device.scopes, t)}
                    </p>
                  </div>
                  <button
                    aria-busy={removing}
                    className={destructiveButtonClass}
                    data-action-state={removing ? "working" : "idle"}
                    disabled={Boolean(busyDeviceId) || refreshing}
                    onClick={(event) => {
                      removalTriggerRef.current = event.currentTarget;
                      setMessage("");
                      setError("");
                      setPendingRemoval(device);
                    }}
                    type="button"
                  >
                    {t("remote_devices.remove")}
                  </button>
                </article>
              );
            })}
          </div>
        ) : (
          <p className="mt-2 text-sm text-[var(--foreground-muted)]">
            {t("remote_devices.empty")}
          </p>
        )}
      </div>
      {pendingRemoval ? (
        <RemoveRemoteDeviceDialog
          busy={busyDeviceId === pendingRemoval.remoteDeviceId}
          error={error}
          name={pendingRemoval.label}
          onCancel={closeRemovalDialog}
          onConfirm={() => void removeDevice(pendingRemoval)}
          t={t}
        />
      ) : null}
    </section>
  );
}

function RemoveRemoteDeviceDialog({
  busy,
  error,
  name,
  onCancel,
  onConfirm,
  t,
}: {
  busy: boolean;
  error: string;
  name: string;
  onCancel: () => void;
  onConfirm: () => void;
  t: TranslateFn;
}) {
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (busy) dialogRef.current?.focus();
    else cancelRef.current?.focus();
  }, [busy]);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const buttons = Array.from(
      dialogRef.current?.querySelectorAll<HTMLButtonElement>(
        "button:not(:disabled)",
      ) ?? [],
    );
    if (buttons.length === 0) {
      event.preventDefault();
      dialogRef.current?.focus();
      return;
    }
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6">
      <div
        aria-busy={busy}
        aria-describedby={descriptionId}
        aria-labelledby={titleId}
        aria-modal="true"
        className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-6 shadow-[var(--shadow-raised)]"
        onKeyDown={handleKeyDown}
        ref={dialogRef}
        role="dialog"
        tabIndex={-1}
      >
        <h3 className="text-lg font-semibold" id={titleId}>
          {t("remote_devices.remove_title", { name })}
        </h3>
        <p
          className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]"
          id={descriptionId}
        >
          {t("remote_devices.remove_help")}
        </p>
        {error ? (
          <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">
            {error}
          </p>
        ) : null}
        <div className="mt-6 flex justify-end gap-2">
          <button
            className={secondaryButtonClass}
            disabled={busy}
            onClick={onCancel}
            ref={cancelRef}
            type="button"
          >
            {t("common.cancel")}
          </button>
          <button
            aria-busy={busy}
            className="inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--destructive)] px-3 text-sm font-semibold text-white transition-opacity hover:opacity-90 disabled:cursor-wait disabled:opacity-50"
            data-action-state={busy ? "working" : "idle"}
            disabled={busy}
            onClick={onConfirm}
            type="button"
          >
            {busy ? t("remote_devices.removing") : t("remote_devices.remove_action")}
          </button>
        </div>
      </div>
    </div>
  );
}
