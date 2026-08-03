import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { ProjectRecord } from "../../projects/projectClient";
import type { ConnectorManifest } from "../integrationClient";
import { Microsoft365ControlPanel } from "./Microsoft365ControlPanel";
import type { Microsoft365Account } from "./microsoft365Client";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const project = {
  projectId: "project_22222222-2222-4222-8222-222222222222",
  name: "Quarterly review",
  description: "",
  dataPolicy: "ask_before_cloud",
  instructions: "",
  archivedAtMs: null,
  createdAtMs: 1,
  updatedAtMs: 1,
  sourceCount: 0,
  conversationCount: 0,
  workflowCount: 0,
  taskCount: 1,
} as ProjectRecord;

const manifest: ConnectorManifest = {
  manifestId: "microsoft_365",
  name: "BACKEND NAME CANARY",
  version: 1,
  transport: "BACKEND TRANSPORT CANARY",
  authMethod: "oauth_authorization_code_pkce",
  tools: [],
  requestedPermissions: ["BACKEND PERMISSION CANARY"],
  dataDestinations: ["https://login.microsoftonline.com", "https://graph.microsoft.com"],
  projectEligible: true,
  supported: true,
  baseScopes: ["openid", "profile", "email", "offline_access", "User.Read"],
  operationGrants: [{
    operation: "onedrive.file.read",
    purposeCode: "onedrive_file_read",
    accessLevel: "read",
    requiredScopes: ["Files.Read"],
    adminConsentRequired: false,
    remoteMutation: false,
  }],
};

const account: Microsoft365Account = {
  connectorId: "connector_11111111-1111-4111-8111-111111111111",
  manifestId: "microsoft_365",
  accountLabel: "alex@example.test",
  accountPrincipal: "alex@example.test",
  accountKind: "work",
  tenantId: "11111111-2222-4333-8444-555555555555",
  tenantLabel: "Example Company",
  accountId: "account",
  identityBindingHash: "a".repeat(64),
  grantedScopes: ["openid", "profile", "email", "offline_access", "User.Read"],
  connectionState: "authorized",
  schemaVersion: 1,
  lastProbeAtMs: 100,
  lastProbeCode: "oauth_completed",
  allProjectsEnabled: false,
  projectScopeReviewedAtMs: 100,
  enabledProjectIds: [project.projectId],
  capabilityGrants: [{
    capabilityId: "onedrive.file.read",
    accessLevel: "read",
    requiredScopes: ["Files.Read"],
    granted: false,
    adminConsentRequired: false,
    remoteMutation: false,
    available: true,
  }],
  dataRouting: ["https://graph.microsoft.com"],
};

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }],
    translations: {},
  };
}

function renderPanel(nextManifest = manifest) {
  return render(<Microsoft365ControlPanel manifest={nextManifest} projects={[project]} />, { wrapper: I18nProvider });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_locale_state") return localeState();
    if (command === "list_connector_accounts") return [account];
    return null;
  });
});
afterEach(cleanup);

