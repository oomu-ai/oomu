import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppShell, resolveAppDestination, useAppShell } from "../AppShell";

const navigationMock = vi.hoisted(() => ({
  pathname: "/",
  push: vi.fn(),
}));

const invokeMock = vi.hoisted(() => vi.fn());
const approvalContextMock = vi.hoisted(() => ({
  value: null as null | {
    focusNextApproval: () => void;
    pendingApprovalCount: number;
  },
}));

vi.mock("next/navigation", () => ({
  usePathname: () => navigationMock.pathname,
  useRouter: () => ({
    push: navigationMock.push,
  }),
}));

vi.mock("@/app/components/BrowserEnvironmentGuard", () => ({
  BrowserEnvironmentGuard: () => null,
  useBrowserEnvironment: () => ({
    isRuntimeChecked: true,
    isUncontainedBrowser: false,
  }),
}));

vi.mock("@/context/AppContext", () => ({
  useAppContext: () => ({
    isInitializing: false,
    isSecureEnvironment: true,
  }),
}));

vi.mock("@/context/ApprovalContext", () => ({
  useOptionalApproval: () => approvalContextMock.value,
}));

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string, values?: Record<string, string | number>) =>
      ({
        "approvals.open": "Review",
        "approvals.pending_many": `${values?.count ?? 0} need your OK`,
        "approvals.pending_one": "1 needs your OK",
        "sidebar.agents": "Agents",
        "sidebar.artifacts": "Documents",
        "sidebar.channels": "Channels",
        "sidebar.chat": "Chat",
        "sidebar.connections": "Connections",
        "sidebar.developer": "Developer",
        "sidebar.ledger": "Ledger",
        "sidebar.menu": "Menu",
        "sidebar.mods": "Mods",
        "sidebar.personalization": "Personalization",
        "sidebar.primary": "Primary",
        "sidebar.projects": "Projects",
        "sidebar.settings": "Settings",
        "sidebar.tasks": "Tasks",
        "sidebar.workflows": "Workflows",
        "status.secure": "Secure",
      })[key] ?? key,
  }),
}));

vi.mock("@/lib/buildFlags", () => ({
  isDeveloperBuild: false,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@/app/components/integrations/RecommendedModelInstallIndicator", () => ({
  RecommendedModelInstallIndicator: () => null,
}));

