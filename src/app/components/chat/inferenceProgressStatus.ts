type ProgressTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export function inferenceProgressStatus(
  phase: "contacting" | "streaming",
  dynamicRoutingEnabled: boolean,
  concreteModelLabel: string,
  t: ProgressTranslate,
) {
  if (dynamicRoutingEnabled) {
    return t("chat.status.choosing_model");
  }
  return t(
    phase === "contacting"
      ? "chat.status.contacting_model"
      : "chat.status.streaming_model",
    { model: concreteModelLabel },
  );
}
