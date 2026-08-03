import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import {
  GENERIC_SCOPE_LABEL_KEYS,
  genericConnectionSummaryKey,
  IntegrationsScreen,
  visibleConnectorManifests,
} from "./IntegrationsScreen";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const manifests = [
  {
    manifestId: "google_workspace",
    name: "BACKEND GOOGLE NAME CANARY",
    version: 1,
    transport: "BACKEND TRANSPORT CANARY",
    authMethod: "oauth2",
    tools: [],
    requestedPermissions: ["BACKEND PERMISSION CANARY"],
    baseScopes: [
      "https://www.googleapis.com/auth/userinfo.email",
      "https://www.googleapis.com/auth/userinfo.profile",
    ],
    operationGrants: [],
    dataDestinations: ["https://accounts.google.com"],
    projectEligible: true,
    supported: true,
  },
  {
    manifestId: "microsoft_365",
    name: "BACKEND MICROSOFT NAME CANARY",
    version: 1,
    transport: "https",
    authMethod: "oauth2_pkce",
    tools: [],
    requestedPermissions: [],
    baseScopes: ["openid", "User.Read"],
    operationGrants: [],
    dataDestinations: ["https://graph.microsoft.com"],
    projectEligible: true,
    supported: true,
  },
];

const shippedGenericScopes = [
  "https://www.googleapis.com/auth/userinfo.email",
  "https://www.googleapis.com/auth/userinfo.profile",
  "https://www.googleapis.com/auth/gmail.readonly",
  "https://www.googleapis.com/auth/gmail.compose",
  "https://www.googleapis.com/auth/calendar.readonly",
  "https://www.googleapis.com/auth/calendar.events",
  "https://www.googleapis.com/auth/drive.readonly",
  "channels:history",
  "channels:read",
  "groups:history",
  "groups:read",
  "im:history",
  "im:read",
  "mpim:history",
  "mpim:read",
  "app_mentions:read",
  "search:read",
  "chat:write",
] as const;

const googleAccount = {
  connectorId: "connector-google",
  manifestId: "google_workspace",
  accountLabel: "alex@example.test",
  grantedScopes: ["openid", "email"],
  connectionState: "authorized",
  schemaVersion: 1,
  allProjectsEnabled: false,
  projectScopeReviewedAtMs: 1,
  enabledProjectIds: [],
  capabilityGrants: [],
  dataRouting: [],
};

function localeState() {
  return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_locale_state") return localeState();
    if (command === "list_connector_manifests") return manifests;
    if (command === "list_connector_accounts") return [googleAccount];
    if (command === "list_projects") return [{ projectId: "project-1", name: "Launch", status: "active" }];
    if (command === "test_connector") return { capabilityId: "google_workspace", state: "reachable", detail: "BACKEND HEALTH CANARY", checkedAtMs: 1 };
    return null;
  });
});
afterEach(cleanup);

