"use client";

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import type { DegradedModeStatus } from "../homeAgents";
import { LicenseMarkdown } from "./LicenseMarkdown";

type LocalModelOption = {
  id: string;
  name: string;
  compatibility: "ready" | "unsupported" | "invalid" | "asset_missing";
};

type LocalInferenceRecoveryResponse = {
  modelId: string;
  modelName: string;
  degradedMode: DegradedModeStatus;
};

const DEGRADED_SUBSYSTEM_LABELS: Record<string, string> = {
  agent: "degraded.subsystems.agents",
  artifactPipeline: "degraded.subsystems.documents",
  autoRouteClassifier: "chat.route.needs_attention",
  audit: "degraded.subsystems.audit",
  backgroundHooks: "degraded.subsystems.background",
  chatSessionPersistence: "degraded.subsystems.saved_work",
  gateway: "degraded.subsystems.channels",
  identity: "degraded.subsystems.identity",
  inference: "degraded.subsystems.local_model",
  knowledge: "degraded.subsystems.knowledge",
  mcpRuntime: "degraded.subsystems.tools",
  memory: "degraded.subsystems.memory",
  taskFlow: "degraded.subsystems.tasks",
  workflowScheduler: "degraded.subsystems.routines",
};

const DEGRADED_SUBSYSTEM_IMPACT_KEYS: Record<string, string> = {
  agent: "degraded.impacts.agents",
  artifactPipeline: "degraded.impacts.documents",
  autoRouteClassifier: "chat.auto_route_attention.attention_content",
  audit: "degraded.impacts.audit",
  backgroundHooks: "degraded.impacts.background",
  chatSessionPersistence: "degraded.impacts.saved_work",
  gateway: "degraded.impacts.channels",
  identity: "degraded.impacts.identity",
  inference: "degraded.impacts.local_model",
  knowledge: "degraded.impacts.knowledge",
  mcpRuntime: "degraded.impacts.tools",
  memory: "degraded.impacts.memory",
  taskFlow: "degraded.impacts.tasks",
  workflowScheduler: "degraded.impacts.routines",
};

