import { invoke } from "@/lib/invoke";
import type { ConnectorProjectScope } from "../integrationClient";

const MICROSOFT365_COMMANDS = {
  accounts: "list_connector_accounts",
  beginOauth: "begin_connector_oauth",
  projectScope: "set_connector_project_scope",
  health: "test_connector",
  disconnect: "disconnect_connector",
} as const;

const MICROSOFT365_MANIFEST_IDS = [
  "microsoft_365",
  "microsoft365",
  "microsoft_graph",
] as const;

type Microsoft365HealthState =
  | "healthy"
  | "reachable"
  | "degraded"
  | "partial"
  | "offline"
  | "expired"
  | "revoked"
  | "rate_limited"
  | "tenant_policy"
  | "unavailable";

export type Microsoft365CapabilityGrant = {
  capabilityId: string;
  accessLevel: "read" | "draft" | "write" | "send" | "share" | string;
  requiredScopes: string[];
  granted: boolean;
  adminConsentRequired: boolean;
  remoteMutation: boolean;
  available: boolean;
  unavailableReasonCode?: string | null;
};

export type Microsoft365Account = {
  connectorId: string;
  manifestId: string;
  accountLabel: string;
  grantedScopes: string[];
  connectionState: Microsoft365HealthState | string;
  schemaVersion: number;
  tokenExpiresAtMs?: number | null;
  lastProbeAtMs?: number | null;
  lastProbeCode?: string | null;
  allProjectsEnabled: boolean;
  projectScopeReviewedAtMs?: number | null;
  enabledProjectIds: string[];
  identityBindingHash?: string | null;
  tenantId?: string | null;
  tenantLabel?: string | null;
  accountId?: string | null;
  accountKind?: "personal" | "work" | "school" | string;
  accountPrincipal?: string | null;
  capabilityGrants?: Microsoft365CapabilityGrant[];
  dataRouting?: string[];
  consentReviewedAtMs?: number | null;
  identityVerifiedAtMs?: number | null;
};

export type Microsoft365Health = {
  capabilityId: string;
  state: Microsoft365HealthState | string;
  detail: string;
  repairAction?: string;
  detailCode?: string;
  repairActionCode?: string;
  checkedAtMs: number;
};

type Microsoft365OauthResponse = {
  connectorId: string;
  authorizationUrl: string;
  expiresAtMs: number;
  requestedScopes?: string[];
};

export function isMicrosoft365Manifest(manifestId: string) {
  const normalized = manifestId.toLowerCase();
  return MICROSOFT365_MANIFEST_IDS.includes(
    normalized as (typeof MICROSOFT365_MANIFEST_IDS)[number],
  ) || normalized.includes("microsoft");
}

function microsoftAccounts(accounts: Microsoft365Account[]) {
  return accounts.filter((account) => isMicrosoft365Manifest(account.manifestId));
}

export const microsoft365Api = {
  accounts: async () =>
    microsoftAccounts(
      await invoke<Microsoft365Account[]>(MICROSOFT365_COMMANDS.accounts),
    ),
  beginOauth: (
    manifestId: string,
    options: { connectorId?: string; requestedOperations?: string[] } = {},
  ) =>
    invoke<Microsoft365OauthResponse>(MICROSOFT365_COMMANDS.beginOauth, {
      request: {
        manifestId,
        ...(options.connectorId ? { connectorId: options.connectorId } : {}),
        ...(options.requestedOperations?.length
          ? { requestedOperations: options.requestedOperations }
          : {}),
      },
    }),
  setProjectScope: (connectorId: string, allProjectsEnabled: boolean, enabledProjectIds: string[]) =>
    invoke<ConnectorProjectScope>(MICROSOFT365_COMMANDS.projectScope, {
      request: { connectorId, allProjectsEnabled, enabledProjectIds },
    }),
  test: async (connectorId: string) => {
    const raw = await invoke<Microsoft365Health & {
      detailCode?: string;
      repairActionCode?: string;
    }>(MICROSOFT365_COMMANDS.health, { request: { connectorId } });
    return {
      capabilityId: raw.capabilityId,
      state: raw.state,
      detail: raw.detail,
      repairAction: raw.repairAction,
      detailCode: raw.detailCode,
      repairActionCode: raw.repairActionCode,
      checkedAtMs: raw.checkedAtMs,
    } satisfies Microsoft365Health;
  },
  disconnect: (connectorId: string) =>
    invoke<void>(MICROSOFT365_COMMANDS.disconnect, {
      request: { connectorId },
    }),
};
