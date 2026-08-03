import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApprovalProvider } from "@/context/ApprovalContext";
import { I18nProvider } from "@/context/I18nContext";
import { ModsScreen } from "../ModsScreen";

const invokeMock = vi.hoisted(() => vi.fn());
const listeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (eventName: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(eventName, handler);
    return () => listeners.delete(eventName);
  }),
}));

type InstalledMod = {
  id: string;
  name: string;
  description: string;
  isActive: boolean;
  version: string;
  author: string;
  category: string;
  packageSize: string;
  lastUpdated: string;
  permissions: Array<{ label: string; detail: string }>;
  endpoints: string[];
  reviewState?: "reviewed" | "unreviewed" | "revoked";
  publisherIdentityVerified?: boolean;
  integrityState?: "verified" | "unsigned" | "modified" | "unknown";
  isBuiltIn?: boolean;
};

const installedMods: InstalledMod[] = [
  {
    id: "ai.eldris.mods.pundamentals",
    name: "Pundamentals",
    description: "Adds context-aware puns to active agent sessions.",
    isActive: true,
    version: "1.0.0",
    author: "Eldris AI Engineering",
    category: "Prompt Hook",
    packageSize: "34.5 KB",
    lastUpdated: "June 22, 2026",
    permissions: [
      {
        label: "No extra permissions",
        detail: "The manifest does not request additional local permissions.",
      },
    ],
    endpoints: ["None declared"],
    reviewState: "unreviewed",
    publisherIdentityVerified: true,
    integrityState: "verified",
    isBuiltIn: false,
  },
  {
    id: "ai.eldris.mods.workspace-auditor",
    name: "Workspace Auditor",
    description: "Reviews local workspace metadata after explicit approval.",
    isActive: false,
    version: "1.2.0",
    author: "Acme Corp",
    category: "Code Intelligence",
    packageSize: "48.0 KB",
    lastUpdated: "June 22, 2026",
    permissions: [
      {
        label: "Workspace files",
        detail: "Reads user-selected workspace files.",
      },
    ],
    endpoints: ["None declared"],
    reviewState: "reviewed",
    publisherIdentityVerified: false,
    integrityState: "verified",
    isBuiltIn: false,
  },
];

const installedOomuMod: InstalledMod = {
  id: "ai.eldris.mods.briefing-coach",
  name: "Briefing Coach",
  description: "Adds crisp executive summary framing to active agent sessions.",
  isActive: false,
  version: "1.0.0",
  author: "Eldris AI Engineering",
  category: "Prompt Hook",
  packageSize: "41.0 KB",
  lastUpdated: "June 22, 2026",
  permissions: [
    {
      label: "No extra permissions",
      detail: "The manifest does not request additional local permissions.",
    },
  ],
  endpoints: ["None declared"],
  reviewState: "unreviewed",
  publisherIdentityVerified: false,
  integrityState: "verified",
  isBuiltIn: false,
};

const builtInAlignmentMod: InstalledMod = {
  id: "enterprise.core.policy",
  name: "Core Alignment Matrix",
  description: "Controls alignment-specific behavior.",
  isActive: false,
  version: "1.0.0",
  author: "Eldris AI",
  category: "Behavior",
  packageSize: "Built in",
  lastUpdated: "July 3, 2026",
  permissions: [],
  endpoints: [],
  reviewState: "reviewed",
  publisherIdentityVerified: true,
  integrityState: "verified",
  isBuiltIn: true,
};

function cloneInstalledMods() {
  return installedMods.map((mod) => ({
    ...mod,
    permissions: mod.permissions.map((permission) => ({ ...permission })),
    endpoints: [...mod.endpoints],
  }));
}

function renderModsScreen() {
  return render(
    <I18nProvider>
      <ModsScreen />
    </I18nProvider>,
  );
}

