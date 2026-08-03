import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { SetupState } from "../integrations/integrationClient";
import {
  initialSetupStepIndex,
  setupStateForJourney,
  setupSteps,
  shouldShowSetupJourney,
  startupSurface,
} from "../integrations/setupGate";

function GateHarness({
  activeScreen,
  firstRunRequested = false,
  state,
}: {
  activeScreen: string;
  firstRunRequested?: boolean;
  state: SetupState;
}) {
  return shouldShowSetupJourney(state, firstRunRequested) ? (
    <p>onboarding</p>
  ) : (
    <p>{activeScreen}</p>
  );
}

afterEach(cleanup);

describe("setup access gate", () => {
  const finished: SetupState = {
    currentStep: "finished",
    completedAtMs: 1_725_000_000_000,
    sampleProjectId: "project_verified",
  };

  it("keeps durable completion authoritative across navigation and remounts", () => {
    const view = render(<GateHarness activeScreen="Mods" state={finished} />);
    expect(screen.getByText("Mods")).toBeVisible();

    view.rerender(<GateHarness activeScreen="Integrations" state={{ ...finished }} />);
    expect(screen.getByText("Integrations")).toBeVisible();

    view.unmount();
    render(<GateHarness activeScreen="Channels" state={{ ...finished }} />);
    expect(screen.getByText("Channels")).toBeVisible();
    expect(screen.queryByText("onboarding")).toBeNull();
  });

  it("opens a resetless first-run preview when the native launch flag is present", () => {
    render(
      <GateHarness activeScreen="Mods" firstRunRequested state={finished} />,
    );
    expect(screen.getByText("onboarding")).toBeVisible();
    expect(setupStateForJourney(finished, true)).toEqual({
      currentStep: "model",
      completedAtMs: undefined,
      sampleProjectId: undefined,
    });
  });

  it("still exposes onboarding for an intentionally incomplete test profile", () => {
    render(
      <GateHarness
        activeScreen="Mods"
        firstRunRequested
        state={{ currentStep: "model" }}
      />,
    );
    expect(screen.getByText("onboarding")).toBeVisible();
  });

  it("always repairs blocked storage before offering setup", () => {
    expect(startupSurface({ currentStep: "sample" }, true)).toBe("recovery");
    expect(startupSurface({ currentStep: "sample" }, false)).toBe("setup");
    expect(startupSurface(finished, false)).toBe("app");
  });

  it("maps completed and legacy terminal states to the final step, never step zero", () => {
    const sampleIndex = setupSteps.indexOf("sample");
    expect(initialSetupStepIndex("finished")).toBe(sampleIndex);
    expect(initialSetupStepIndex("complete")).toBe(sampleIndex);
    expect(initialSetupStepIndex("channel")).toBe(sampleIndex);
  });
});
