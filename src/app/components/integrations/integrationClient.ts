import { invoke } from "@/lib/invoke";

type ConnectorTool = { name: string; risk: string; description: string; inputSchema: Record<string, unknown>; outputSchema?: Record<string, unknown> };
export type ConnectorCapabilityGrant = { capabilityId: string; accessLevel: string; requiredScopes: string[]; granted: boolean; adminConsentRequired: boolean; remoteMutation: boolean; available: boolean; unavailableReasonCode?: string | null };
type ConnectorOperationGrant = { operation: string; purposeCode: string; accessLevel: string; requiredScopes: string[]; adminConsentRequired: boolean; remoteMutation: boolean; available?: boolean; unavailableReasonCode?: string | null };
export type ConnectorManifest = { manifestId: string; name: string; version: number; transport: string; authMethod: string; tools: ConnectorTool[]; requestedPermissions: string[]; dataDestinations: string[]; projectEligible: boolean; supported: boolean; availabilityReasonCode?: string; baseScopes?: string[]; operationGrants?: ConnectorOperationGrant[] };
export type ConnectorAccount = {
  connectorId: string;
  manifestId: string;
  accountLabel: string;
  grantedScopes: string[];
  connectionState: string;
  schemaVersion: number;
  tokenExpiresAtMs?: number;
  lastProbeAtMs?: number;
  lastProbeCode?: string;
  allProjectsEnabled: boolean;
  projectScopeReviewedAtMs?: number | null;
  enabledProjectIds: string[];
  identityBindingHash?: string;
  tenantId?: string;
  tenantLabel?: string;
  accountId?: string;
  accountKind?: string;
  accountPrincipal?: string;
  capabilityGrants?: ConnectorCapabilityGrant[];
  dataRouting?: string[];
  consentReviewedAtMs?: number;
  identityVerifiedAtMs?: number;
};
export type ConnectorProjectScope = { connectorId: string; allProjectsEnabled: boolean; enabledProjectIds: string[]; projectScopeReviewedAtMs: number; updatedAtMs: number };
export type SlackConversation = { id: string; name?: string; kind: "channel" | "private_channel" | "group_message" | "direct_message" };
export type ConnectorConnectionStatus = { connectorId: string; connectionState: string; grantedScopes: string[]; lastProbeAtMs?: number; lastProbeCode?: string };
export type CapabilityHealth = { capabilityId: string; state: string; detail: string; repairAction?: string; checkedAtMs: number };
export type SetupState = { currentStep: string; modelPath?: string; completionChannel?: string; sampleProjectId?: string; completedAtMs?: number };
export type RunSetupSampleOptions = { completeSetup?: boolean };

export const integrationApi = {
  manifests: () => invoke<ConnectorManifest[]>("list_connector_manifests"),
  accounts: () => invoke<ConnectorAccount[]>("list_connector_accounts"),
  connectionStatus: (connectorId: string) => invoke<ConnectorConnectionStatus>("get_connector_connection_status", { request: { connectorId } }),
  slackConversations: (connectorId: string) => invoke<SlackConversation[]>("list_slack_conversations", { request: { connectorId } }),
  connect: (manifestId: string, connectorId?: string, requestedOperations?: string[]) => invoke<{ connectorId: string; authorizationUrl: string; expiresAtMs: number; requestedScopes?: string[] }>("begin_connector_oauth", { request: { manifestId, ...(connectorId ? { connectorId } : {}), ...(requestedOperations?.length ? { requestedOperations } : {}) } }),
  setProjectScope: (connectorId: string, allProjectsEnabled: boolean, enabledProjectIds: string[]) => invoke<ConnectorProjectScope>("set_connector_project_scope", { request: { connectorId, allProjectsEnabled, enabledProjectIds } }),
  test: (connectorId: string) => invoke<CapabilityHealth>("test_connector", { request: { connectorId } }),
  disconnect: (connectorId: string) => invoke<void>("disconnect_connector", { request: { connectorId } }),
  health: () => invoke<CapabilityHealth[]>("get_capability_health"),
  setup: () => invoke<SetupState>("get_setup_state"),
  saveSetup: (currentStep: string, modelPath?: string, completionChannel?: string) => invoke<SetupState>("save_setup_progress", { request: { currentStep, modelPath, completionChannel } }),
  runSample: (modelRoute: string, options?: RunSetupSampleOptions) => invoke<SetupState>("run_setup_sample_task", { request: { modelRoute, ...(options?.completeSetup === undefined ? {} : { completeSetup: options.completeSetup }) } }),
};
