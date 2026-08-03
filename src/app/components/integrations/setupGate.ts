import type { SetupState } from "./integrationClient";

export const setupSteps = ["model", "permissions", "connectors", "sample"] as const;
type SetupStep = (typeof setupSteps)[number];

export function initialSetupStepIndex(step: string): number {
  // Legacy delivery states and a completed journey belong at the terminal
  // sample step. In particular, `finished` must never fall through to step 0.
  if (step === "channel" || step === "complete" || step === "finished") {
    return setupSteps.indexOf("sample");
  }
  const index = setupSteps.indexOf(step as SetupStep);
  return index < 0 ? 0 : index;
}

export function shouldShowSetupJourney(
  state: SetupState,
  firstRunRequested = false,
): boolean {
  return firstRunRequested || state.currentStep !== "finished";
}

export function setupStateForJourney(
  state: SetupState,
  firstRunRequested = false,
): SetupState {
  if (!firstRunRequested) return state;
  return {
    ...state,
    currentStep: "model",
    sampleProjectId: undefined,
    completedAtMs: undefined,
  };
}

type StartupSurface = "recovery" | "setup" | "app";

export function startupSurface(
  state: SetupState,
  recoveryRequired: boolean,
  firstRunRequested = false,
): StartupSurface {
  if (recoveryRequired) return "recovery";
  return shouldShowSetupJourney(state, firstRunRequested) ? "setup" : "app";
}
