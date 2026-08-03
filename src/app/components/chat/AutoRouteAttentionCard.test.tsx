import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AutoRouteAttentionCard,
  type AutoRouteAttention,
} from "./AutoRouteAttentionCard";

const translations: Record<string, string> = {
  "sprint_301.route.on_device_model": "On-device model",
  "sprint_301.auto_route_recovery.choose_model_title": "Choose an on-device model",
  "sprint_301.auto_route_recovery.choose_model_body": "Choose the model Auto-route should use. Your message is saved.",
  "sprint_301.auto_route_recovery.preparing_title": "Getting the on-device model ready",
  "sprint_301.auto_route_recovery.preparing_body": "OOMU is preparing the model. Your message is saved.",
  "sprint_301.auto_route_recovery.timeout_title": "The on-device model took too long",
  "sprint_301.auto_route_recovery.timeout_body": "Your message is saved.",
  "sprint_301.auto_route_recovery.cloud_title": "Set up a cloud model to continue",
  "sprint_301.auto_route_recovery.cloud_body": "This request needs a cloud model. Your message is saved.",
  "sprint_301.auto_route_recovery.saved_work_title": "Check saved work",
  "sprint_301.auto_route_recovery.saved_work_body": "OOMU needs to check saved work before continuing.",
  "sprint_301.auto_route_recovery.unknown_title": "Auto-route needs attention",
  "sprint_301.auto_route_recovery.unknown_body": "Your message is saved. Choose how to continue.",
  "sprint_301.auto_route_recovery.interrupted_title": "Reply stopped",
  "sprint_301.auto_route_recovery.interrupted_body": "Your message is saved. Continue when you're ready.",
  "sprint_301.auto_route_recovery.choose_model": "Choose model",
  "sprint_301.auto_route_recovery.try_again": "Try again",
  "sprint_301.auto_route_recovery.open_models": "Open Models",
  "sprint_301.auto_route_recovery.check_saved_work": "Check and recover",
  "sprint_301.auto_route_recovery.continue_when_ready": "Continue when ready",
  "sprint_301.auto_route_recovery.preparing_wait": "OOMU will continue this message when the model is ready.",
  "sprint_301.auto_route_recovery.use_local": "Use {model}",
  "sprint_301.auto_route_recovery.use_cloud": "Use cloud model",
  "sprint_301.auto_route_recovery.cloud_disclosure": "Using the cloud sends this turn off your Mac.",
  "sprint_301.auto_route_recovery.cancel": "Cancel request",
  "sprint_301.auto_route_recovery.working": "Working…",
  "sprint_301.auto_route_recovery.action_failed": "That did not work. Your message is still saved.",
  "sprint_301.auto_route_recovery.technical_details": "Show technical details",
  "sprint_301.auto_route_recovery.error_code": "Error code",
  "sprint_301.auto_route_recovery.stopped_at": "Stopped at",
};

function t(key: string, variables?: Record<string, string | number>) {
  let value = translations[key] ?? key;
  Object.entries(variables ?? {}).forEach(([name, replacement]) => {
    value = value.replace(`{${name}}`, String(replacement));
  });
  return value;
}

function attention(overrides: Partial<AutoRouteAttention> = {}): AutoRouteAttention {
  return {
    sessionId: "session-301",
    rootTurnId: "turn-301",
    turnId: "turn-301",
    generationToken: "generation-301",
    localProviderId: "local-provider",
    localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    recommendedLocalProviderId: "local-provider",
    recommendedLocalModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    cloudModelId: "gemini-3.5-flash",
    failureCode: "auto_route_session_baseline_missing",
    failureBoundary: "active_session_configs",
    kind: "choose_model",
    continueWhenReady: false,
    ...overrides,
  };
}

afterEach(cleanup);

describe("AutoRouteAttentionCard", () => {
  it("repairs a verified model baseline without showing raw storage identity", async () => {
    const user = userEvent.setup();
    const onChoice = vi.fn(async () => undefined);
    render(<AutoRouteAttentionCard attention={attention({ localModelId: "models" })} onChoice={onChoice} t={t} />);
    expect(screen.getByRole("alert")).toHaveFocus();
    expect(screen.queryByText("models")).toBeNull();
    screen.getByRole("button", { name: "Use Gemma 4 E2B" }).focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(onChoice).toHaveBeenCalledWith("repair_model"));
  });

  it("continues automatically after readiness returns without offering a duplicate retry", async () => {
    const onChoice = vi.fn(async () => undefined);
    render(<AutoRouteAttentionCard attention={attention({
      kind: "preparing",
      failureCode: "classifier_recovering",
    })} onChoice={onChoice} t={t} />);
    expect(screen.getByRole("status")).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue when ready" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel request" }));
    await waitFor(() => expect(onChoice).toHaveBeenCalledWith("cancel"));
  });

  it("opens real model selection when no verified recommendation exists", async () => {
    const user = userEvent.setup();
    const onChoice = vi.fn(async () => undefined);
    render(<AutoRouteAttentionCard attention={attention({
      recommendedLocalProviderId: "",
      recommendedLocalModelId: "",
      localProviderId: "",
      localModelId: "",
    })} onChoice={onChoice} t={t} />);

    await user.click(screen.getByRole("button", { name: "Choose model" }));
    expect(onChoice).toHaveBeenCalledWith("open_models");
  });

  it("announces a new recovery turn even when its session and error code are unchanged", () => {
    const view = render(<AutoRouteAttentionCard attention={attention()} onChoice={vi.fn()} t={t} />);
    const card = screen.getByRole("alert");
    expect(card).toHaveFocus();
    screen.getByRole("button", { name: "Cancel request" }).focus();

    view.rerender(<AutoRouteAttentionCard attention={attention({
      turnId: "turn-302",
      generationToken: "generation-302",
    })} onChoice={vi.fn()} t={t} />);

    expect(card).toHaveFocus();
  });

  it("offers only actions that can change a real classifier timeout", () => {
    render(<AutoRouteAttentionCard attention={attention({
      kind: "timeout",
      failureCode: "classifier_inference_timeout",
    })} onChoice={vi.fn()} t={t} />);
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Use Gemma 4 E2B" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /cloud/i })).toBeNull();
  });

  it("opens model setup or keeps the request on-device when cloud setup is missing", () => {
    render(<AutoRouteAttentionCard attention={attention({
      kind: "cloud_setup",
      cloudModelId: "",
      failureCode: "auto_route_cloud_target_missing",
    })} onChoice={vi.fn()} t={t} />);
    expect(screen.getByRole("button", { name: "Open Models" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Use Gemma 4 E2B" })).toBeVisible();
    expect(screen.queryByText(/sends this turn/i)).toBeNull();
  });

  it("offers one obvious continuation for a user-stopped reply", () => {
    render(<AutoRouteAttentionCard attention={attention({
      kind: "interrupted",
      failureCode: "turn_interrupted",
      failureBoundary: "user_stop",
      localProviderId: "",
      localModelId: "",
      recommendedLocalProviderId: "",
      recommendedLocalModelId: "",
      cloudModelId: "",
    })} onChoice={vi.fn()} t={t} />);
    expect(screen.getByText("Reply stopped")).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue when ready" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /model/i })).toBeNull();
  });
});