beforeEach(() => {
  approvalContextMock.value = null;
  navigationMock.pathname = "/";
  navigationMock.push.mockReset();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({
    debugMode: false,
    dumpDb: false,
    firstRunSetup: false,
    logLevel: "info",
    resetState: false,
    safeMode: false,
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function NavigationProbe() {
  const {
    activeItem,
    connectionsSection,
    globalChatRequestId,
    tasksSection,
    workflowsView,
    workflowDraft,
    setActiveItem,
    setWorkflowDraft,
    setWorkflowsView,
  } = useAppShell();

  return (
    <div>
      <output aria-label="Active item">{activeItem}</output>
      <output aria-label="Tasks section">{tasksSection}</output>
      <output aria-label="Connections section">{connectionsSection}</output>
      <output aria-label="Global chat request">{globalChatRequestId}</output>
      <output aria-label="Workflows view">{workflowsView}</output>
      <output aria-label="Workflow draft">{workflowDraft ? "present" : "none"}</output>
      <button onClick={() => setActiveItem("routines")} type="button">
        Open routines
      </button>
      <button onClick={() => setActiveItem("workflows")} type="button">
        Open workflows
      </button>
      <button onClick={() => setActiveItem("integrations")} type="button">
        Open integrations
      </button>
      <button onClick={() => setActiveItem("channels")} type="button">
        Open channels
      </button>
      <button onClick={() => setActiveItem("chat")} type="button">
        Open chat programmatically
      </button>
      <button
        onClick={() => {
          setWorkflowsView("saved_workflows");
          setWorkflowDraft({ id: "workflow-1", name: "Prepared", description: "" });
        }}
        type="button"
      >
        Prepare workflow edit
      </button>
    </div>
  );
}

describe("AppShell navigation", () => {
  it("keeps pending approvals visible in chrome and opens the next prompt", async () => {
    const focusNextApproval = vi.fn();
    approvalContextMock.value = { focusNextApproval, pendingApprovalCount: 2 };
    const user = userEvent.setup();

    render(
      <AppShell>
        <div>Current route content</div>
      </AppShell>,
    );

    const indicator = screen.getByRole("button", { name: "Review" });
    expect(indicator).toHaveTextContent("2 need your OK");
    await user.click(indicator);
    expect(focusNextApproval).toHaveBeenCalledTimes(1);
  });

  it("resolves canonical and legacy destinations through one typed contract", () => {
    expect(resolveAppDestination("routines")).toEqual({
      item: "tasks",
      pathname: "/",
      tasksSection: "scheduled",
    });
    expect(resolveAppDestination("workflows")).toEqual({
      item: "tasks",
      pathname: "/",
      tasksSection: "workflows",
    });
    expect(resolveAppDestination("integrations")).toEqual({
      item: "connections",
      pathname: "/",
      connectionsSection: "work_apps",
    });
    expect(resolveAppDestination("channels")).toEqual({
      item: "connections",
      pathname: "/channels",
      connectionsSection: "messaging",
    });
    expect(resolveAppDestination("agents")).toEqual({ item: "agents", pathname: "/" });
    expect(resolveAppDestination("hero")).toEqual({ item: "hero", pathname: "/" });

    for (const item of [
      "chat",
      "projects",
      "tasks",
      "artifacts",
      "connections",
      "mods",
    ] as const) {
      expect(resolveAppDestination(item).item).toBe(item);
    }
  });

  it("renders only the four primary starting points in the intended order", () => {
    render(
      <AppShell>
        <div>Current route content</div>
      </AppShell>,
    );

    const menu = screen.getByRole("navigation", { name: "Menu" });
    expect(within(menu).getAllByRole("button").map((button) => button.textContent)).toEqual([
      "Chat",
      "Projects",
      "Connections",
      "Mods",
    ]);
    expect(screen.getByRole("button", { name: "Settings" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Ledger" })).toBeVisible();
  });

  it("marks sidebar Chat as an explicit request for global chat", async () => {
    const user = userEvent.setup();
    render(
      <AppShell>
        <NavigationProbe />
      </AppShell>,
    );

    expect(screen.getByLabelText("Global chat request")).toHaveTextContent("0");
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.getByLabelText("Global chat request")).toHaveTextContent("1");

    await user.click(screen.getByRole("button", { name: "Open chat programmatically" }));
    expect(screen.getByLabelText("Global chat request")).toHaveTextContent("1");
  });

  it("marks first-run setup launch mode on the shell", async () => {
    invokeMock.mockResolvedValueOnce({
      debugMode: false,
      dumpDb: false,
      firstRunSetup: true,
      logLevel: "info",
      resetState: false,
      safeMode: false,
    });

    render(
      <AppShell>
        <div>Current route content</div>
      </AppShell>,
    );

    await waitFor(() =>
      expect(screen.getByText("Current route content").closest("[data-oomu-first-run-setup]"))
        .toHaveAttribute("data-oomu-first-run-setup", "true"),
    );
  });

  it("selects Connections and Messaging immediately for a /channels deep link", async () => {
    const user = userEvent.setup();
    navigationMock.pathname = "/channels";

    render(
      <AppShell>
        <NavigationProbe />
      </AppShell>,
    );

    const connectionsButton = screen.getByRole("button", { name: "Connections" });
    const chatButton = screen.getByRole("button", { name: "Chat" });

    expect(connectionsButton).toHaveAttribute("aria-current", "page");
    expect(screen.getByLabelText("Active item")).toHaveTextContent("connections");
    expect(screen.getByLabelText("Connections section")).toHaveTextContent("messaging");

    await user.click(chatButton);

    expect(navigationMock.push).toHaveBeenCalledWith("/", { scroll: false });
    await waitFor(() => expect(chatButton).toHaveAttribute("aria-current", "page"));
    expect(connectionsButton).not.toHaveAttribute("aria-current");
  });

  it("preserves canonical Connections routing from a /channels deep link", async () => {
    const user = userEvent.setup();
    navigationMock.pathname = "/channels";

    render(
      <AppShell>
        <NavigationProbe />
      </AppShell>,
    );

    await user.click(screen.getByRole("button", { name: "Connections" }));

    expect(navigationMock.push).toHaveBeenCalledWith("/", { scroll: false });
    expect(screen.getByLabelText("Active item")).toHaveTextContent("connections");
    expect(screen.getByLabelText("Connections section")).toHaveTextContent("work_apps");
  });

  it("updates section state and resets workflow editing from legacy requests", async () => {
    const user = userEvent.setup();

    render(
      <AppShell>
        <NavigationProbe />
      </AppShell>,
    );

    await user.click(screen.getByRole("button", { name: "Open routines" }));
    expect(screen.getByLabelText("Active item")).toHaveTextContent("tasks");
    expect(screen.getByLabelText("Tasks section")).toHaveTextContent("scheduled");

    await user.click(screen.getByRole("button", { name: "Prepare workflow edit" }));
    expect(screen.getByLabelText("Workflows view")).toHaveTextContent("saved_workflows");
    expect(screen.getByLabelText("Workflow draft")).toHaveTextContent("present");

    await user.click(screen.getByRole("button", { name: "Open workflows" }));
    expect(screen.getByLabelText("Tasks section")).toHaveTextContent("workflows");
    expect(screen.getByLabelText("Workflows view")).toHaveTextContent("composer");
    expect(screen.getByLabelText("Workflow draft")).toHaveTextContent("none");

    await user.click(screen.getByRole("button", { name: "Open integrations" }));
    expect(screen.getByLabelText("Active item")).toHaveTextContent("connections");
    expect(screen.getByLabelText("Connections section")).toHaveTextContent("work_apps");

    await user.click(screen.getByRole("button", { name: "Open channels" }));
    expect(screen.getByLabelText("Connections section")).toHaveTextContent("messaging");
    expect(navigationMock.push).toHaveBeenLastCalledWith("/channels", { scroll: false });
  });
});