describe("Microsoft365ControlPanel account access", () => {
  it("shows purpose, readiness, Project access, and one next action before technical details", async () => {
    renderPanel();
    expect(await screen.findByRole("heading", { name: "Microsoft 365" })).toBeVisible();
    expect(screen.getByText("Use your work email, calendar, files, and Teams in selected Projects.")).toBeVisible();
    expect(screen.getByText("This account is ready.")).toBeVisible();
    expect(screen.getByText("Quarterly review")).toBeVisible();
    expect(screen.getByRole("button", { name: "Review access" })).toBeVisible();
    expect(screen.getByText("Account details")).not.toBeVisible();
    expect(screen.queryByText("BACKEND NAME CANARY")).toBeNull();
    expect(screen.queryByText("BACKEND PERMISSION CANARY")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    expect(screen.getByText("Account details")).toBeVisible();
    expect(screen.getByText("Where data goes")).toBeVisible();
  });

  it("reviews one capability's exact outgoing scope union and cancellation invokes nothing", async () => {
    renderPanel();
    await screen.findByText("This account is ready.");
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    fireEvent.click(screen.getByRole("button", { name: "Add access" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(within(dialog).getByText("Identify this Microsoft account")).not.toBeVisible();
    expect(within(dialog).getByText("Read OneDrive files")).not.toBeVisible();
    expect(within(dialog).getByText("Microsoft sign-in")).not.toBeVisible();
    expect(within(dialog).getByText("Microsoft 365 services")).not.toBeVisible();
    for (const scope of ["openid", "profile", "email", "offline_access", "User.Read", "Files.Read"]) {
      expect(within(dialog).queryByText(scope)).toBeNull();
    }
    expect(within(dialog).queryByText("https://graph.microsoft.com")).toBeNull();
    fireEvent.click(within(dialog).getByText("Details"));
    expect(within(dialog).getByText("Identify this Microsoft account")).toBeVisible();
    expect(within(dialog).getByText("Read OneDrive files")).toBeVisible();
    expect(within(dialog).getByText("Microsoft sign-in")).toBeVisible();
    expect(within(dialog).getByText("Microsoft 365 services")).toBeVisible();
    for (const scope of ["openid", "profile", "email", "offline_access", "User.Read", "Files.Read"]) {
      expect(within(dialog).queryByText(scope)).toBeNull();
    }
    expect(within(dialog).queryByText("https://graph.microsoft.com")).toBeNull();
    expect(within(dialog).queryByText("Mail.Read")).toBeNull();
    expect(within(dialog).queryByText("Sites.Read.All")).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(0);
  });

  it("keeps an empty tenant label diagnosable inside Details without leaking it onto the novice surface", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_accounts") return [{ ...account, tenantLabel: "" }];
      return null;
    });
    renderPanel();
    await screen.findByText("This account is ready.");
    expect(screen.getByText("Work or school organization")).not.toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    expect(screen.getByText("Work or school organization")).toBeVisible();
    expect(screen.getByText("Organization ID")).toBeVisible();
    expect(screen.getByText("11111111-2222-4333-8444-555555555555")).toBeVisible();
  });

  it("removes a connected account only after confirmation and refreshes the local account state", async () => {
    let connected = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_accounts") return connected ? [account] : [];
      if (command === "disconnect_connector") { connected = false; return null; }
      return null;
    });
    renderPanel();
    await screen.findByText("This account is ready.");
    expect(screen.getByRole("button", { name: "Remove account" })).not.toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove account" }));
    const dialog = screen.getByRole("dialog", { name: "Remove this Microsoft 365 account?" });
    expect(within(dialog).getByText("alex@example.test")).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "disconnect_connector")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove account" }));
    await waitFor(() => expect(screen.getByText("No Microsoft 365 account is connected.")).toBeVisible());
    expect(invokeMock.mock.calls.filter(([command]) => command === "disconnect_connector")).toHaveLength(1);
  });
});

