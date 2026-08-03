"use client";

import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import { TrustPanel } from "./TrustPanel";

type SovereignIdentityProfile = {
  fingerprint: string;
  public_key: string;
  hardware_binding?: string | null;
  storage_backend?: string | null;
};

export function humanizeStorageBackend(
  raw: string | null | undefined,
  fallback: string,
) {
  const value = (raw ?? "").toLowerCase();
  if (!value || value.includes("sqlite") || value.includes("sqlcipher")) {
    return fallback;
  }
  return raw!;
}

function PrivacySwitch({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={`relative h-7 w-12 shrink-0 rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        checked
          ? "border-[var(--inverse-background)] bg-[var(--inverse-background)]"
          : "border-[var(--border-strong)] bg-[var(--background)]"
      }`}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      role="switch"
      type="button"
    >
      <span
        className={`absolute left-0 top-1 h-5 w-5 rounded-full bg-[var(--background)] shadow-[0_1px_3px_rgba(15,23,42,0.24)] transition-transform ${
          checked ? "translate-x-5" : "translate-x-1"
        }`}
      />
    </button>
  );
}

export function PrivacyPanel({
  onPrivacySettingsChange,
}: {
  onPrivacySettingsChange?: (settings: PrivacySettingsState) => void;
}) {
  const { t } = useI18n();
  const tRef = useRef(t);
  const [profile, setProfile] = useState<SovereignIdentityProfile | null>(null);
  const [privacySettings, setPrivacySettings] =
    useState<PrivacySettingsState | null>(null);
  const [copiedFingerprint, setCopiedFingerprint] = useState(false);
  const [copiedKey, setCopiedKey] = useState(false);
  const [showFullKey, setShowFullKey] = useState(false);
  const [error, setError] = useState("");
  const [privacySettingsError, setPrivacySettingsError] = useState("");
  const [isSavingWebGrounding, setIsSavingWebGrounding] = useState(false);
  const [isRetryingIdentity, setIsRetryingIdentity] = useState(false);

  useEffect(() => {
    tRef.current = t;
  }, [t]);

  useEffect(() => {
    let active = true;
    invoke<SovereignIdentityProfile>("get_sovereign_identity")
      .then((data) => {
        if (active) setProfile(data);
      })
      .catch((err) => {
        if (active) {
          setError(
            err && typeof err === "object" && "message" in err
              ? String(err.message)
              : tRef.current("settings.privacy.errors.identity"),
          );
        }
      });
    invoke<PrivacySettingsState>("get_privacy_settings")
      .then((data) => {
        if (active) {
          setPrivacySettings(data);
          onPrivacySettingsChange?.(data);
        }
      })
      .catch((err) => {
        if (active) {
          setPrivacySettingsError(
            err && typeof err === "object" && "message" in err
              ? String(err.message)
              : tRef.current("settings.privacy.errors.privacy_settings"),
          );
        }
      });
    return () => {
      active = false;
    };
  }, [onPrivacySettingsChange]);

  const copyToClipboard = (text: string, type: "fingerprint" | "key") => {
    void navigator.clipboard.writeText(text);
    if (type === "fingerprint") {
      setCopiedFingerprint(true);
      setTimeout(() => setCopiedFingerprint(false), 2000);
    } else {
      setCopiedKey(true);
      setTimeout(() => setCopiedKey(false), 2000);
    }
  };

  const handleOpenPrivacyPolicy = async (
    event: MouseEvent<HTMLButtonElement>,
  ) => {
    event.preventDefault();
    await invoke("open_oomu_privacy_policy");
  };

  const updateAutomatedWebGroundingEnabled = async (enabled: boolean) => {
    setIsSavingWebGrounding(true);
    setPrivacySettingsError("");
    try {
      const response = await invoke<PrivacySettingsState>(
        "set_automated_web_grounding_enabled",
        { enabled },
      );
      setPrivacySettings(response);
      onPrivacySettingsChange?.(response);
    } catch (err) {
      setPrivacySettingsError(
        err && typeof err === "object" && "message" in err
          ? String(err.message)
          : t("settings.privacy.errors.web_grounding_update"),
      );
    } finally {
      setIsSavingWebGrounding(false);
    }
  };

  const retryIdentity = async () => {
    setIsRetryingIdentity(true);
    try {
      const recovered = await invoke<SovereignIdentityProfile>(
        "retry_sovereign_identity_health",
      );
      setProfile(recovered);
      setError("");
    } catch (err) {
      setError(
        err && typeof err === "object" && "message" in err
          ? String(err.message)
          : t("settings.privacy.errors.identity"),
      );
    } finally {
      setIsRetryingIdentity(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <h2 className="text-sm font-semibold text-[var(--foreground)]">
          {t("settings.privacy.overview_title")}
        </h2>
        <div className="mt-2 flex flex-col gap-3 text-sm leading-6 text-[var(--foreground-muted)]">
          <p>
            {t("settings.privacy.overview.local_first")}
          </p>
          <p>
            {t("settings.privacy.overview.web_search")}
          </p>
          <p>
            {t("settings.privacy.overview.read_policy_prefix")}{" "}
            <button
              className="inline p-0 font-medium text-[var(--accent)] hover:underline"
              onClick={(event) => void handleOpenPrivacyPolicy(event)}
              type="button"
            >
              {t("settings.privacy.overview.policy_link_inline")}
            </button>
            {t("settings.privacy.overview.read_policy_suffix")}
          </p>
        </div>
        <div className="mt-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
          <label className="flex items-center justify-between gap-4">
            <span>
              <span className="block text-sm font-semibold text-[var(--foreground)]">
                {t("settings.privacy.web_grounding_title")}
              </span>
              <span className="mt-1 block text-xs leading-5 text-[var(--foreground-muted)]">
                {privacySettings?.automatedWebGroundingEnabled
                  ? t("settings.privacy.web_grounding_on")
                  : t("settings.privacy.web_grounding_off")}
              </span>
            </span>
            <PrivacySwitch
              checked={Boolean(privacySettings?.automatedWebGroundingEnabled)}
              disabled={!privacySettings || isSavingWebGrounding}
              label={t("settings.privacy.web_grounding_title")}
              onChange={(checked) => void updateAutomatedWebGroundingEnabled(checked)}
            />
          </label>
        </div>
        {privacySettingsError && (
          <p className="mt-3 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-xs font-medium text-[var(--destructive)]">
            {privacySettingsError}
          </p>
        )}
      </section>

      <TrustPanel />

      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <h2 className="mb-4 text-sm font-semibold text-[var(--foreground)]">
          {t("settings.privacy.device_identity")}
        </h2>
        {error ? (
          <div
            className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4"
            role="status"
          >
            <p className="text-sm font-semibold text-[var(--foreground)]">
              {t("settings.privacy.identity_attention_title")}
            </p>
            <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
              {t("settings.privacy.identity_attention_description")}
            </p>
            <button
              className="mt-3 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-50"
              disabled={isRetryingIdentity}
              onClick={() => void retryIdentity()}
              type="button"
            >
              {isRetryingIdentity
                ? t("settings.privacy.identity_retrying")
                : t("settings.privacy.identity_retry")}
            </button>
            <details className="mt-3 text-xs text-[var(--foreground-muted)]">
              <summary className="cursor-pointer font-medium">
                {t("common.details")}
              </summary>
              <p className="mt-2 break-words font-mono leading-5">{error}</p>
            </details>
          </div>
        ) : profile ? (
          <div className="flex flex-col gap-4">
            <div className="flex items-center justify-between border-b border-[var(--border-soft)] pb-3">
              <div>
                <span className="text-xs font-semibold text-[var(--foreground-muted)]">{t("settings.privacy.fingerprint")}</span>
                <p className="mt-1 font-mono text-sm text-[var(--foreground)]">{profile.fingerprint.slice(0, 16)}...</p>
              </div>
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-medium hover:bg-[var(--fill-hover)]"
                onClick={() => copyToClipboard(profile.fingerprint, "fingerprint")}
                type="button"
              >
                {copiedFingerprint ? t("common.copied") : t("common.copy")}
              </button>
            </div>

            <div className="flex items-center justify-between border-b border-[var(--border-soft)] pb-3">
              <div>
                <span className="text-xs font-semibold text-[var(--foreground-muted)]">{t("settings.privacy.hardware_binding")}</span>
                <p className="mt-1 text-sm text-[var(--foreground)]">{profile.hardware_binding || t("settings.privacy.default_hardware_binding")}</p>
              </div>
            </div>

            <div className="flex items-center justify-between border-b border-[var(--border-soft)] pb-3">
              <div>
                <span className="text-xs font-semibold text-[var(--foreground-muted)]">{t("settings.privacy.storage_backend")}</span>
                <p className="mt-1 text-sm text-[var(--foreground)]">{humanizeStorageBackend(profile.storage_backend, t("settings.privacy.default_storage_backend"))}</p>
              </div>
            </div>

            <div className="flex flex-col gap-2 pt-1">
              <span className="text-xs font-semibold text-[var(--foreground-muted)]">{t("settings.privacy.public_key")}</span>
              <div className="flex gap-2">
                <button
                  className="text-xs font-medium text-[var(--accent)] hover:underline"
                  onClick={() => setShowFullKey(!showFullKey)}
                  type="button"
                >
                  {showFullKey ? t("settings.privacy.hide_public_key") : t("settings.privacy.show_public_key")}
                </button>
                <button
                  className="ml-2 text-xs font-medium text-[var(--accent)] hover:underline"
                  onClick={() => copyToClipboard(profile.public_key, "key")}
                  type="button"
                >
                  {copiedKey ? t("common.copied") : t("common.copy_key")}
                </button>
              </div>
              {showFullKey && (
                <div className="mt-2 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--accent-background)] p-3">
                  <p className="break-all font-mono text-xs text-[var(--foreground-muted)] select-all">{profile.public_key}</p>
                </div>
              )}
            </div>
          </div>
        ) : (
          <p className="text-xs text-[var(--foreground-muted)]">{t("settings.privacy.retrieving_identity")}</p>
        )}
      </section>
    </div>
  );
}