export function DegradedModeLanding({
  status,
  onContinue,
  onOpenSettings,
  onStatusChange,
}: {
  status: DegradedModeStatus;
  onContinue: () => void;
  onOpenSettings: () => void;
  onStatusChange?: (status: DegradedModeStatus) => void;
}) {
  const { t } = useI18n();
  const tRef = useRef(t);
  const [isBrowsing, setIsBrowsing] = useState(false);
  const [setupMessage, setSetupMessage] = useState<string | null>(null);
  const [showDetails, setShowDetails] = useState(false);
  const [isRecovering, setIsRecovering] = useState(false);
  const [isRecoveringInference, setIsRecoveringInference] = useState(false);
  const [isLoadingLocalModels, setIsLoadingLocalModels] = useState(false);
  const [readyLocalModels, setReadyLocalModels] = useState<LocalModelOption[]>([]);
  const [selectedLocalModelId, setSelectedLocalModelId] = useState("");
  const [needsRecoveryConfirmation, setNeedsRecoveryConfirmation] = useState(false);
  const [recoveryVerified, setRecoveryVerified] = useState(false);

  useEffect(() => {
    tRef.current = t;
  }, [t]);

  const displayReason = status.reason?.trim() || t("degraded.fallback_error");
  const activeSubsystems = status.subsystems.filter((subsystem) => subsystem.active);
  const inferenceUnavailable = activeSubsystems.some(
    (subsystem) => subsystem.subsystem === "inference",
  );

  const refreshReadyLocalModels = useCallback(async (isActive: () => boolean = () => true) => {
    setIsLoadingLocalModels(true);
    try {
      const models = await invoke<LocalModelOption[]>("list_local_models");
      if (!isActive()) return;
      const readyModels = models.filter((model) => model.compatibility === "ready");
      setReadyLocalModels(readyModels);
      setSelectedLocalModelId((current) =>
        readyModels.some((model) => model.id === current)
          ? current
          : readyModels[0]?.id ?? "",
      );
    } catch {
      if (!isActive()) return;
      setReadyLocalModels([]);
      setSelectedLocalModelId("");
      setSetupMessage(tRef.current("degraded.model_list_error"));
    } finally {
      if (isActive()) setIsLoadingLocalModels(false);
    }
  }, []);

  useEffect(() => {
    if (!inferenceUnavailable) return;
    let active = true;
    const timeout = window.setTimeout(() => {
      void refreshReadyLocalModels(() => active);
    }, 0);
    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [inferenceUnavailable, refreshReadyLocalModels]);

  useEffect(() => {
    if (!status.hasVolatileStorage) return;
    let cancelled = false;
    invoke<{
      cleanupEligible: boolean;
      requiresConfirmation: boolean;
      lastResult: string | null;
    } | null>(
      "get_persistence_recovery_status",
    )
      .then((recovery) => {
        if (cancelled || !recovery) return;
        setRecoveryVerified(recovery.cleanupEligible);
        setNeedsRecoveryConfirmation(recovery.requiresConfirmation);
        if (recovery.lastResult) {
          setSetupMessage(
            recovery.cleanupEligible
              ? tRef.current("degraded.recovery_ready")
              : tRef.current("degraded.recovery_check_needed"),
          );
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [status.hasVolatileStorage]);

  const chooseModelFolder = async () => {
    setIsBrowsing(true);
    setSetupMessage(null);

    try {
      const setting = await invoke<{ path: string; isDefault: boolean } | null>(
        "choose_local_model_directory",
      );
      if (setting) {
        setSetupMessage(t("degraded.folder_set", { path: setting.path }));
        await refreshReadyLocalModels();
      } else {
        setSetupMessage(t("degraded.no_folder"));
      }
    } catch {
      setSetupMessage(t("degraded.picker_error"));
    } finally {
      setIsBrowsing(false);
    }
  };

  const recoverLocalInference = async () => {
    if (!selectedLocalModelId) return;
    setIsRecoveringInference(true);
    setSetupMessage(null);
    try {
      const recovered = await invoke<LocalInferenceRecoveryResponse>(
        "recover_local_inference",
        {
          modelId: selectedLocalModelId,
          model_id: selectedLocalModelId,
        },
      );
      setSetupMessage(
        t("degraded.model_recovered", {
          model: recovered.modelName || recovered.modelId,
        }),
      );
      onStatusChange?.(recovered.degradedMode);
    } catch {
      setSetupMessage(t("degraded.model_recovery_error"));
    } finally {
      setIsRecoveringInference(false);
    }
  };

  const reconcilePersistence = async (confirmOverwrite: boolean) => {
    setIsRecovering(true);
    setSetupMessage(null);
    try {
      const report = await invoke<{
        recoveredRecords: number;
        skippedRecords: number;
        conflictingRecords: number;
        failedRecords: number;
        requiresConfirmation: boolean;
      }>("reconcile_volatile_persistence", { confirmOverwrite });
      setNeedsRecoveryConfirmation(report.requiresConfirmation);
      setRecoveryVerified(!report.requiresConfirmation && report.failedRecords === 0);
      setSetupMessage(
        report.requiresConfirmation
          ? t("degraded.recovery_conflict", { count: report.conflictingRecords })
          : t("degraded.recovery_complete", { count: report.recoveredRecords }),
      );
    } catch {
      setSetupMessage(t("degraded.recovery_error"));
    } finally {
      setIsRecovering(false);
    }
  };

  const exportRecoverySession = async () => {
    setIsRecovering(true);
    try {
      const destination = await invoke<string | null>("choose_volatile_persistence_export");
      if (destination) setSetupMessage(t("degraded.recovery_exported", { path: destination }));
    } catch {
      setSetupMessage(t("degraded.recovery_export_error"));
    } finally {
      setIsRecovering(false);
    }
  };

  const cleanupRecoverySession = async () => {
    setIsRecovering(true);
    try {
      await invoke("cleanup_reconciled_volatile_persistence");
      setSetupMessage(t("degraded.recovery_cleaned"));
      // Cleanup is the terminal recovery action. Refresh the authoritative
      // native status now so the repaired app replaces this launch surface
      // immediately; navigation must not keep rendering a stale recovery gate
      // until the background health poll happens to run.
      try {
        const refreshedStatus = await invoke<DegradedModeStatus>(
          "get_degraded_mode_status",
        );
        onStatusChange?.(refreshedStatus);
      } catch {
        // Cleanup already succeeded. Keep the calm success state and let the
        // existing health poll refresh it instead of misreporting a failure.
      }
    } catch {
      setSetupMessage(t("degraded.recovery_cleanup_error"));
    } finally {
      setIsRecovering(false);
    }
  };

  return (
    <section className="flex h-full min-h-0 w-full overflow-y-auto bg-[var(--background)] text-[var(--foreground)]">
      <div className="mx-auto flex w-full max-w-xl flex-col gap-6 px-6 py-16">
        <div className="flex flex-col gap-3">
          <span className="w-fit rounded-[var(--radius-sm)] bg-[var(--accent-background)] px-3 py-1 text-xs font-semibold text-[var(--foreground-muted)]">
            {t("degraded.badge")}
          </span>
          <h1 className="text-3xl font-semibold leading-tight tracking-tight text-[var(--foreground)]">
            {t("degraded.title")}
          </h1>
          <p className="text-base leading-7 text-[var(--foreground-muted)]">
            {t("degraded.description")}
          </p>
        </div>

        {status.hasVolatileStorage && (
          <div
            aria-live="assertive"
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--accent-background)] p-4"
            role="alert"
          >
            <p className="text-sm font-semibold text-[var(--foreground)]">
              {t("degraded.volatile_title")}
            </p>
            <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
              {t("degraded.volatile_description")}
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
                disabled={isRecovering}
                onClick={() => void reconcilePersistence(false)}
                type="button"
              >
                {isRecovering ? t("degraded.recovery_working") : t("degraded.recovery_probe")}
              </button>
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-xs font-semibold text-[var(--foreground)] disabled:opacity-50"
                disabled={isRecovering}
                onClick={() => void exportRecoverySession()}
                type="button"
              >
                {t("degraded.recovery_export")}
              </button>
              {needsRecoveryConfirmation && (
                <button
                  className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-xs font-semibold text-[var(--foreground)] disabled:opacity-50"
                  disabled={isRecovering}
                  onClick={() => void reconcilePersistence(true)}
                  type="button"
                >
                  {t("degraded.recovery_confirm_overwrite")}
                </button>
              )}
              {recoveryVerified && (
                <button
                  className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-xs font-semibold text-[var(--foreground)] disabled:opacity-50"
                  disabled={isRecovering}
                  onClick={() => void cleanupRecoverySession()}
                  type="button"
                >
                  {t("degraded.recovery_cleanup")}
                </button>
              )}
            </div>
          </div>
        )}

        {activeSubsystems.length > 0 && (
          <ul className="flex flex-col gap-2" aria-label={t("degraded.affected_subsystems")}>
            {activeSubsystems.map((subsystem) => (
              <li
                className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-3"
                key={subsystem.subsystem}
              >
                <p className="text-sm font-semibold text-[var(--foreground)]">
                  {t(
                    DEGRADED_SUBSYSTEM_LABELS[subsystem.subsystem] ??
                      "degraded.subsystems.other",
                  )}
                </p>
                <p className="mt-1 text-sm leading-5 text-[var(--foreground-muted)]">
                  {t(
                    DEGRADED_SUBSYSTEM_IMPACT_KEYS[subsystem.subsystem] ??
                      "degraded.impacts.other",
                  )}
                </p>
              </li>
            ))}
          </ul>
        )}

        {inferenceUnavailable && (
          <label className="flex flex-col gap-2" htmlFor="degraded-local-model">
            <span className="text-sm font-semibold text-[var(--foreground)]">
              {t("degraded.model_label")}
            </span>
            <select
              className="h-11 w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-sm text-[var(--foreground)] outline-none disabled:cursor-wait disabled:opacity-60"
              disabled={isLoadingLocalModels || isRecoveringInference || readyLocalModels.length === 0}
              id="degraded-local-model"
              onChange={(event) => setSelectedLocalModelId(event.target.value)}
              value={selectedLocalModelId}
            >
              {readyLocalModels.length > 0 ? (
                readyLocalModels.map((model) => (
                  <option key={model.id} value={model.id}>
                    {model.name}
                  </option>
                ))
              ) : (
                <option value="">
                  {isLoadingLocalModels
                    ? t("degraded.model_loading")
                    : t("degraded.model_empty")}
                </option>
              )}
            </select>
          </label>
        )}

        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <button
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-5 py-2.5 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
            onClick={onContinue}
            type="button"
          >
            {t("setup.continue")}
          </button>
          {inferenceUnavailable && (
            <>
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-5 py-2.5 text-sm font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
                disabled={isBrowsing || isRecoveringInference}
                onClick={chooseModelFolder}
                type="button"
              >
                {isBrowsing ? t("degraded.choosing") : t("degraded.choose_folder")}
              </button>
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-5 py-2.5 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
                disabled={
                  isLoadingLocalModels ||
                  isRecoveringInference ||
                  !selectedLocalModelId
                }
                onClick={() => void recoverLocalInference()}
                type="button"
              >
                {isRecoveringInference
                  ? t("degraded.model_recovering")
                  : t("degraded.model_recover")}
              </button>
            </>
          )}
          <button
            className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-5 py-2.5 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
            onClick={onOpenSettings}
            type="button"
          >
            {t("degraded.open_settings")}
          </button>
        </div>

        {setupMessage && (
          <p
            aria-live="polite"
            className="text-sm leading-6 text-[var(--foreground-muted)]"
          >
            {setupMessage}
          </p>
        )}

        <div className="border-t border-[var(--border-soft)] pt-4">
          <button
            aria-expanded={showDetails}
            className="text-sm font-medium text-[var(--foreground-muted)] transition-colors hover:text-[var(--foreground)]"
            onClick={() => setShowDetails((value) => !value)}
            type="button"
          >
            {showDetails ? t("degraded.hide_details") : t("degraded.show_details")}
          </button>
          {showDetails && (
            <p className="mt-3 whitespace-pre-wrap rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 font-mono text-xs leading-5 text-[var(--foreground)]">
              {displayReason}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

export function LicenseAgreementGate({
  error,
  isAccepting = false,
  onAccept,
  onDecline,
  settings,
}: {
  error?: string;
  isAccepting?: boolean;
  onAccept: () => void;
  onDecline: () => void;
  settings: PrivacySettingsState;
}) {
  const { t } = useI18n();
  const acceptButtonRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLElement>(null);

  useEffect(() => {
    acceptButtonRef.current?.focus();
  }, []);

  const trapFocus = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key !== "Tab") return;
    const controls = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    );
    const first = controls[0];
    const last = controls.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--background)] px-4 py-6">
      <section
        aria-describedby="license-description"
        aria-labelledby="license-gate-title"
        aria-modal="true"
        className="flex max-h-full w-full max-w-3xl flex-col gap-4 overflow-y-auto rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5 shadow-[0_24px_60px_rgba(15,23,42,0.24)]"
        onKeyDown={trapFocus}
        ref={dialogRef}
        role="dialog"
      >
        <div className="flex flex-col gap-2">
          <span className="w-fit rounded-[var(--radius-sm)] bg-[var(--accent-background)] px-3 py-1 text-xs font-semibold text-[var(--foreground-muted)]">
            {t("license.badge")}
          </span>
          <h1
            className="text-2xl font-semibold leading-tight tracking-tight text-[var(--foreground)]"
            id="license-gate-title"
          >
            {t("license.title")}
          </h1>
          <p
            className="text-sm font-medium leading-6 text-[var(--foreground)]"
            id="license-description"
          >
            {t("license.body")}
          </p>
          <p className="text-xs leading-5 text-[var(--foreground-muted)]">
            {t("license.version_effective", {
              version: settings.licenseVersion,
              date: settings.licenseEffectiveDate,
            })}
          </p>
        </div>
        <article
          aria-label={t("license.full_license_label")}
          className="min-h-32 max-h-[48vh] shrink-0 overflow-y-auto rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-5 py-4 text-[var(--foreground)]"
          tabIndex={0}
        >
          <LicenseMarkdown text={settings.licenseText} />
        </article>
        {error && (
          <p
            aria-live="assertive"
            role="alert"
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--accent-background)] px-3 py-2 text-sm leading-5 text-[var(--foreground)]"
          >
            {error}
          </p>
        )}
        <div className="flex flex-col gap-2 sm:flex-row sm:justify-end">
          <button
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
            disabled={isAccepting}
            onClick={onDecline}
            type="button"
          >
            {t("license.decline")}
          </button>
          <button
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-1.5 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-60"
            disabled={isAccepting}
            onClick={onAccept}
            ref={acceptButtonRef}
            type="button"
          >
            {isAccepting ? t("license.accepting") : t("license.accept")}
          </button>
        </div>
      </section>
    </div>
  );
}

interface ScreenHeaderProps {
  title: string;
  showBorder?: boolean;
  children?: ReactNode;
  className?: string;
}

export function ScreenHeader({
  title,
  showBorder = false,
  children,
  className = "",
}: ScreenHeaderProps) {
  return (
    <div
      className={`flex flex-col gap-3 pb-4 lg:flex-row lg:items-end lg:justify-between ${
        showBorder ? "border-b border-[var(--border-strong)]" : ""
      } ${className}`}
    >
      <div>
        <h2 className="text-xl font-semibold tracking-tight text-[var(--foreground)]">
          {title}
        </h2>
      </div>
      {children && (
        <div className="flex w-full flex-col gap-2 sm:flex-row sm:justify-end lg:w-auto shrink-0">
          {children}
        </div>
      )}
    </div>
  );
}

export function Panel({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5 sm:p-6">
      <ScreenHeader title={title} />
      {description && (
        <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
          {description}
        </p>
      )}
      <div className={description ? "mt-5" : "mt-4"}>{children}</div>
    </section>
  );
}
