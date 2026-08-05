"use client";

import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { useState, type ReactNode } from "react";
import type { DegradedModeStatus } from "../../homeAgents";
import { shouldShowDegradedLanding } from "../../homeAgents";
import { DegradedModeLanding } from "../HomeChrome";
import type { SetupState } from "./integrationClient";
import { SetupJourney } from "./SetupJourney";
import { setupStateForJourney, startupSurface } from "./setupGate";

type SetupLaunchGateProps = {
  activeItem: string;
  children: ReactNode;
  degradedModeStatus: DegradedModeStatus;
  firstRunSetup?: boolean;
  onOpenSettings: () => void;
  onProviderConfigured: (provider: ConfiguredProvider) => void;
  onSetupStateChange: (state: SetupState) => void;
  onStatusChange: (status: DegradedModeStatus) => void;
  setupState: SetupState;
};

export function SetupLaunchGate({
  activeItem,
  children,
  degradedModeStatus,
  firstRunSetup = false,
  onOpenSettings,
  onProviderConfigured,
  onSetupStateChange,
  onStatusChange,
  setupState,
}: SetupLaunchGateProps) {
  const [firstRunJourneyDismissed, setFirstRunJourneyDismissed] = useState(false);
  const [dismissedRecoveryKey, setDismissedRecoveryKey] = useState<string | null>(null);
  const firstRunRequested = firstRunSetup && !firstRunJourneyDismissed;
  const previewMode = firstRunRequested && setupState.currentStep === "finished";
  const recoveryKey = JSON.stringify({
    hasVolatileStorage: degradedModeStatus.hasVolatileStorage,
    reason: degradedModeStatus.reason,
    subsystems: degradedModeStatus.subsystems
      .filter((subsystem) => subsystem.active)
      .map((subsystem) => [subsystem.subsystem, subsystem.cause]),
  });
  const surface = startupSurface(
    setupState,
    dismissedRecoveryKey !== recoveryKey &&
      shouldShowDegradedLanding(degradedModeStatus, activeItem),
    firstRunRequested,
  );

  if (surface === "recovery") {
    return (
      <DegradedModeLanding
        status={degradedModeStatus}
        onContinue={() => setDismissedRecoveryKey(recoveryKey)}
        onOpenSettings={onOpenSettings}
        onStatusChange={onStatusChange}
      />
    );
  }

  // Setup is the Chat landing experience, not a global navigation lock. Native
  // model installation continues independently and its app-wide indicator
  // remains visible while the user works elsewhere.
  if (surface === "setup" && activeItem === "chat") {
    return (
      <SetupJourney
        initialState={setupStateForJourney(setupState, firstRunRequested)}
        onComplete={(state) => {
          if (!previewMode) onSetupStateChange(state);
          if (firstRunRequested) setFirstRunJourneyDismissed(true);
        }}
        onProviderConfigured={onProviderConfigured}
        previewMode={previewMode}
      />
    );
  }

  return children;
}
