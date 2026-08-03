import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChannelsDashboard } from "../ChannelsDashboard";

const invokeMock = vi.hoisted(() => vi.fn());
let connectionReceipt = {
  connectorId: "slack-workspace-1",
  connectionState: "configured",
  grantedScopes: [] as string[],
  lastProbeAtMs: Date.now(),
  lastProbeCode: "oauth_started",
};

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

const statuses = [
  {
    platform: "telegram",
    label: "Telegram",
    isActive: false,
    connectionState: "inactive",
    ownerId: null,
    workerState: "idle",
    lastCheckedAtMs: null,
    detail: null,
  },
  {
    platform: "discord",
    label: "Discord",
    isActive: false,
    connectionState: "inactive",
    ownerId: null,
    workerState: "idle",
    lastCheckedAtMs: null,
    detail: null,
  },
  {
    platform: "slack",
    label: "Slack",
    isActive: false,
    connectionState: "inactive",
    ownerId: null,
    workerState: "idle",
    lastCheckedAtMs: null,
    detail: null,
  },
];

const readOnlySlack = {
  connectorId: "slack-workspace-1",
  manifestId: "slack",
  accountLabel: "Acme",
  grantedScopes: ["channels:read", "search:read"],
  connectionState: "authorized",
  schemaVersion: 1,
  allProjectsEnabled: true,
  enabledProjectIds: [],
};

const messagingSlack = {
  ...readOnlySlack,
  grantedScopes: [...readOnlySlack.grantedScopes, "chat:write"],
  accountId: "U123OWNER",
};

function renderDashboard() {
  return render(<I18nProvider><ChannelsDashboard /></I18nProvider>);
}

function installInvoke(accounts: unknown[] = [], nextStatuses = statuses) {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_channel_statuses") return nextStatuses;
    if (command === "list_connector_accounts") return accounts;
    if (command === "list_connector_manifests") return [{
      manifestId: "slack",
      name: "Slack",
      supported: true,
      operationGrants: [{ operation: "slack.messaging", available: true }],
    }];
    if (command === "get_connector_connection_status") return connectionReceipt;
    if (command === "list_slack_conversations") return [
      { id: "C123ALLOWED", name: "general", kind: "channel" },
      { id: "D123OWNER", kind: "direct_message" },
    ];
    if (command === "begin_connector_oauth") {
      return {
        connectorId: readOnlySlack.connectorId,
        authorizationUrl: "https://slack.com/oauth/v2/authorize",
        expiresAtMs: Date.now() + 60_000,
      };
    }
    return undefined;
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  connectionReceipt = {
    connectorId: "slack-workspace-1",
    connectionState: "configured",
    grantedScopes: [],
    lastProbeAtMs: Date.now(),
    lastProbeCode: "oauth_started",
  };
  installInvoke();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("ChannelsDashboard", () => {
  it("gives the introductory sentence a comfortable desktop measure", () => {
    renderDashboard();

    const subtitle = screen.getByText(
      "Choose where you want to talk with OOMU. Only the people and conversations you approve can reach it.",
    );
    expect(subtitle.closest("header")).toHaveClass("max-w-3xl");
  });

  it("shows exactly the three first-party messaging platforms", async () => {
    renderDashboard();

    expect(await screen.findByRole("heading", { name: "Telegram" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Discord" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Slack" })).toBeVisible();
    expect(screen.getAllByRole("article")).toHaveLength(3);
    expect(screen.queryByText(/Signal|WhatsApp/i)).toBeNull();
  });

  it("isolates unavailable real-time messaging without disabling Slack work access", async () => {
    installInvoke();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_channel_statuses") return statuses;
      if (command === "list_connector_accounts") return [];
      if (command === "list_connector_manifests") return [{
        manifestId: "slack",
        name: "Slack",
        supported: true,
        operationGrants: [{
          operation: "slack.messaging",
          available: false,
          unavailableReasonCode: "build_missing_oauth_broker",
        }],
      }];
      return undefined;
    });
    renderDashboard();

    expect(await screen.findByText("Real-time Slack messaging isn’t available in this version.")).toBeVisible();
    expect(screen.getByText(/still connect Slack under Apps OOMU can use/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Install OOMU in Slack" })).toBeNull();
  });
});

describe("ChannelsDashboard Slack authorization", () => {
  it("starts Slack messaging installation with the messaging operation", async () => {
    const user = userEvent.setup();
    renderDashboard();

    await user.click(await screen.findByRole("button", { name: "Install OOMU in Slack" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("begin_connector_oauth", {
      request: { manifestId: "slack", requestedOperations: ["slack.messaging"] },
    }));
    expect(screen.getByText("Finish the secure Slack sign-in, then return here.")).toBeVisible();
  });

  it("upgrades a read-only Slack connection through the same connector identity", async () => {
    const user = userEvent.setup();
    installInvoke([readOnlySlack]);
    renderDashboard();

    expect(await screen.findByText("Connected for reading")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Turn on messaging" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("begin_connector_oauth", {
      request: {
        manifestId: "slack",
        connectorId: readOnlySlack.connectorId,
        requestedOperations: ["slack.messaging"],
      },
    }));
  });

  it("explains a canceled Slack install instead of leaving sign-in pending", async () => {
    const user = userEvent.setup();
    connectionReceipt = {
      connectorId: "slack-workspace-1",
      connectionState: "disconnected",
      grantedScopes: [],
      lastProbeAtMs: Date.now() + 1_000,
      lastProbeCode: "slack_authorization_access_denied",
    };
    renderDashboard();

    await user.click(await screen.findByRole("button", { name: "Install OOMU in Slack" }));

    expect(await screen.findByText(
      "Slack sign-in was canceled. Nothing changed. Try again whenever you’re ready.",
    )).toBeVisible();
  });

  it("saves the approved Slack owner and conversation allowlist without a pasted token", async () => {
    const user = userEvent.setup();
    installInvoke([messagingSlack]);
    renderDashboard();

    const slackCard = (await screen.findByRole("heading", { name: "Slack" })).closest("article");
    expect(slackCard).not.toBeNull();
    await user.click(await within(slackCard!).findByRole("button", { name: "Configure" }));

    const dialog = screen.getByRole("dialog", { name: "Set up Slack" });
    expect(within(dialog).queryByLabelText(/token/i)).toBeNull();
    expect(within(dialog).getByLabelText("Approved Slack user")).toHaveValue("U123OWNER");
    await user.click(await within(dialog).findByRole("checkbox", { name: /#general/ }));
    await user.click(within(dialog).getByRole("checkbox", { name: /Direct message/ }));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("save_channel_config", {
      request: {
        platform: "slack",
        isActive: true,
        credentialsJson: JSON.stringify({
          connectorId: messagingSlack.connectorId,
          allowlistChannels: ["C123ALLOWED", "D123OWNER"],
        }),
        ownerId: "U123OWNER",
      },
    }));
  });

  it("keeps Telegram token drafts private across cancellation", async () => {
    const user = userEvent.setup();
    renderDashboard();
    const telegramCard = (await screen.findByRole("heading", { name: "Telegram" })).closest("article");
    await user.click(within(telegramCard!).getByRole("button", { name: "Configure" }));
    await user.type(screen.getByLabelText("Telegram bot token"), "telegram-secret-canary");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(within(telegramCard!).getByRole("button", { name: "Configure" }));
    expect(screen.getByLabelText("Telegram bot token")).toHaveValue("");
  });
});