describe("Microsoft365ControlPanel connection recovery", () => {
  it("never offers OAuth when the Microsoft app identity is unavailable", async () => {
    invokeMock.mockImplementation(async (command: string) => command === "get_locale_state" ? localeState() : []);
    renderPanel({ ...manifest, supported: false, availabilityReasonCode: "build_missing_oauth_client" });
    expect(await screen.findByText("Microsoft 365 isn’t available in this version.")).toBeVisible();
    expect(screen.getByText(/There’s nothing to configure here/)).toBeVisible();
    expect(screen.queryByText("BACKEND CANARY")).toBeNull();
    expect(screen.queryByRole("button", { name: "Connect Microsoft 365" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Continue to Microsoft" })).toBeNull();
  });

  it("offers an obvious retry when account loading fails", async () => {
    let reads = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_accounts") {
        reads += 1;
        if (reads === 1) throw new Error("offline");
        return [];
      }
      return null;
    });
    renderPanel();
    expect(await screen.findByText("Microsoft 365 is not available right now. No account access is being assumed.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("No Microsoft 365 account is connected.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect Microsoft 365" })).toBeVisible();
    expect(reads).toBe(2);
  });

  it("polls the returned account until OAuth reaches a terminal state", async () => {
    let resolvePoll!: (value: Microsoft365Account[]) => void;
    const poll = new Promise<Microsoft365Account[]>((resolve) => { resolvePoll = resolve; });
    let reads = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_accounts") { reads += 1; return reads === 1 ? [] : poll; }
      if (command === "begin_connector_oauth") return { connectorId: account.connectorId, authorizationUrl: "https://login.microsoftonline.com", expiresAtMs: Date.now() + 60_000, requestedScopes: manifest.baseScopes };
      return null;
    });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Connect Microsoft 365" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue to Microsoft" }));
    expect(await screen.findByText("Finishing connection…")).toBeVisible();
    await act(async () => { resolvePoll([{ ...account, lastProbeAtMs: Date.now() + 1, lastProbeCode: "oauth_completed" }]); await poll; });
    expect(await screen.findByText("This account is ready.")).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(1);
  });

  it("keeps the consent dialog locked while Microsoft sign-in is starting", async () => {
    let resolveOauth!: (value: {
      connectorId: string;
      authorizationUrl: string;
      expiresAtMs: number;
      requestedScopes: string[];
    }) => void;
    const oauth = new Promise<{
      connectorId: string;
      authorizationUrl: string;
      expiresAtMs: number;
      requestedScopes: string[];
    }>((resolve) => { resolveOauth = resolve; });
    let accountReads = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_connector_accounts") {
        accountReads += 1;
        return accountReads === 1
          ? []
          : [{ ...account, lastProbeAtMs: Date.now() + 1_000, lastProbeCode: "oauth_completed" }];
      }
      if (command === "begin_connector_oauth") return oauth;
      return null;
    });

    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Connect Microsoft 365" }));
    const dialog = screen.getByRole("dialog");
    const continueButton = within(dialog).getByRole("button", { name: "Continue to Microsoft" });
    fireEvent.click(continueButton);
    fireEvent.click(continueButton);

    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Continue to Microsoft" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Continue to Microsoft" })).toHaveAttribute("aria-busy", "true");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(1);

    await act(async () => {
      resolveOauth({
        connectorId: account.connectorId,
        authorizationUrl: "https://login.microsoftonline.com",
        expiresAtMs: Date.now() + 60_000,
        requestedScopes: manifest.baseScopes ?? [],
      });
      await oauth;
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(1);
  });

  it("does not treat an existing authorized account as completed incremental consent", async () => {
    renderPanel();
    await screen.findByText("This account is ready.");
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    fireEvent.click(screen.getByRole("button", { name: "Add access" }));
    vi.useFakeTimers();
    try {
      let reads = 0;
      invokeMock.mockImplementation(async (command: string) => {
        if (command === "get_locale_state") return localeState();
        if (command === "list_connector_accounts") {
          reads += 1;
          return reads < 2 ? [account] : [{ ...account, lastProbeAtMs: Date.now() + 1, lastProbeCode: "oauth_completed" }];
        }
        if (command === "begin_connector_oauth") return { connectorId: account.connectorId, authorizationUrl: "https://login.microsoftonline.com", expiresAtMs: Date.now() + 60_000, requestedScopes: [...account.grantedScopes, "Files.Read"] };
        return null;
      });
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Continue to Microsoft" })); await Promise.resolve(); });
      expect(screen.getByText("Finishing connection…")).toBeVisible();
      expect(reads).toBe(1);
      await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
      expect(screen.queryByText("Finishing connection…")).toBeNull();
      expect(reads).toBe(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