function renderModsScreenWithApprovals() {
  return render(
    <I18nProvider>
      <ApprovalProvider>
        <ModsScreen />
      </ApprovalProvider>
    </I18nProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  listeners.clear();
  invokeMock.mockImplementation((command: string) => {
    if (command === "list_installed_mods") {
      return Promise.resolve(cloneInstalledMods());
    }
    return Promise.resolve(undefined);
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("ModsScreen", () => {
  it("shows install failures as an opaque in-flow alert above the mod grid", async () => {
    const grantId = "ac".repeat(32);
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") {
        return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      }
      if (command === "install_mod_from_path") {
        return Promise.reject(new Error("The selected mod package could not be installed."));
      }
      return Promise.resolve(undefined);
    });

    const view = renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));

    const alert = await screen.findByRole("alert");
    const grid = view.container.querySelector(".grid.grid-cols-2");
    expect(alert).toHaveTextContent("The selected mod package could not be installed.");
    expect(alert).toHaveAttribute("data-oomu-mod-notice", "error");
    expect(alert).toHaveClass(
      "bg-[var(--background)]",
      "border-[var(--destructive)]",
      "shadow-[var(--shadow-card)]",
    );
    expect(alert).not.toHaveClass("absolute", "bg-[var(--destructive-background)]");
    expect(grid).not.toBeNull();
    expect(alert.compareDocumentPosition(grid!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });

  it("opens the OOMU marketplace in the external browser", async () => {
    const user = userEvent.setup();
    renderModsScreen();

    const marketplaceButton = screen.getByRole("button", {
      name: "Browse marketplace",
    });
    const installButton = screen.getByRole("button", { name: "Install mod" });
    expect(marketplaceButton).toBeEnabled();
    expect(marketplaceButton.className).toBe(installButton.className);
    expect(screen.queryByText("Soon")).not.toBeInTheDocument();

    await user.click(marketplaceButton);

    expect(invokeMock).toHaveBeenCalledWith("open_oomu_marketplace");
  });

  it("loads installed mods and updates the active counter when a mod is toggled", async () => {
    const reviewedBundle = {
      bundleId: "bundle_workspace_auditor",
      packageVersion: "1.2.0",
      modId: "ai.eldris.mods.workspace-auditor",
      name: "Workspace Auditor",
      publisherName: "Acme Corp",
      publisherIdentityVerified: false,
      reviewState: "reviewed",
      integrityState: "verified",
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "inspect_capability_bundle") return Promise.resolve(reviewedBundle);
      if (command === "activate_capability_bundle") {
        return Promise.resolve({ ...reviewedBundle, installState: "active" });
      }
      return Promise.resolve(undefined);
    });
    const user = userEvent.setup();
    renderModsScreen();

    expect(await screen.findByText("1 Active")).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Activate Workspace Auditor" }));

    expect(invokeMock).toHaveBeenCalledWith("inspect_capability_bundle", {
      request: {
        modId: "ai.eldris.mods.workspace-auditor",
        projectIds: [],
      },
    });
    expect(invokeMock).toHaveBeenCalledWith("activate_capability_bundle", {
      request: {
        bundleId: "bundle_workspace_auditor",
        packageVersion: "1.2.0",
        acknowledgeUnreviewed: false,
      },
    });
    expect(screen.getByText("2 Active")).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Deactivate Workspace Auditor" }),
    ).toBeInTheDocument();
  });

  it("uses backend trust facts while preserving each developer's author name", async () => {
    renderModsScreen();

    const customCard = (await screen.findByText("Pundamentals")).closest("article");
    const reviewedCard = screen.getByText("Workspace Auditor").closest("article");
    expect(customCard).not.toBeNull();
    expect(reviewedCard).not.toBeNull();
    expect(within(customCard!).getByText("Custom Mod")).toBeVisible();
    expect(within(customCard!).queryByText("Reviewed by OOMU")).toBeNull();
    expect(within(reviewedCard!).getByText("Reviewed by OOMU")).toBeVisible();
    expect(within(reviewedCard!).getByText("Acme Corp")).toBeVisible();
    fireEvent.click(within(customCard!).getByRole("button", { name: "Configure Pundamentals" }));
    expect(screen.getByText(/developer signature is valid/i)).toBeVisible();
  });

  it("shows a warm Modified Mod state only from explicit backend integrity data", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") {
        return Promise.resolve([{ ...installedMods[0], reviewState: "reviewed", integrityState: "modified" }]);
      }
      return Promise.resolve(undefined);
    });
    renderModsScreen();

    const card = (await screen.findByText("Pundamentals")).closest("article");
    expect(card).not.toBeNull();
    expect(within(card!).getByText("Modified Mod")).toBeVisible();
    expect(within(card!).queryByText("Reviewed by OOMU")).toBeNull();
    fireEvent.click(within(card!).getByRole("button", { name: "Configure Pundamentals" }));
    expect(screen.getByText(/If you recently edited this mod's files, this is normal/i)).toBeVisible();
  });

  it("does not infer review from an Eldris author or a familiar mod ID", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") {
        return Promise.resolve([{
          ...installedMods[0],
          id: "ai.eldris.mods.alignment",
          author: "Eldris AI",
          reviewState: undefined,
          publisherIdentityVerified: undefined,
          integrityState: undefined,
          isBuiltIn: false,
        }]);
      }
      return Promise.resolve(undefined);
    });
    renderModsScreen();

    const card = (await screen.findByText("Pundamentals")).closest("article");
    expect(card).not.toBeNull();
    expect(within(card!).getByText("Review not available")).toBeVisible();
    expect(within(card!).queryByText("Reviewed by OOMU")).toBeNull();
    fireEvent.click(within(card!).getByRole("button", { name: "Configure Pundamentals" }));
    expect(screen.getByRole("button", { name: "Remove mod" })).toBeVisible();
  });

  it("installs a selected .oomu package using only its opaque native grant", async () => {
    const grantId = "ab".repeat(32);
    const bundle={bundleId:"bundle_legacy_abc",packageVersion:"1.0.0",modId:installedOomuMod.id,name:installedOomuMod.name,publisherName:"Eldris AI Engineering",publisherIdentityVerified:false,reviewState:"unreviewed",compatibilityState:"compatible",capabilities:[],projectIds:[],installState:"inspected",previousVersion:null,updatedAtMs:Date.now()};
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") {
        return Promise.resolve(cloneInstalledMods());
      }
      if (command === "choose_mod_package_path") {
        return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      }
      if (command === "install_mod_from_path") {
        return Promise.resolve(installedOomuMod);
      }
      if (command === "inspect_capability_bundle") {
        return Promise.resolve(bundle);
      }
      if (command === "activate_capability_bundle") {
        return Promise.resolve({...bundle,installState:"active"});
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();

    expect(await screen.findByText("Pundamentals")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Install mod" }));

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(invokeMock).toHaveBeenCalledWith("choose_mod_package_path");
    expect(invokeMock).toHaveBeenCalledWith("install_mod_from_path", {
      grantId,
    });
    expect(JSON.stringify(invokeMock.mock.calls)).not.toContain("/Users/");
    expect(screen.queryByText(grantId)).not.toBeInTheDocument();
    expect(screen.getByText("Briefing Coach")).toBeInTheDocument();
    expect(screen.getByRole("heading",{name:"Review Briefing Coach"})).toBeInTheDocument();
    const reviewDialog = screen.getByRole("dialog");
    fireEvent.click(within(reviewDialog).getByRole("checkbox",{name:/I trust Eldris AI Engineering/}));
    const installButton = within(reviewDialog).getByRole("button",{name:"Install mod"});
    expect(installButton).toHaveClass("text-[var(--inverse-foreground)]");
    expect(installButton).not.toHaveClass("text-white");
    fireEvent.click(installButton);
    expect(await screen.findByText("Briefing Coach successfully installed.")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("inspect_capability_bundle", {
      request: { modId: installedOomuMod.id, projectIds: [] },
    });
  });

  it("turns on an Eldris-reviewed package without showing a trust dialog", async () => {
    const grantId = "aa".repeat(32);
    const reviewedBundle = {
      bundleId: "bundle_reviewed_eldris",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Eldris Inc",
      publisherIdentityVerified: false,
      reviewState: "reviewed" as const,
      integrityState: "verified" as const,
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") {
        return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      }
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") return Promise.resolve(reviewedBundle);
      if (command === "activate_capability_bundle") {
        return Promise.resolve({ ...reviewedBundle, installState: "active" });
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));

    expect(await screen.findByText("Briefing Coach successfully installed.")).toBeVisible();
    expect(screen.queryByRole("dialog", { name: "Review Briefing Coach" })).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("activate_capability_bundle", {
      request: {
        bundleId: reviewedBundle.bundleId,
        packageVersion: reviewedBundle.packageVersion,
        acknowledgeUnreviewed: false,
      },
    });
  });

  it("opens the missing sideload acknowledgement when an inactive mod is turned on", async () => {
    const sideloadedMod = {
      ...installedOomuMod,
      author: "Eldris / DeepSeek",
      name: "Sovereign Boost",
    };
    const bundle = {
      bundleId: "bundle_sideloaded_boost",
      packageVersion: "1.0.0",
      modId: sideloadedMod.id,
      name: sideloadedMod.name,
      publisherName: "Eldris / DeepSeek",
      publisherIdentityVerified: false,
      reviewState: "unreviewed" as const,
      integrityState: "unsigned" as const,
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve([sideloadedMod]);
      if (command === "inspect_capability_bundle") return Promise.resolve(bundle);
      if (command === "activate_capability_bundle") {
        return Promise.resolve({ ...bundle, installState: "active" });
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(
      await screen.findByRole("switch", { name: "Activate Sovereign Boost" }),
    );

    const dialog = await screen.findByRole("dialog", { name: "Review Sovereign Boost" });
    expect(within(dialog).queryByText(/Projects that can use/)).toBeNull();
    const acknowledgement = within(dialog).getByRole("checkbox", {
      name: /I trust Eldris \/ DeepSeek/,
    });
    expect(acknowledgement).toBeVisible();
    fireEvent.click(acknowledgement);
    fireEvent.click(within(dialog).getByRole("button", { name: "Turn on" }));

    expect(await screen.findByText("Sovereign Boost is on.")).toBeVisible();
  });

  it("hides unsafe bundle identity and capability text while offering a generic sideload acknowledgement", async () => {
    const grantId = "ce".repeat(32);
    const unsafeBundle = {
      bundleId: "bundle_unsafe_identity",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: "`MALICIOUS_NAME_CANARY`",
      publisherName: "Bearer PUBLISHER_SECRET_CANARY",
      publisherIdentityVerified: false,
      reviewState: "unreviewed" as const,
      integrityState: "unsigned" as const,
      compatibilityState: "compatible",
      capabilities: [{
        capability: "file",
        boundedScope: "```sh\nrm -rf /tmp/CAPABILITY_SECRET_CANARY\n```",
        reason: "RAW_REASON_CANARY",
      }],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") return Promise.resolve(unsafeBundle);
      if (command === "activate_capability_bundle") {
        return Promise.resolve({ ...unsafeBundle, installState: "active" });
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));

    const dialog = await screen.findByRole("dialog", { name: "Review Unknown" });
    expect(within(dialog).getByText("Can create and change files in Unknown.")).toBeVisible();
    expect(within(dialog).getByText(/valid developer signature for Unknown/)).toBeVisible();
    expect(within(dialog).queryByText(/MALICIOUS_NAME_CANARY|PUBLISHER_SECRET_CANARY|CAPABILITY_SECRET_CANARY|RAW_REASON_CANARY|```|Bearer/)).toBeNull();

    const acknowledgement = within(dialog).getByRole("checkbox", {
      name: /I trust where this mod came from/,
    });
    const installButton = within(dialog).getByRole("button", { name: "Install mod" });
    expect(installButton).toBeDisabled();
    fireEvent.click(acknowledgement);
    expect(installButton).toBeEnabled();
    fireEvent.click(installButton);
    expect(await screen.findByText("Unknown successfully installed.")).toBeInTheDocument();
  });

  it("keeps a bundle permission review in front and queues a later native Shield status", async () => {
    const grantId = "bc".repeat(32);
    const bundle = {
      bundleId: "bundle_fifo_abc",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: "**Briefing Coach**",
      publisherName: "[Eldris AI Engineering](https://eldris.example)",
      publisherIdentityVerified: true,
      reviewState: "unreviewed" as "reviewed" | "unreviewed" | "revoked",
      integrityState: "verified" as "verified" | "unsigned" | "modified" | "unknown",
      compatibilityState: "compatible",
      capabilities: [
        { capability: "file", boundedScope: "/Users/example/Finance", reason: "RAW_REASON_CANARY" },
        { capability: "network", boundedScope: "https://api.example/private?token=secret", reason: "RAW_REASON_CANARY" },
        { capability: "future_capability", boundedScope: "**approved_service**", reason: "RAW_REASON_CANARY" },
      ],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") return Promise.resolve(bundle);
      if (command === "list_pending_shield_approvals") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    renderModsScreenWithApprovals();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));
    const bundleDialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    expect(within(bundleDialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(within(bundleDialog).getByText("Publisher listed in the signed package: Eldris AI Engineering.")).toBeVisible();
    expect(within(bundleDialog).getByText("Can create and change files in Finance.")).toBeVisible();
    expect(within(bundleDialog).getByText("Can connect to api.example on the internet.")).toBeVisible();
    expect(within(bundleDialog).getByText("Can use the approved approved service capability.")).toBeVisible();
    expect(within(bundleDialog).queryByText(/future_capability|mods\.capability_sentences/)).toBeNull();
    expect(within(bundleDialog).queryByText(/RAW_REASON|\/Users\/|private|token=secret|\*\*|\]\(/)).toBeNull();
    await waitFor(() => expect(listeners.has("shield-approval-status-changed")).toBe(true));

    act(() => listeners.get("shield-approval-status-changed")?.({
      payload: {
        displayId: "queued-after-bundle",
        sessionId: "session-queued",
        actionLabel: "List files",
        semanticSummary: "Review files in the selected folder.",
        requestedAtMs: 1,
        pending: true,
      },
    }));

    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(bundleDialog).toBeVisible();
    fireEvent.keyDown(bundleDialog, { key: "Escape" });
    expect(await screen.findByRole("heading", { name: "Review the native OOMU prompt" })).toBeVisible();
    expect(screen.getByText("Review files in the selected folder.")).toBeVisible();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("locks dismissal while a sideload activation is being checked", async () => {
    const grantId = "bd".repeat(32);
    const bundle = {
      bundleId: "bundle_busy_abc",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Eldris AI Engineering",
      publisherIdentityVerified: true,
      reviewState: "unreviewed" as "reviewed" | "unreviewed" | "revoked",
      integrityState: "verified" as "verified" | "unsigned" | "modified" | "unknown",
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    let inspectionCount = 0;
    let resolveReinspection!: (value: typeof bundle) => void;
    const reinspection = new Promise<typeof bundle>((resolve) => {
      resolveReinspection = resolve;
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") {
        inspectionCount += 1;
        return inspectionCount === 1 ? Promise.resolve(bundle) : reinspection;
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));
    const dialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    const acknowledgement = within(dialog).getByRole("checkbox", {
      name: /I trust Eldris AI Engineering/,
    });
    fireEvent.click(acknowledgement);
    fireEvent.click(within(dialog).getByRole("button", { name: "Install mod" }));

    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(acknowledgement).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Installing…" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Installing…" })).toHaveAttribute("aria-busy", "true");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(dialog).toBeVisible();

    await act(async () => {
      resolveReinspection({ ...bundle, reviewState: "revoked" });
      await reinspection;
    });
    expect(within(dialog).getByText("Review withdrawn")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
  });

  it("keeps the trust acknowledgement available for a publisher collaboration name", async () => {
    const grantId = "bf".repeat(32);
    const bundle = {
      bundleId: "bundle_identity_reinspect",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Eldris / DeepSeek",
      publisherIdentityVerified: false,
      reviewState: "unreviewed" as const,
      integrityState: "unsigned" as const,
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    let inspectionCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") {
        inspectionCount += 1;
        return Promise.resolve(bundle);
      }
      if (command === "activate_capability_bundle") {
        return Promise.resolve({ ...bundle, installState: "active" });
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));
    const dialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    const acknowledgement = within(dialog).getByRole("checkbox", {
      name: /I trust Eldris \/ DeepSeek/,
    });
    expect(acknowledgement).toBeVisible();
    fireEvent.click(acknowledgement);
    fireEvent.click(within(dialog).getByRole("button", { name: "Install mod" }));

    expect(await screen.findByText("Briefing Coach successfully installed.")).toBeVisible();
    expect(inspectionCount).toBe(2);
  });

  it("does not offer an acknowledgement bypass for a revoked capability bundle", async () => {
    const grantId = "cd".repeat(32);
    const revokedBundle = {
      bundleId: "bundle_revoked_abc",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Eldris AI Engineering",
      publisherIdentityVerified: true,
      reviewState: "revoked",
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") return Promise.resolve(revokedBundle);
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));

    const dialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    expect(within(dialog).getByText("Review withdrawn")).toBeVisible();
    expect(within(dialog).queryByRole("checkbox", { name: /I trust/i })).toBeNull();
    const installButton = within(dialog).getByRole("button", { name: "Install mod" });
    expect(installButton).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("activate_capability_bundle", expect.anything());
  });

  it("keeps a modified package visible but unavailable for activation", async () => {
    const grantId = "de".repeat(32);
    const modifiedBundle = {
      bundleId: "bundle_modified_abc",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Acme Corp",
      publisherIdentityVerified: false,
      reviewState: "unreviewed",
      integrityState: "modified",
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") return Promise.resolve(modifiedBundle);
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));

    const dialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    expect(within(dialog).getByText("Modified Mod")).toBeVisible();
    expect(within(dialog).queryByRole("checkbox", { name: /I trust/i })).toBeNull();
    const installButton = within(dialog).getByRole("button", { name: "Install mod" });
    expect(installButton).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("activate_capability_bundle", expect.anything());
  });

  it.each([
    { name: "revoked", reviewState: "revoked", integrityState: "verified", label: "Review withdrawn" },
    { name: "modified", reviewState: "unreviewed", integrityState: "modified", label: "Modified Mod" },
  ])("stops activation when re-inspection returns $name trust", async ({ integrityState, label, reviewState }) => {
    const grantId = "ef".repeat(32);
    const reviewedBundle = {
      bundleId: "bundle_reinspect_abc",
      packageVersion: "1.0.0",
      modId: installedOomuMod.id,
      name: installedOomuMod.name,
      publisherName: "Acme Corp",
      publisherIdentityVerified: true,
      reviewState: "unreviewed",
      integrityState: "unsigned",
      compatibilityState: "compatible",
      capabilities: [],
      projectIds: [],
      installState: "inspected",
      previousVersion: null,
      updatedAtMs: Date.now(),
    };
    let inspectionCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") return Promise.resolve(cloneInstalledMods());
      if (command === "choose_mod_package_path") return Promise.resolve({ grantId, expiresAtMs: Date.now() + 60_000 });
      if (command === "install_mod_from_path") return Promise.resolve(installedOomuMod);
      if (command === "inspect_capability_bundle") {
        inspectionCount += 1;
        return Promise.resolve(inspectionCount === 1
          ? reviewedBundle
          : { ...reviewedBundle, reviewState, integrityState });
      }
      return Promise.resolve(undefined);
    });

    renderModsScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Install mod" }));
    const dialog = await screen.findByRole("dialog", { name: "Review Briefing Coach" });
    fireEvent.click(within(dialog).getByRole("checkbox", { name: /I trust Acme Corp/ }));
    const installButton = within(dialog).getByRole("button", { name: "Install mod" });
    expect(installButton).toBeEnabled();
    fireEvent.click(installButton);

    expect(await within(dialog).findByText(label)).toBeVisible();
    expect(dialog).toBeVisible();
    expect(installButton).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("activate_capability_bundle", expect.anything());
  });

  it("anchors the configuration drawer to the viewport right edge", async () => {
    renderModsScreen();

    expect(await screen.findByText("Pundamentals")).toBeInTheDocument();

    const modCard = screen.getByText("Pundamentals").closest("article");
    expect(modCard).not.toBeNull();

    fireEvent.click(
      within(modCard as HTMLElement).getByRole("button", {
        name: "Configure Pundamentals",
      }),
    );

    expect(screen.getByRole("dialog", { name: "Pundamentals" })).toHaveClass(
      "fixed",
      "right-0",
      "top-12",
      "w-[min(24rem,100vw)]",
    );
  });

  it("offers confirmed removal for a mod marked as built in", async () => {
    let registry = [builtInAlignmentMod];
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") {
        return Promise.resolve(registry);
      }
      if (command === "uninstall_mod") {
        registry = [];
        return Promise.resolve(true);
      }
      return Promise.resolve(undefined);
    });
    renderModsScreen();

    expect(await screen.findByText("Core Alignment Matrix")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Configure Core Alignment Matrix",
      }),
    );

    const drawer = screen.getByRole("dialog", { name: "Core Alignment Matrix" });
    expect(
      within(drawer).getByRole("switch", { name: "Activate Core Alignment Matrix" }),
    ).toBeInTheDocument();
    fireEvent.click(within(drawer).getByRole("button", { name: "Remove mod" }));

    const confirmation = screen.getByRole("dialog", {
      name: "Remove Core Alignment Matrix?",
    });
    expect(
      within(confirmation).getByText(
        "This mod and its files will be permanently removed from OOMU. You cannot undo this.",
      ),
    ).toBeVisible();
    fireEvent.click(within(confirmation).getByRole("button", { name: "Remove mod" }));

    expect(invokeMock).toHaveBeenCalledWith("uninstall_mod", {
      modId: builtInAlignmentMod.id,
    });
    expect(await screen.findByText("Core Alignment Matrix removed.")).toBeInTheDocument();
  });

  it("removes a mod only after the native registry confirms the uninstall", async () => {
    let registry = cloneInstalledMods();
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_mods") {
        return Promise.resolve(registry);
      }
      if (command === "uninstall_mod") {
        registry = registry.filter((mod) => mod.id !== "ai.eldris.mods.pundamentals");
        return Promise.resolve(true);
      }
      return Promise.resolve(undefined);
    });
    renderModsScreen();

    expect(await screen.findByText("Pundamentals")).toBeInTheDocument();

    const modCard = screen.getByText("Pundamentals").closest("article");
    expect(modCard).not.toBeNull();

    fireEvent.click(
      within(modCard as HTMLElement).getByRole("button", {
        name: "Configure Pundamentals",
      }),
    );

    expect(screen.getByRole("dialog", { name: "Pundamentals" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Remove mod" }));

    const confirmation = screen.getByRole("dialog", { name: "Remove Pundamentals?" });
    fireEvent.click(within(confirmation).getByRole("button", { name: "Remove mod" }));

    expect(screen.getByRole("dialog", { name: "Pundamentals" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("uninstall_mod", {
      modId: "ai.eldris.mods.pundamentals",
    });
    expect(await screen.findByText("Pundamentals removed.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Pundamentals" })).not.toBeInTheDocument();
    expect(screen.getByText("0 Active")).toBeInTheDocument();
  });

  it("keeps the mod visible when the native registry does not confirm removal", async () => {
    renderModsScreen();

    expect(await screen.findByText("Pundamentals")).toBeInTheDocument();

    const modCard = screen.getByText("Pundamentals").closest("article");
    expect(modCard).not.toBeNull();

    fireEvent.click(
      within(modCard as HTMLElement).getByRole("button", {
        name: "Configure Pundamentals",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Remove mod" }));
    const confirmation = screen.getByRole("dialog", { name: "Remove Pundamentals?" });
    fireEvent.click(within(confirmation).getByRole("button", { name: "Remove mod" }));

    expect(await screen.findByText("Failed to remove mod.")).toBeInTheDocument();
    expect(within(modCard as HTMLElement).getByText("Pundamentals")).toBeInTheDocument();
    expect(screen.getByText("1 Active")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Pundamentals" })).toBeInTheDocument();
  });
});
