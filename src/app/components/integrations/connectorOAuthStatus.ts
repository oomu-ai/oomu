import type { ConnectorConnectionStatus } from "./integrationClient";

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

const READY_CONNECTION_STATES = new Set(["authorized", "reachable"]);

export function oauthConnectionOutcome(
  status: ConnectorConnectionStatus,
): "pending" | "ready" | "failed" {
  if (READY_CONNECTION_STATES.has(status.connectionState)) return "ready";
  return status.connectionState === "configured" ? "pending" : "failed";
}

export function localizedOAuthFailure(
  status: ConnectorConnectionStatus,
  t: TranslateFn,
): string {
  switch (status.lastProbeCode) {
    case "oauth_expired":
      return t("setup.errors.setup_oauth_expired");
    case "google_token_access_denied":
      return t("setup.errors.setup_google_oauth_access_denied");
    case "google_token_invalid_client":
    case "google_token_invalid_request":
    case "google_token_client_authentication_required":
      return t("setup.errors.setup_google_oauth_configuration_failed");
    case "google_token_rate_limited":
    case "google_token_unavailable":
      return t("setup.errors.setup_google_oauth_temporarily_unavailable");
    case "oauth_broker_unreachable":
      return t("integrations.oauth_broker.unreachable");
    case "oauth_broker_unconfigured":
    case "oauth_broker_identity_unavailable":
      return t("connector_availability.build_missing_oauth_broker.title", {
        service: t("setup.connector_slack_name"),
      });
    default:
      return t("setup.errors.setup_connector_failed");
  }
}
