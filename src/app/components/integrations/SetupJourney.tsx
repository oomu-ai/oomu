"use client";

import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import {
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  integrationApi,
  type ConnectorAccount,
  type ConnectorManifest,
  type SetupState,
} from "./integrationClient";
import { localizedOAuthFailure, oauthConnectionOutcome } from "./connectorOAuthStatus";
import { ConnectorAvailabilityNotice } from "./ConnectorAvailabilityNotice";
import { persistLocalSetupSelection } from "./localModelSetup";
import { SetupModelStep } from "./SetupModelStep";
import { useRecommendedSetupCompletion } from "./useRecommendedSetupCompletion";
import { useRecommendedModelSettingsRoute } from "./recommendedModelSettingsRoute";
import { initialSetupStepIndex, setupSteps } from "./setupGate";

type Provider = ConfiguredProvider;

type SetupJourneyProps = {
  initialState: SetupState;
  onComplete: (state: SetupState) => void;
  onProviderConfigured?: (provider: ConfiguredProvider) => void;
  previewMode?: boolean;
};

type LocalModel = {
  id: string;
  compatibility: string;
};

async function loadLocalModels(): Promise<LocalModel[]> {
  const models = await invoke<LocalModel[]>("list_local_models");
  return Array.isArray(models) ? models : [];
}

const connectorCopy = {
  apple_apps: {
    name: "setup.connector_apple_name",
    permissions: "setup.connector_apple_permissions",
  },
  google_workspace: {
    name: "setup.connector_google_name",
    permissions: "setup.connector_google_permissions",
  },
  microsoft_365: {
    name: "setup.connector_microsoft_name",
    permissions: "setup.connector_microsoft_permissions",
  },
  slack: {
    name: "setup.connector_slack_name",
    permissions: "setup.connector_slack_permissions",
  },
} as const;

const setupErrorCodes = new Set([
  "setup_notification_permission_request_failed",
  "setup_notification_permission_check_failed",
  "setup_notification_permission_denied",
  "setup_storage_recovery_required",
  "setup_sample_already_running",
  "setup_model_execution_failed",
  "setup_model_output_empty",
  "setup_provider_not_found",
  "setup_provider_credentials_missing",
  "setup_provider_model_missing",
  "setup_project_policy_denied",
  "setup_finalization_failed",
  "setup_internal_error",
]);

function localizedError(
  cause: unknown,
  fallbackCode: string,
  t: (key: string) => string,
): string {
  const code =
    cause && typeof cause === "object" && "code" in cause
      ? String((cause as { code?: unknown }).code ?? "")
      : "";
  return t(`setup.errors.${setupErrorCodes.has(code) ? code : fallbackCode}`);
}

function isConnected(account: ConnectorAccount | undefined): boolean {
  return account?.connectionState === "authorized" || account?.connectionState === "reachable";
}

function selectableCloudProviders(providers: Provider[]): Provider[] {
  return providers.filter((item) => item.authMethod === "api_key" && item.credentialConfigured);
}

async function saveSetupStep(
  currentStep: string,
  modelRoute: string,
  previewMode: boolean,
): Promise<void> {
  if (!previewMode) {
    await integrationApi.saveSetup(currentStep, modelRoute, "local");
  }
}

function runSetupSample(modelRoute: string, previewMode: boolean): Promise<SetupState> {
  return integrationApi.runSample(
    modelRoute,
    previewMode ? { completeSetup: false } : undefined,
  );
}

function finishSetup(
  initialState: SetupState,
  modelRoute: string,
  previewMode: boolean,
): Promise<SetupState> {
  return previewMode
    ? Promise.resolve({ ...initialState, currentStep: "finished" })
    : integrationApi.saveSetup("finished", modelRoute, "local");
}

function setupPermissions(modelRoute: string, t: (key: string) => string) {
  return modelRoute === "local"
    ? [t("setup.permission_files"), t("setup.permission_keychain")]
    : [t("setup.permission_keychain"), t("setup.permission_network")];
}

