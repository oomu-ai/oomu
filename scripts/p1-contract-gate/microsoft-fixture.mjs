const MICROSOFT_OPERATION_FIXTURE = {
  "outlook.mail.search": ["read", ["Mail.Read"], false],
  "outlook.mail.read": ["read", ["Mail.Read"], false],
  "outlook.mail.draft": ["draft_write", ["Mail.ReadWrite"], true],
  "outlook.calendar.read": ["read", ["Calendars.Read"], false],
  "outlook.calendar.draft_event": ["local_draft", [], false],
  "onedrive.file.search": ["read", ["Files.Read"], false],
  "onedrive.file.read": ["read", ["Files.Read"], false],
  "onedrive.file.write": ["write", ["Files.ReadWrite"], true],
  "sharepoint.file.search": ["tenant_read", ["Sites.Read.All"], false],
  "sharepoint.file.read": ["tenant_read", ["Sites.Read.All"], false],
  "sharepoint.file.write": ["tenant_write", ["Sites.ReadWrite.All"], true],
  "teams.chat.search": ["read", ["Chat.Read"], false],
  "teams.chat.draft_message": ["local_draft", [], false],
};

function sameMembers(actual, expected) {
  const left = [...new Set(actual)].sort();
  const right = [...new Set(expected)].sort();
  return left.length === right.length && left.every(
    (value, index) => value === right[index],
  );
}

function hasExactKeys(value, required, optional = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return required.every((key) => keys.includes(key)) && keys.every(
    (key) => required.includes(key) || optional.includes(key),
  );
}

export function validateMicrosoftImplementationFixture(fixture) {
  const failures = [];
  if (!hasExactKeys(fixture, ["schemaVersion", "sprint", "qualificationStatus", "accounts"]) || fixture.schemaVersion !== 1 || fixture.sprint !== 234 || fixture.qualificationStatus !== "contract_tested_not_real_tenant" || !Array.isArray(fixture.accounts) || fixture.accounts.length < 2) {
    return ["Microsoft fixture must be the strict Sprint 234 account DTO envelope"];
  }
  const accountKeys = [
    "connectorId", "manifestId", "accountLabel", "grantedScopes", "connectionState", "schemaVersion",
    "tokenExpiresAtMs", "lastProbeAtMs", "lastProbeCode", "enabledProjectIds", "identityBindingHash",
    "tenantId", "tenantLabel", "accountId", "accountPrincipal", "accountKind", "capabilityGrants",
    "dataRouting", "consentReviewedAtMs", "identityVerifiedAtMs",
  ];
  for (const account of fixture.accounts) {
    if (!hasExactKeys(account, accountKeys) || account.schemaVersion !== 1 || account.manifestId !== "microsoft_365" || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(account.tenantId ?? "")) {
      failures.push("Microsoft fixture account does not match the current tenant-bound ConnectorAccount DTO");
      continue;
    }
    if (!["personal", "work", "school"].includes(account.accountKind) || !Array.isArray(account.grantedScopes) || !Array.isArray(account.capabilityGrants)) {
      failures.push(`${account.connectorId}: Microsoft account kind, scopes, and grants are required`);
      continue;
    }
    if (account.dataRouting.length !== 5 || !sameMembers(account.dataRouting, ["https://login.microsoftonline.com", "https://graph.microsoft.com", "https://*.sharepoint.com", "https://*.sharepointonline.com", "https://*.1drv.com"])) {
      failures.push(`${account.connectorId}: Microsoft routing must contain the exact production URLs`);
    }
    if (account.capabilityGrants.length !== 13 || !sameMembers(account.capabilityGrants.map((grant) => grant.capabilityId), Object.keys(MICROSOFT_OPERATION_FIXTURE))) {
      failures.push(`${account.connectorId}: capability grants must contain exactly the 13 production operation IDs`);
      continue;
    }
    for (const grant of account.capabilityGrants) {
      if (!hasExactKeys(grant, ["capabilityId", "accessLevel", "requiredScopes", "granted", "adminConsentRequired", "remoteMutation", "available", "unavailableReasonCode"])) {
        failures.push(`${account.connectorId}: capability grant has an unknown or missing DTO field`);
        continue;
      }
      const [accessLevel, scopes, remoteMutation] = MICROSOFT_OPERATION_FIXTURE[grant.capabilityId];
      const workOnly = grant.capabilityId.startsWith("sharepoint.") || grant.capabilityId.startsWith("teams.");
      const available = !(account.accountKind === "personal" && workOnly);
      const granted = available && scopes.every((scope) => account.grantedScopes.includes(scope));
      const unavailableReason = available ? null : "microsoft_capability_work_account_required";
      if (
        grant.accessLevel !== accessLevel ||
        !sameMembers(grant.requiredScopes, scopes) ||
        grant.adminConsentRequired !== false ||
        grant.remoteMutation !== remoteMutation ||
        grant.available !== available ||
        grant.granted !== granted ||
        grant.unavailableReasonCode !== unavailableReason
      ) {
        failures.push(`${account.connectorId}: ${grant.capabilityId} contradicts the production operation grant`);
      }
    }
  }
  if (!fixture.accounts.some((account) => account.accountKind === "personal") || !fixture.accounts.some((account) => ["work", "school"].includes(account.accountKind))) {
    failures.push("Microsoft fixture must exercise both personal and work-or-school availability semantics");
  }
  return failures;
}
