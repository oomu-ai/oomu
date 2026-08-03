import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AutoRouteActivationRecoveryCard } from "./AutoRouteActivationRecoveryCard";

const copy: Record<string, string> = {
  "auto_route_activation.title": "Auto-route needs attention",
  "auto_route_activation.enable_body": "OOMU couldn’t turn on Auto-route. Your current model and message are unchanged.",
  "auto_route_activation.disable_body": "OOMU couldn’t turn off Auto-route. Auto-route is still on, and your message is unchanged.",
  "auto_route_activation.choose_model_body": "Choose an on-device model, then try again. Your message is saved.",
  "auto_route_activation.working": "Trying again…",
  "auto_route_activation.retry": "Try again",
  "auto_route_activation.choose_model": "Choose a model",
  "auto_route_activation.keep_current_model": "Keep current model",
  "auto_route_activation.leave_auto_route_on": "Leave Auto-route on",
  "auto_route_activation.technical_details": "Technical details",
  "auto_route_activation.error_code": "Repair code",
};
const t = (key: string) => copy[key] ?? key;

describe("AutoRouteActivationRecoveryCard", () => {
  afterEach(cleanup);

  it("makes model selection the only primary recovery for a missing provider choice", () => {
    render(
      <AutoRouteActivationRecoveryCard
        failure={{
          sessionId: "session-302",
          code: "auto_route_provider_configuration_missing",
          retryable: false,
          desiredEnabled: true,
        }}
        onChooseModel={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
        t={t}
      />,
    );

    expect(screen.queryByRole("button", { name: "Try again" }))
      .toBeNull();
    expect(screen.getByRole("button", { name: "Choose a model" }))
      .toHaveClass("bg-[var(--inverse-background)]");
    expect(screen.getByRole("button", { name: "Keep current model" })).toBeVisible();
    expect(screen.getByText(/Choose an on-device model/)).toBeVisible();
  });

  it("offers retry only for a transient activation failure", () => {
    render(
      <AutoRouteActivationRecoveryCard
        failure={{
          sessionId: "session-302",
          code: "auto_route_provider_store_unavailable",
          retryable: true,
          desiredEnabled: true,
        }}
        onChooseModel={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
        t={t}
      />,
    );

    expect(screen.getByRole("button", { name: "Try again" }))
      .toBeTruthy();
    expect(screen.queryByRole("button", { name: "Choose a model" }))
      .toBeNull();
  });

  it("rechecks an uncategorized failure instead of leaving a dead end", () => {
    render(
      <AutoRouteActivationRecoveryCard
        failure={{
          sessionId: "session-302",
          code: "auto_route_activation_state_unavailable",
          retryable: false,
          desiredEnabled: true,
        }}
        onChooseModel={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
        t={t}
      />,
    );

    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Keep current model" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Choose a model" })).toBeNull();
  });

  it("describes a failed disable truthfully and never offers model selection", () => {
    render(
      <AutoRouteActivationRecoveryCard
        failure={{
          sessionId: "session-302",
          code: "auto_route_provider_identity_mismatch",
          retryable: false,
          desiredEnabled: false,
        }}
        onChooseModel={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
        t={t}
      />,
    );

    expect(screen.getByRole("alert")).toHaveFocus();
    expect(screen.getByText(/couldn’t turn off Auto-route/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Leave Auto-route on" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Choose a model" })).toBeNull();
  });
});
