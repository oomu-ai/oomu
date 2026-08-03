export function stableErrorCode(error: unknown): string {
  if (error instanceof Error) {
    const explicitCode = "code" in error && typeof error.code === "string"
      ? error.code.trim()
      : "";
    if (/^[a-z][a-z0-9_]*$/u.test(explicitCode)) return explicitCode;
    const message = error.message.trim();
    return /^[a-z][a-z0-9_]*$/u.test(message) ? message : "";
  }
  if (typeof error === "string") {
    try {
      return stableErrorCode(JSON.parse(error));
    } catch {
      return "";
    }
  }
  return error &&
    typeof error === "object" &&
    "code" in error &&
    typeof error.code === "string"
    ? error.code
    : "";
}

export function isAutoRouteAttentionError(error: unknown) {
  const code = stableErrorCode(error);
  return (
    code.startsWith("classifier_") ||
    code.startsWith("auto_route_") ||
    code === "dynamic_routing_audit_persistence_failed"
  );
}
