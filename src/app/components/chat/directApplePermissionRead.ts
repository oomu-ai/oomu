export type PermissionRecoveryChoice = "retry" | "cancel" | "unavailable";

export type PermissionReadOutcome<T> =
  | { status: "completed"; value: T }
  | { status: "cancelled" };

export async function runPermissionRecoverableAppleRead<T>(
  operation: () => Promise<T>,
  recover: (error: unknown) => Promise<PermissionRecoveryChoice> | null,
): Promise<PermissionReadOutcome<T>> {
  for (;;) {
    try {
      return { status: "completed", value: await operation() };
    } catch (error) {
      const pendingRecovery = recover(error);
      const choice = pendingRecovery ? await pendingRecovery : "unavailable";
      if (choice === "retry") continue;
      if (choice === "cancel") return { status: "cancelled" };
      throw error;
    }
  }
}