describe("Integrations novice surface", () => {
  it("maps service names, exposes account controls, and hides technical access details until reviewed", async () => {
    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    expect(screen.getByRole("heading", { name: "Connections" })).toBeVisible();
    expect(screen.getByText("Connect the apps OOMU can use, and see exactly what each one can access.")).toBeVisible();
    expect(screen.queryByText("Integrations")).toBeNull();
    expect(await screen.findByRole("heading", { name: "Google Workspace" })).toBeVisible();
    expect(screen.getByRole("button", { name: /Microsoft 365Use your work email, calendar, files, and Teams in selected Projects\.Available/ })).toBeVisible();
    expect(screen.queryByText("BACKEND GOOGLE NAME CANARY")).toBeNull();
    expect(screen.queryByText("BACKEND MICROSOFT NAME CANARY")).toBeNull();
    expect(screen.queryByText("BACKEND TRANSPORT CANARY")).toBeNull();
    expect(screen.queryByText("Doctor")).toBeNull();
    const googleOutcomes = screen.getAllByText("Use Gmail, Google Calendar, and Drive only with the access you approve.");
    expect(googleOutcomes).toHaveLength(2);
    googleOutcomes.forEach((outcome) => expect(outcome).toBeVisible());
    expect(screen.getByRole("button", { name: "Check connection" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Remove connection" })).toBeVisible();
    expect(screen.getByRole("checkbox", { name: /Use in all my projects/ })).not.toBeChecked();
    expect(screen.getByText("Launch")).toBeVisible();
    expect(screen.getByRole("button", { name: "Save project access" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    expect(screen.queryByRole("combobox")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
    await waitFor(() => expect(screen.getAllByText("Ready").length).toBeGreaterThan(0));
    expect(screen.queryByText("BACKEND HEALTH CANARY")).toBeNull();
  });

  it("requires confirmation and reports progress when removing a connection", async () => {
    let resolveRemove!: () => void;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return manifests;
      if (command === "list_connector_accounts") return [googleAccount];
      if (command === "list_projects") return [];
      if (command === "disconnect_connector") {
        await new Promise<void>((resolve) => { resolveRemove = resolve; });
      }
      return null;
    });
    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Remove connection" }));
    const dialog = screen.getByRole("dialog", { name: "Remove alex@example.test?" });
    expect(dialog).toBeVisible();
    expect(within(dialog).getByText(/Nothing in the connected app will be deleted/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove connection" }));
    expect(await screen.findByRole("button", { name: "Removing…" })).toHaveAttribute("aria-busy", "true");
    resolveRemove();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("Integrations connection recovery", () => {
  it("clears stale removal feedback when a fresh OAuth connection starts", async () => {
    let connected = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts") return connected ? [googleAccount] : [];
      if (command === "list_projects") return [];
      if (command === "disconnect_connector") {
        connected = false;
        return null;
      }
      if (command === "begin_connector_oauth") {
        return {
          connectorId: "replacement-google",
          authorizationUrl: "https://accounts.google.com",
          expiresAtMs: 10,
        };
      }
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Remove connection" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Remove connection" }));
    expect(await screen.findByText("Connection removed from this Mac.")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Google Workspace" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("begin_connector_oauth", {
      request: { manifestId: "google_workspace" },
    }));
    expect(screen.queryByText("Connection removed from this Mac.")).toBeNull();
  });

  it("keeps a removal failure inside the confirmation dialog", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return manifests;
      if (command === "list_connector_accounts") return [googleAccount];
      if (command === "list_projects") return [];
      if (command === "disconnect_connector") throw new Error("offline");
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Remove connection" }));
    const dialog = screen.getByRole("dialog", { name: "Remove alex@example.test?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove connection" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "OOMU could not remove this connection. It is still available on this Mac.",
    );
    expect(screen.getAllByText("OOMU could not remove this connection. It is still available on this Mac.")).toHaveLength(1);
  });

  it("allows only one OAuth start while secure sign-in is opening", async () => {
    let resolveConnect!: () => void;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts") return [];
      if (command === "list_projects") return [];
      if (command === "begin_connector_oauth") {
        await new Promise<void>((resolve) => { resolveConnect = resolve; });
        return { connectorId: "new", authorizationUrl: "https://accounts.google.com", expiresAtMs: 1 };
      }
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Connect" }));
    const continueButton = screen.getByRole("button", { name: "Continue to Google Workspace" });
    fireEvent.click(continueButton);
    fireEvent.click(continueButton);

    expect(await screen.findByRole("button", { name: "Opening secure sign-in…" })).toHaveAttribute("aria-busy", "true");
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(1);
    resolveConnect();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("Integrations OAuth callback status", () => {
  it("finishes a fresh Google connection from the exact returned connector status", async () => {
    let connected = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts") return connected ? [googleAccount] : [];
      if (command === "list_projects") return [];
      if (command === "begin_connector_oauth") {
        return {
          connectorId: googleAccount.connectorId,
          authorizationUrl: "https://accounts.google.com",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "get_connector_connection_status") {
        connected = true;
        return {
          connectorId: googleAccount.connectorId,
          connectionState: "authorized",
          grantedScopes: googleAccount.grantedScopes,
          lastProbeCode: "oauth_completed",
        };
      }
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Connect" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Google Workspace" }));

    expect(await screen.findByText("alex@example.test", {}, { timeout: 2_500 })).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith("get_connector_connection_status", {
      request: { connectorId: googleAccount.connectorId },
    });
  });

  it("surfaces a fresh disconnected Google token failure and stops waiting", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts" || command === "list_projects") return [];
      if (command === "begin_connector_oauth") {
        return {
          connectorId: "fresh-google-failure",
          authorizationUrl: "https://accounts.google.com",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "get_connector_connection_status") {
        return {
          connectorId: "fresh-google-failure",
          connectionState: "disconnected",
          grantedScopes: [],
          lastProbeCode: "google_token_client_authentication_required",
        };
      }
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Connect" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Google Workspace" }));

    expect(await screen.findByRole("alert", {}, { timeout: 2_500 })).toHaveTextContent(
      "Google sign-in isn't set up correctly in this version. Try again after updating OOMU.",
    );
    expect(invokeMock).toHaveBeenCalledWith("get_connector_connection_status", {
      request: { connectorId: "fresh-google-failure" },
    });
    expect(screen.getByRole("button", { name: "Connect" })).toBeEnabled();
  });

  it("describes configured and unhealthy accounts honestly", () => {
    expect(genericConnectionSummaryKey([{ ...googleAccount, connectionState: "configured" }])).toBe("integrations.connection_finishing");
    expect(genericConnectionSummaryKey([{ ...googleAccount, connectionState: "degraded" }])).toBe("integrations.connection_attention");
    expect(genericConnectionSummaryKey([googleAccount])).toBe("integrations.connection_ready");
  });

  it("removes Custom Tools from the production manifest projection only", () => {
    const mcp = { ...manifests[0], manifestId: "mcp_runtime" };
    expect(visibleConnectorManifests([...manifests, mcp], false).map((item) => item.manifestId))
      .not.toContain("mcp_runtime");
    expect(visibleConnectorManifests([...manifests, mcp], true).map((item) => item.manifestId))
      .toContain("mcp_runtime");
  });

});

describe("Integrations access contracts", () => {
  it("uses typed honest availability copy and never renders native diagnostics", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [{
        ...manifests[0],
        supported: false,
        availabilityReasonCode: "build_missing_oauth_client",
        name: "RAW BACKEND DIAGNOSTIC",
      }];
      if (command === "list_connector_accounts" || command === "list_projects") return [];
      return null;
    });
    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    expect(await screen.findByText("Google Workspace isn’t available in this version.")).toBeVisible();
    expect(screen.getByText(/built without the secure sign-in identity/)).toBeVisible();
    expect(screen.getByText(/There’s nothing to configure here/)).toBeVisible();
    expect(screen.queryByText(/RAW BACKEND|coming soon|needs app setup/i)).toBeNull();
  });

  it("reconnects an unhealthy account through the same connector identity", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts") return [{ ...googleAccount, connectionState: "expired" }];
      if (command === "list_projects") return [];
      if (command === "begin_connector_oauth") return { connectorId: googleAccount.connectorId, authorizationUrl: "https://accounts.google.com", expiresAtMs: 10 };
      return null;
    });
    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Reconnect" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Google Workspace" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("begin_connector_oauth", {
      request: { manifestId: "google_workspace", connectorId: googleAccount.connectorId },
    }));
  });

  it("keeps requested access behind Details and describes every scope in plain language", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") {
        return [{
          ...manifests[0],
          baseScopes: [...shippedGenericScopes.slice(0, 7), "future:scope"],
        }];
      }
      if (command === "list_connector_accounts") return [];
      if (command === "list_projects") return [];
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Connect" }));

    const dialog = screen.getByRole("dialog", { name: "Review Google Workspace access" });
    expect(dialog).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(within(dialog).getByText("Identify your Google account")).not.toBeVisible();
    expect(within(dialog).getByText("Additional access: Unknown")).not.toBeVisible();
    fireEvent.click(within(dialog).getByText("Details"));
    expect(screen.getByText("Identify your Google account")).toBeVisible();
    expect(screen.getByText("Read Gmail")).toBeVisible();
    expect(screen.getByText("Create Gmail drafts")).toBeVisible();
    expect(screen.getByText("Read Google Calendar")).toBeVisible();
    expect(screen.getByText("Change Google Calendar")).toBeVisible();
    expect(screen.getByText("Search and read Google Drive")).toBeVisible();
    expect(screen.getByText("Additional access: Unknown")).toBeVisible();
    expect(screen.queryByText(/future:scope|https:\/\//)).toBeNull();
    expect(screen.queryByText("https://www.googleapis.com/auth/gmail.readonly")).toBeNull();
  });

  it("closes a connector access review with Escape and restores focus", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [manifests[0]];
      if (command === "list_connector_accounts") return [];
      if (command === "list_projects") return [];
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });
    const connectButton = await screen.findByRole("button", { name: "Connect" });
    connectButton.focus();
    fireEvent.click(connectButton);
    const dialog = screen.getByRole("dialog", { name: "Review Google Workspace access" });
    fireEvent.keyDown(dialog, { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(connectButton).toHaveFocus();
  });

  it("keeps the frontend scope labels in lockstep with the shipped Rust manifests", () => {
    const source = readFileSync("src-tauri/src/connectors/manifest.rs", "utf8");
    const blocks = ["GOOGLE_BASE_SCOPES", "SLACK_READ_SCOPES", "SLACK_MESSAGING_BOT_SCOPES"].map((name) => {
      const match = source.match(new RegExp(`const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`));
      expect(match, `${name} should remain discoverable`).not.toBeNull();
      return match?.[1] ?? "";
    });
    const rustScopes = blocks.flatMap((block) =>
      [...block.matchAll(/"([^"]+)"/g)].map((match) => match[1]),
    );
    for (const name of [
      "GOOGLE_EMAIL_SCOPE",
      "GOOGLE_PROFILE_SCOPE",
      "GOOGLE_GMAIL_READ",
      "GOOGLE_GMAIL_DRAFT",
      "GOOGLE_CALENDAR_READ",
      "GOOGLE_CALENDAR_WRITE",
      "GOOGLE_DRIVE_READ",
    ]) {
      const match = source.match(new RegExp(`const ${name}: &str = "([^"]+)";`));
      expect(match, `${name} should remain discoverable`).not.toBeNull();
      if (match?.[1]) rustScopes.push(match[1]);
    }
    rustScopes.push("openid", "email", "profile");

    expect(Object.keys(GENERIC_SCOPE_LABEL_KEYS).sort()).toEqual([...new Set(rustScopes)].sort());
  });
});

