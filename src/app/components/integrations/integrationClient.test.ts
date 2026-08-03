import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

import { integrationApi } from "./integrationClient";

describe("integrationApi.runSample", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ currentStep: "sample" });
  });

  it("preserves the legacy request when completion is not specified", async () => {
    await integrationApi.runSample("local");

    expect(invokeMock).toHaveBeenCalledWith("run_setup_sample_task", {
      request: { modelRoute: "local" },
    });
  });

  it("can keep onboarding open after the durable sample completes", async () => {
    await integrationApi.runSample("local", { completeSetup: false });

    expect(invokeMock).toHaveBeenCalledWith("run_setup_sample_task", {
      request: { modelRoute: "local", completeSetup: false },
    });
  });
});
