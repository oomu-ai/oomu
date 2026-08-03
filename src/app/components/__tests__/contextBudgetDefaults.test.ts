import { describe, expect, it } from "vitest";
import { contextBudgetBoundsForProvider } from "../ChatScreen";
import { cloudConfiguredProviders, configuredProviders } from "./ChatScreen.fixtures";

describe("local context defaults", () => {
  it("starts a new local conversation at 12K before hardware details arrive", () => {
    const bounds = contextBudgetBoundsForProvider(configuredProviders, "provider-1");
    expect(bounds.defaultValue).toBe(12_288);
    expect(bounds.steps).toContain(12_288);
  });

  it("defaults capable Macs to 12K", () => {
    const bounds = contextBudgetBoundsForProvider(configuredProviders, "provider-1", {
      physicalMemoryGb: 64,
      processorTier: "High",
      maxLocalContextBudget: 32_768,
    });
    expect(bounds.defaultValue).toBe(12_288);
  });

  it("keeps the default inside a standard Mac's supported range", () => {
    const bounds = contextBudgetBoundsForProvider(configuredProviders, "provider-1", {
      physicalMemoryGb: 8,
      processorTier: "Standard",
      maxLocalContextBudget: 8192,
    });
    expect(bounds.defaultValue).toBe(8192);
  });

  it("starts cloud conversations with the same practical 12K working budget", () => {
    const bounds = contextBudgetBoundsForProvider(cloudConfiguredProviders, "cloud-provider-1");
    expect(bounds.defaultValue).toBe(12_288);
    expect(bounds.steps).toContain(12_288);
  });
});
