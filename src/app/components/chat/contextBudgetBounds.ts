import type { ConfiguredProvider } from "@/lib/modelRegistry";

const LOCAL_DEFAULT = 12_288;
const LOCAL_STANDARD = [4096, 8192, 12_288];
const LOCAL_PREMIUM = [4096, 8192, 12_288, 16_384];
const LOCAL_ULTRA = [4096, 8192, 12_288, 16_384, 20_480, 24_576, 28_672, 32_768];
const CLOUD_STEPS = [8192, 12_288, 16_384, 32_768, 65_536, 98_304, 131_072];

export type ContextBudgetBounds = {
  target: "local" | "cloud";
  min: number;
  max: number;
  defaultValue: number;
  steps: number[];
  processorTier?: string;
};

export function contextBudgetBoundsForProvider(
  providers: ConfiguredProvider[],
  routeProviderId: string,
  hardware?: { maxLocalContextBudget?: number; processorTier: string; [key: string]: unknown } | null,
): ContextBudgetBounds {
  const providerClassId = providers.find((entry) => entry.id === routeProviderId)?.providerId
    ?? routeProviderId;
  const normalized = providerClassId.trim().toLowerCase().replace(/[\s-]+/g, "_");
  if (!routeProviderId.trim() || ["local", "local_model", "local_gemma"].includes(normalized)) {
    const maximum = hardware?.maxLocalContextBudget;
    const candidateSteps = maximum && maximum >= 32_768
      ? LOCAL_ULTRA
      : maximum && maximum >= 16_384
        ? LOCAL_PREMIUM
        : LOCAL_STANDARD;
    const supportedSteps = maximum
      ? candidateSteps.filter((step) => step <= maximum)
      : candidateSteps;
    const steps = supportedSteps.length > 0 ? supportedSteps : [Math.max(4096, maximum ?? 4096)];
    return {
      target: "local",
      min: steps[0],
      max: steps[steps.length - 1],
      defaultValue: Math.min(LOCAL_DEFAULT, steps[steps.length - 1]),
      steps,
      processorTier: hardware?.processorTier,
    };
  }
  return {
    target: "cloud",
    min: CLOUD_STEPS[0],
    max: CLOUD_STEPS[CLOUD_STEPS.length - 1],
    defaultValue: 12_288,
    steps: CLOUD_STEPS,
  };
}
