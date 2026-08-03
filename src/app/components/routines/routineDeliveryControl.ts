import type { RoutineRecord } from "./routineClient";

export function routineDeliveryBlocksControls(
  state: RoutineRecord["deliveryState"],
) {
  return state === "retrying" || state === "needs_review";
}