function SetupSampleStep({
  busy,
  modelInventoryLoaded,
  onRunSample,
  onSkipSample,
  sampleAlreadyVerified,
  sampleModelReady,
  t,
}: {
  busy: boolean;
  modelInventoryLoaded: boolean;
  onRunSample: () => void;
  onSkipSample: () => void;
  sampleAlreadyVerified: boolean;
  sampleModelReady: boolean;
  t: (key: string) => string;
}) {
  const runnableSample = sampleModelReady && !sampleAlreadyVerified;
  return (
    <div className="rounded border p-5">
      <h2 className="font-semibold">{t("setup.sample_name")}</h2>
      <p className="mt-2 text-sm text-[var(--foreground-muted)]">
        {t("setup.sample_help")}
      </p>
      {sampleAlreadyVerified ? (
        <p className="mt-4 text-sm font-semibold text-[var(--foreground)]" role="status">
          {t("recommended_model.ready")}
        </p>
      ) : null}
      {!sampleAlreadyVerified && !sampleModelReady && modelInventoryLoaded ? (
        <p className="mt-4 text-sm leading-6 text-[var(--foreground-muted)]">
          {t("setup.sample_model_unavailable")}
        </p>
      ) : null}
      <div className="mt-4 flex flex-wrap items-center gap-3">
        {runnableSample ? (
          <button
            className="rounded bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
            disabled={busy}
            onClick={onRunSample}
            type="button"
          >
            {busy ? t("common.loading") : t("setup.run_sample")}
          </button>
        ) : null}
        <button
          className={`rounded px-4 py-2 text-sm font-semibold disabled:opacity-50 ${
            runnableSample
              ? "border"
              : "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
          }`}
          data-setup-action="skip-sample"
          disabled={busy}
          onClick={onSkipSample}
          type="button"
        >
          {busy
            ? t("common.loading")
            : sampleAlreadyVerified
              ? t("setup.continue")
              : t("setup.skip_sample")}
        </button>
      </div>
    </div>
  );
}

function useConnectorPolling(
  pendingConnectors: Record<string, string>,
  setAccounts: Dispatch<SetStateAction<ConnectorAccount[]>>,
  setPendingConnectors: Dispatch<SetStateAction<Record<string, string>>>,
  setError: Dispatch<SetStateAction<string>>,
  t: (key: string, variables?: Record<string, string | number>) => string,
) {
  useEffect(() => {
    if (Object.keys(pendingConnectors).length === 0) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const connectorIds = Object.values(pendingConnectors);
        const statuses = await Promise.all(connectorIds.map(async (connectorId) => ({
          connectorId,
          status: await integrationApi.connectionStatus(connectorId),
        })));
        if (cancelled) return;
        const completed = statuses.filter(({ status }) =>
          oauthConnectionOutcome(status) !== "pending");
        if (completed.length === 0) return;
        const failed = completed.find(({ status }) =>
          oauthConnectionOutcome(status) === "failed");
        if (failed) setError(localizedOAuthFailure(failed.status, t));
        const completedIds = new Set(completed.map(({ connectorId }) => connectorId));
        setPendingConnectors((current) => Object.fromEntries(
          Object.entries(current).filter(([, connectorId]) => !completedIds.has(connectorId)),
        ));
        const nextAccounts = await integrationApi.accounts();
        if (!cancelled) setAccounts(nextAccounts);
      } catch (cause) {
        if (!cancelled) {
          setError(localizedError(cause, "setup_connector_status_failed", t));
        }
      }
    };
    const timer = window.setInterval(() => void poll(), 1_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [pendingConnectors, setAccounts, setError, setPendingConnectors, t]);
}

