import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoutinesScreen } from "./RoutinesScreen";
import { routineDeliveryBlocksControls } from "./routineDeliveryControl";
import { routineFixture } from "./workflowReviewFixtures";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), isActive: true }));

vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@/components/AppShell", () => ({
  useAppShell: () => ({ setActiveItem: vi.fn() }),
}));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) => ({
      "routines.pause": "Pause",
      "routines.resume": "Resume",
      "routines.run_now": "Run now",
    })[key] ?? key,
  }),
}));

describe("Routine delivery control availability", () => {
  beforeEach(() => {
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "list_routines") {
        return [{
          ...routineFixture,
          deliveryState: "delivered",
          isActive: mocks.isActive,
        }];
      }
      if (command === "list_projects") {
        return [{ projectId: "project-1", name: "Launch" }];
      }
      if (command === "get_workflows") {
        return [{ id: "workflow-1", name: "Daily brief", workflowVersion: 1 }];
      }
      if (command === "get_background_service_status") {
        return { userEnabled: true, state: "active", detail: "Ready", checkedAtMs: 1 };
      }
      if (command === "get_channel_statuses") return [];
      return null;
    });
  });

  afterEach(() => {
    cleanup();
    mocks.isActive = true;
  });

  it.each([
    [true, "Pause"],
    [false, "Resume"],
  ] as const)(
    "keeps Run now and %s-mode controls available after delivery completes",
    async (isActive, expectedControl) => {
      mocks.isActive = isActive;
      render(<RoutinesScreen />);
      await screen.findByRole("heading", { name: "Morning brief" });

      expect(screen.getByRole("button", { name: "Run now" })).toBeEnabled();
      expect(screen.getByRole("button", { name: expectedControl })).toBeEnabled();
    },
  );

  it("blocks only active or unresolved delivery operations", () => {
    expect(routineDeliveryBlocksControls("delivered")).toBe(false);
    expect(routineDeliveryBlocksControls(null)).toBe(false);
    expect(routineDeliveryBlocksControls("retrying")).toBe(true);
    expect(routineDeliveryBlocksControls("needs_review")).toBe(true);
  });
});
