import type { ContextBudgetBounds } from "./contextBudgetBounds";

const MAX_PROVIDER_CONTEXT_BUDGET = 1_000_000;

function parseContextBudgetTokens(value: string | number | null | undefined) {
  if (typeof value === "number" && Number.isFinite(value)) {
    const budget = Math.round(value);
    return budget > 0 ? budget : null;
  }

  const rawValue = String(value ?? "").trim().toLowerCase();
  if (!rawValue || rawValue.includes("provider-defined")) return null;
  const match = rawValue.match(/(\d+(?:\.\d+)?)(?:\s*([km]))?/i);
  if (!match) return null;
  const parsed = Number(match[1]);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  const multiplier = match[2]?.toLowerCase() === "m"
    ? 1_000_000
    : match[2]?.toLowerCase() === "k" ? 1_000 : 1;
  return Math.round(parsed * multiplier);
}

export function normalizeContextBudget(
  value: string | number | null | undefined,
  bounds: ContextBudgetBounds,
) {
  const parsedBudget = parseContextBudgetTokens(value);
  if (parsedBudget === null) return String(bounds.defaultValue);
  const budget = Math.min(MAX_PROVIDER_CONTEXT_BUDGET, parsedBudget);
  if (budget < bounds.min) {
    return String(bounds.target === "cloud" ? bounds.defaultValue : bounds.min);
  }
  return String(nearestContextBudgetStep(Math.min(bounds.max, budget), bounds));
}

export function nearestContextBudgetStep(value: number, bounds: ContextBudgetBounds) {
  const clampedValue = Math.min(bounds.max, Math.max(bounds.min, value));
  return bounds.steps.reduce((nearest, step) => {
    const distance = Math.abs(step - clampedValue);
    const nearestDistance = Math.abs(nearest - clampedValue);
    if (distance < nearestDistance) return step;
    if (distance === nearestDistance && step > nearest) return step;
    return nearest;
  }, bounds.steps[0] ?? bounds.defaultValue);
}

export function formatContextBudgetLabel(value: number) {
  if (value < 1024) return `${value} tokens`;
  const scaledValue = value / 1024;
  const displayValue = Number.isInteger(scaledValue)
    ? String(scaledValue)
    : scaledValue.toFixed(1);
  return `${displayValue}K tokens`;
}