export function SetupJourney({
  initialState, onComplete, onProviderConfigured,
  previewMode = false,
}: SetupJourneyProps) {
  const { t } = useI18n();
  const tRef = useRef(t);
  const [index, setIndex] = useState(() => initialSetupStepIndex(initialState.currentStep));
  useRecommendedModelSettingsRoute(() => setIndex(initialSetupStepIndex("model")));
  const [modelRoute, setModelRoute] = useState(initialState.modelPath || "local");
  const [providers, setProviders] = useState<Provider[]>([]);
  const providerConfigsRef = useRef<Provider[]>([]);
  const [localModels, setLocalModels] = useState<LocalModel[]>([]);
  const [modelInventoryLoaded, setModelInventoryLoaded] = useState(false);
  const modelInventoryRequestRef = useRef(0);
  const [manifests, setManifests] = useState<ConnectorManifest[]>([]);
  const [accounts, setAccounts] = useState<ConnectorAccount[]>([]);
  const [pendingConnectors, setPendingConnectors] = useState<Record<string, string>>({});
  const [busyConnector, setBusyConnector] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const sampleAlreadyVerified = Boolean(initialState.sampleProjectId?.trim());
  const step = setupSteps[index];
  const permissions = setupPermissions(modelRoute, t);

  useEffect(() => {
    tRef.current = t;
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    const modelInventoryRequest = ++modelInventoryRequestRef.current;
    void Promise.all([
      invoke<Provider[]>("list_provider_configs"),
      integrationApi.manifests(),
      integrationApi.accounts(),
      loadLocalModels().catch(() => []),
    ])
      .then(([nextProviders, nextManifests, nextAccounts, nextLocalModels]) => {
        if (cancelled) return;
        providerConfigsRef.current = nextProviders;
        setProviders(selectableCloudProviders(nextProviders));
        setManifests(nextManifests);
        setAccounts(nextAccounts);
        if (modelInventoryRequest === modelInventoryRequestRef.current) {
          setLocalModels(nextLocalModels);
          setModelInventoryLoaded(true);
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          if (modelInventoryRequest === modelInventoryRequestRef.current) {
            setModelInventoryLoaded(true);
          }
          setError(localizedError(cause, "setup_load_failed", tRef.current));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useConnectorPolling(pendingConnectors, setAccounts, setPendingConnectors, setError, t);

  async function advance() {
    setBusy(true);
    setError("");
    try {
      if (step === "model" && modelRoute === "local") {
        const currentLocalModels = await refreshLocalModels();
        const evidence = await persistLocalSetupSelection({
          localModels: currentLocalModels,
          providerConfigs: providerConfigsRef.current,
          providerName: t("models.provider_names.local_model"),
          onProviderConfigured,
        });
        providerConfigsRef.current = evidence.providers;
      }
      await advanceToNextStep();
    } catch (cause) {
      setError(localizedError(cause, "setup_progress_save_failed", t));
    } finally {
      setBusy(false);
    }
  }

  async function advanceToNextStep() {
    const next = Math.min(index + 1, setupSteps.length - 1);
    await saveSetupStep(setupSteps[next], modelRoute, previewMode);
    setIndex(next);
  }

  const recommendedSetup = useRecommendedSetupCompletion({
    advance: advanceToNextStep,
    applyProviders: (currentProviders) => {
      providerConfigsRef.current = currentProviders;
      setProviders(selectableCloudProviders(currentProviders));
    },
    onError: (cause) => {
      setError(localizedError(cause, "setup_progress_save_failed", t));
    },
    onProviderConfigured,
    refreshLocalModels,
    setBusy,
  });

  async function connect(manifestId: string) {
    setBusyConnector(manifestId);
    setError("");
    try {
      const attempt = await integrationApi.connect(manifestId);
      setPendingConnectors((current) => ({
        ...current,
        [manifestId]: attempt.connectorId,
      }));
      setAccounts(await integrationApi.accounts());
    } catch (cause) {
      setError(localizedError(cause, "setup_connector_failed", t));
    } finally {
      setBusyConnector("");
    }
  }

  async function chooseModelFiles() {
    setBusy(true);
    setError("");
    try {
      const setting = await invoke<{ path: string; isDefault: boolean } | null>(
        "choose_local_model_directory",
      );
      if (setting) await refreshLocalModels();
    } catch (cause) {
      setError(localizedError(cause, "setup_model_picker_failed", t));
    } finally {
      setBusy(false);
    }
  }

  async function refreshLocalModels(): Promise<LocalModel[]> {
    const modelInventoryRequest = ++modelInventoryRequestRef.current;
    try {
      const nextLocalModels = await loadLocalModels();
      if (modelInventoryRequest === modelInventoryRequestRef.current) {
        setLocalModels(nextLocalModels);
        setModelInventoryLoaded(true);
      }
      return nextLocalModels;
    } catch (cause) {
      if (modelInventoryRequest === modelInventoryRequestRef.current) {
        setModelInventoryLoaded(true);
      }
      throw cause;
    }
  }

  async function runSample() {
    setBusy(true);
    setError("");
    try {
      const finished = await runSetupSample(modelRoute, previewMode);
      onComplete(finished);
    } catch (cause) {
      setError(localizedError(cause, "setup_sample_failed", t));
    } finally {
      setBusy(false);
    }
  }

  async function skipSample() {
    setBusy(true);
    setError("");
    try {
      const finished = await finishSetup(initialState, modelRoute, previewMode);
      onComplete(finished);
    } catch (cause) {
      setError(localizedError(cause, "setup_finalization_failed", t));
    } finally {
      setBusy(false);
    }
  }

  const connectorManifests = manifests.filter((item) =>
    ["apple_apps", "google_workspace", "microsoft_365", "slack"].includes(item.manifestId),
  );
  const sampleModelReady = modelInventoryLoaded && (modelRoute === "local"
    ? localModels.some((model) => model.compatibility === "ready")
    : Boolean(
        providers.find((provider) => provider.id === modelRoute)?.customModelIds.trim(),
      ));
  return (
    <section
      className="flex h-full min-h-0 w-full items-center justify-center overflow-y-auto bg-[var(--background)] p-6"
      data-setup-journey
      data-setup-step={step}
    >
      <div className="w-full max-w-3xl rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--background)] p-7 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-xs font-semibold text-[var(--foreground-muted)]">
              {t("setup.progress", { current: index + 1, total: setupSteps.length })}
            </p>
            <h1 className="mt-1 text-2xl font-semibold">{t(`setup.${step}_title`)}</h1>
          </div>
          <div className="flex gap-1">
            {setupSteps.map((item, itemIndex) => (
              <span
                className={`h-2 w-8 rounded-full ${
                  itemIndex <= index ? "bg-[var(--foreground)]" : "bg-[var(--border-soft)]"
                }`}
                key={item}
              />
            ))}
          </div>
        </div>
        <p className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]">
          {t(`setup.${step}_help`)}
        </p>

        <div className="mt-7">
          {step === "model" ? (
            <SetupModelStep
              busy={busy}
              localModels={localModels}
              modelRoute={modelRoute}
              onChooseExisting={chooseModelFiles}
              onDefer={recommendedSetup.defer}
              onRouteChange={setModelRoute}
              onUseExisting={advance}
              onVerified={recommendedSetup.accept}
              providers={providers}
            />
          ) : null}

          {step === "permissions" ? (
            <ul className="grid gap-3">
              {permissions.map((permission) => (
                <li className="rounded border p-4 text-sm" key={permission}>
                  {permission}
                </li>
              ))}
              <li className="rounded border p-4 text-sm">
                {t("setup.permission_notifications")}
              </li>
            </ul>
          ) : null}

          {step === "connectors" ? (
            <div className="grid gap-3">
              {connectorManifests.map((item) => {
                const copy = connectorCopy[item.manifestId as keyof typeof connectorCopy];
                if (!copy) return null;
                const connectedAccount = accounts.find(
                  (account) => account.manifestId === item.manifestId && isConnected(account),
                );
                const pendingId = pendingConnectors[item.manifestId];
                const pendingAccount = accounts.find(
                  (account) => account.connectorId === pendingId,
                );
                const waiting = Boolean(
                  pendingId && (!pendingAccount || pendingAccount.connectionState === "configured"),
                );
                return (
                  <div
                    className="flex items-start justify-between gap-4 rounded border p-4"
                    key={item.manifestId}
                  >
                    <div className="min-w-0">
                      <p className="font-semibold">{t(copy.name)}</p>
                      <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
                        {t(copy.permissions)}
                      </p>
                      {!item.supported ? (
                        <ConnectorAvailabilityNotice
                          reasonCode={item.availabilityReasonCode}
                          service={t(copy.name)}
                        />
                      ) : null}
                      {connectedAccount?.accountLabel ? (
                        <p className="mt-2 text-xs font-semibold text-[var(--foreground)]">
                          {connectedAccount.accountLabel}
                        </p>
                      ) : null}
                    </div>
                    {item.manifestId === "microsoft_365" && item.supported ? (
                      <span className="shrink-0 text-xs font-semibold text-[var(--foreground-muted)]">
                        {t("setup.connector_set_up_later")}
                      </span>
                    ) : item.authMethod.includes("oauth") ? (
                      item.supported ? (
                        isConnected(connectedAccount) ? (
                          <span className="shrink-0 text-xs font-semibold text-[var(--foreground-muted)]">
                            {t("setup.connector_connected")}
                          </span>
                        ) : (
                          <button
                            className="shrink-0 rounded border px-3 py-2 text-sm font-semibold disabled:opacity-50"
                            disabled={waiting || busyConnector === item.manifestId}
                            onClick={() => void connect(item.manifestId)}
                            type="button"
                          >
                            {waiting || busyConnector === item.manifestId
                              ? t("setup.connector_connecting")
                              : t("setup.connector_connect")}
                          </button>
                        )
                      ) : (
                        <span className="shrink-0 text-xs font-semibold text-[var(--foreground-muted)]">
                          {t("setup.connector_unavailable")}
                        </span>
                      )
                    ) : (
                      <span className="shrink-0 text-xs font-semibold text-[var(--foreground-muted)]">
                        {t("setup.connector_available")}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          ) : null}

          {step === "sample" ? (
            <SetupSampleStep
              busy={busy}
              modelInventoryLoaded={modelInventoryLoaded}
              onRunSample={() => void runSample()}
              onSkipSample={() => void skipSample()}
              sampleAlreadyVerified={sampleAlreadyVerified}
              sampleModelReady={sampleModelReady}
              t={t}
            />
          ) : null}
        </div>

        {error ? (
          <p className="mt-5 text-sm text-[var(--warning)]" role="alert">
            {error}
          </p>
        ) : null}
        <div className="mt-7 flex justify-between">
          <button
            className="rounded border px-3 py-2 text-sm disabled:opacity-40"
            disabled={index === 0 || busy}
            onClick={() => setIndex((value) => Math.max(0, value - 1))}
            type="button"
          >
            {t("common.back")}
          </button>
          {step !== "sample" && !(step === "model" && modelRoute === "local") ? (
            <button
              className="rounded bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
              data-setup-action="continue"
              disabled={busy || (step === "model" && !modelRoute)}
              onClick={() => void advance()}
              type="button"
            >
              {t("setup.continue")}
            </button>
          ) : null}
        </div>
      </div>
    </section>
  );
}
