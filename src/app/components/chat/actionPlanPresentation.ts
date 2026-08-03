export type ActionPlanStep = {
  step: string;
  tool: {
    kind: string;
    [key: string]: unknown;
  };
  risk_level: "low" | "medium" | "high";
};

type Translate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function createFileDestination(step: ActionPlanStep): string | null {
  if (
    step.tool.kind !== "registered_task_tool"
    || step.tool.operation !== "create_file"
  ) {
    return null;
  }
  const file = record(record(step.tool.arguments)?.file);
  const destination = file?.destinationPath;
  return typeof destination === "string" && destination.trim()
    ? destination
    : null;
}

export function actionPlanStepPresentation(
  step: ActionPlanStep,
  t: Translate,
): string {
  const destination = createFileDestination(step);
  return destination
    ? t("chat.plan.create_file_destination", { path: destination })
    : step.step;
}
