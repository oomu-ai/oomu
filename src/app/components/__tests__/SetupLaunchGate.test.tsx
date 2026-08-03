import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SetupLaunchGate } from "../integrations/SetupLaunchGate";

vi.mock("../integrations/SetupJourney", () => ({
  SetupJourney: ({ initialState, onComplete, previewMode }: {
    initialState: { currentStep: string };
    onComplete: (state: { currentStep: string }) => void;
    previewMode: boolean;
  }) => (
    <button
      data-preview-mode={String(previewMode)}
      onClick={() => onComplete({ currentStep: "finished" })}
      type="button"
    >
      {`Setup ${initialState.currentStep}`}
    </button>
  ),
}));

const healthyStatus = {
  active: false,
  reason: null,
  hasVolatileStorage: false,
  subsystems: [],
};

afterEach(cleanup);

describe("SetupLaunchGate", () => {
  it("keeps the app navigable while setup is incomplete", () => {
    render(
      <SetupLaunchGate
        activeItem="projects"
        degradedModeStatus={healthyStatus}
        onOpenSettings={vi.fn()}
        onProviderConfigured={vi.fn()}
        onSetupStateChange={vi.fn()}
        onStatusChange={vi.fn()}
        setupState={{ currentStep: "model" }}
      >
        <p>OOMU app</p>
      </SetupLaunchGate>,
    );

    expect(screen.getByText("OOMU app")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Setup model" })).toBeNull();
  });

  it("restores setup when the user returns to Chat", () => {
    const props = {
      degradedModeStatus: healthyStatus,
      onOpenSettings: vi.fn(),
      onProviderConfigured: vi.fn(),
      onSetupStateChange: vi.fn(),
      onStatusChange: vi.fn(),
      setupState: { currentStep: "model" },
    };
    const { rerender } = render(
      <SetupLaunchGate {...props} activeItem="chat">
        <p>OOMU app</p>
      </SetupLaunchGate>,
    );

    expect(screen.getByRole("button", { name: "Setup model" })).toBeVisible();

    rerender(
      <SetupLaunchGate {...props} activeItem="projects">
        <p>OOMU app</p>
      </SetupLaunchGate>,
    );
    expect(screen.getByText("OOMU app")).toBeVisible();

    rerender(
      <SetupLaunchGate {...props} activeItem="chat">
        <p>OOMU app</p>
      </SetupLaunchGate>,
    );
    expect(screen.getByRole("button", { name: "Setup model" })).toBeVisible();
  });

  it("opens and exits a resetless first-run preview for a finished profile", () => {
    const onSetupStateChange = vi.fn();
    render(
      <SetupLaunchGate
        activeItem="chat"
        degradedModeStatus={healthyStatus}
        firstRunSetup
        onOpenSettings={vi.fn()}
        onProviderConfigured={vi.fn()}
        onSetupStateChange={onSetupStateChange}
        onStatusChange={vi.fn()}
        setupState={{ currentStep: "finished", completedAtMs: 123 }}
      >
        <p>OOMU app</p>
      </SetupLaunchGate>,
    );

    const setup = screen.getByRole("button", { name: "Setup model" });
    expect(setup).toHaveAttribute("data-preview-mode", "true");
    fireEvent.click(setup);

    expect(screen.getByText("OOMU app")).toBeVisible();
    expect(onSetupStateChange).not.toHaveBeenCalled();
  });
});