describe("Slack messaging tier", () => {
  it("shows Slack's shared messaging tier and opens its upgrade path", async () => {
    const onTurnOnMessaging = vi.fn();
    const slackManifest = {
      ...manifests[0],
      manifestId: "slack",
      baseScopes: ["channels:read", "search:read"],
      dataDestinations: ["https://slack.com"],
    };
    const slackAccount = {
      ...googleAccount,
      connectorId: "connector-slack",
      manifestId: "slack",
      accountLabel: "Acme",
      grantedScopes: ["channels:read", "search:read"],
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [slackManifest];
      if (command === "list_connector_accounts") return [slackAccount];
      if (command === "list_projects") return [];
      return null;
    });

    render(
      <IntegrationsScreen onTurnOnMessaging={onTurnOnMessaging} />,
      { wrapper: I18nProvider },
    );

    expect(await screen.findByText(/can read approved Slack conversations and prepare messages/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Turn on messaging" }));
    expect(onTurnOnMessaging).toHaveBeenCalledTimes(1);
  });

  it("keeps Slack work access available when real-time messaging is unavailable", async () => {
    const slackManifest = {
      ...manifests[0],
      manifestId: "slack",
      baseScopes: ["channels:read", "search:read"],
      dataDestinations: ["https://slack.com"],
      operationGrants: [{
        operation: "slack.messaging",
        available: false,
        unavailableReasonCode: "build_missing_oauth_broker",
      }],
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_manifests") return [slackManifest];
      if (command === "list_connector_accounts" || command === "list_projects") return [];
      if (command === "begin_connector_oauth") {
        return { connectorId: "connector-slack", authorizationUrl: "https://slack.com", expiresAtMs: 10 };
      }
      return null;
    });

    render(<IntegrationsScreen />, { wrapper: I18nProvider });

    expect(await screen.findByText("Available")).toBeVisible();
    expect(screen.queryByText(/isn’t available in this version/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Slack" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("begin_connector_oauth", {
      request: { manifestId: "slack" },
    }));
  });
});
