"use client";

import { useEffect, useState } from "react";

import { integrationApi, type SetupState } from "./components/integrations/integrationClient";
import { shouldShowDegradedLanding, type DegradedModeStatus } from "./homeAgents";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import type { PrivacySettingsState } from "@/lib/privacySettings";

const NATIVE_HEALTH_POLL_MS = 5_000;

export function useHomeStartupState() {
  const [degradedModeStatus, setDegradedModeStatus] = useState<DegradedModeStatus | null>(null);
  const [degradedModeProbeFailed, setDegradedModeProbeFailed] = useState(false);
  const [privacySettings, setPrivacySettings] = useState<PrivacySettingsState | null>(null);
  const [privacySettingsProbeFailed, setPrivacySettingsProbeFailed] = useState(false);
  const [setupState, setSetupState] = useState<SetupState | null>(null);
  const [setupProbeFailed, setSetupProbeFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;

    async function loadDegradedModeStatus() {
      try {
        const result = await invoke<DegradedModeStatus>("get_degraded_mode_status");
        if (!cancelled) {
          setDegradedModeStatus(result);
          setDegradedModeProbeFailed(false);
        }
      } catch (error) {
        if (!cancelled) setDegradedModeProbeFailed(true);
        console.error("Unable to load native recovery status.", error);
      }
    }

    void loadDegradedModeStatus();
    const healthPoll = window.setInterval(loadDegradedModeStatus, NATIVE_HEALTH_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(healthPoll);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function loadPrivacySettings() {
      try {
        const result = await invoke<PrivacySettingsState>("get_privacy_settings");
        if (!cancelled) {
          setPrivacySettings(result);
          setPrivacySettingsProbeFailed(false);
        }
      } catch (error) {
        if (!cancelled) setPrivacySettingsProbeFailed(true);
        console.error("Unable to load native privacy settings.", error);
      }
    }

    void loadPrivacySettings();
    const privacyPoll = window.setInterval(loadPrivacySettings, NATIVE_HEALTH_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(privacyPoll);
    };
  }, []);

  useEffect(() => {
    if (!privacySettings?.licenseAccepted) return;
    let cancelled = false;
    void integrationApi
      .setup()
      .then((state) => {
        if (!cancelled) {
          setSetupState(state);
          setSetupProbeFailed(false);
        }
      })
      .catch((error) => {
        if (!cancelled) setSetupProbeFailed(true);
        console.error("Unable to load first-run setup state.", error);
      });
    return () => {
      cancelled = true;
    };
  }, [privacySettings?.licenseAccepted]);

  const applicationUpdateReady = Boolean(
    privacySettings?.licenseAccepted &&
      setupState?.currentStep === "finished" &&
      degradedModeStatus &&
      !shouldShowDegradedLanding(degradedModeStatus, "chat") &&
      !degradedModeProbeFailed &&
      !privacySettingsProbeFailed &&
      !setupProbeFailed,
  );

  useEffect(() => {
    if (!isTauriRuntime) return;
    void invoke<boolean>("set_application_update_ui_ready", {
      ready: applicationUpdateReady,
    }).catch(() => {
      // The native shell remains safely disabled if readiness cannot be confirmed.
    });
  }, [applicationUpdateReady]);

  return {
    degradedModeProbeFailed,
    degradedModeStatus,
    privacySettings,
    privacySettingsProbeFailed,
    setDegradedModeStatus,
    setPrivacySettings,
    setSetupState,
    setupProbeFailed,
    setupState,
  };
}
