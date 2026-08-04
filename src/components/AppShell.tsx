"use client";

import {
  createContext,
  ReactNode,
  useContext,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import { usePathname, useRouter } from "next/navigation";
import {
  BrowserEnvironmentGuard,
  useBrowserEnvironment,
} from "@/app/components/BrowserEnvironmentGuard";
import { Sidebar, type SidebarItem } from "./Sidebar";
import { useAppContext } from "@/context/AppContext";
import { useOptionalApproval } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { invoke } from "@/lib/invoke";
import { AppControlMonitor } from "@/app/components/computer_use/AppControlMonitor";
import { RecommendedModelInstallIndicator } from "@/app/components/integrations/RecommendedModelInstallIndicator";
import { ApplicationUpdateCoordinator } from "./ApplicationUpdateCoordinator";
import type { RoutineDraft } from "@/app/components/routines/routineDraft";
import type { ResolvedAppSection } from "./appNavigation";
export type {
  PrimaryAppSection,
  ResolvedAppSection,
} from "./appNavigation";

function ChatIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="M4 5h16v11H8l-4 4V5Z" />
      <path d="M8 9h8" />
      <path d="M8 12h5" />
    </svg>
  );
}

function ProjectsIcon() {
  return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3z" /><path d="M3 10h18" /></svg>;
}

function ConnectionsIcon() {
  return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M8 3v4M16 3v4M6 7h12v4a6 6 0 0 1-12 0V7Z" /><path d="M12 17v4" /></svg>;
}

function ModsIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="M12 2L2 7l10 5 10-5-10-5z" />
      <path d="M2 17l10 5 10-5" />
      <path d="M2 12l10 5 10-5" />
    </svg>
  );
}

function LockIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <rect height="11" width="18" rx="1.5" ry="1.5" x="3" y="11" />
      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
    </svg>
  );
}

function UserIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
      <circle cx="12" cy="7" r="4" />
    </svg>
  );
}

function DeveloperIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="m8 9-4 3 4 3" />
      <path d="m16 9 4 3-4 3" />
      <path d="m14 4-4 16" />
    </svg>
  );
}

type LegacyAppSection = "routines" | "workflows" | "integrations" | "channels";
export type AppSection = ResolvedAppSection | LegacyAppSection;
export type TasksSection = "now" | "scheduled" | "workflows";
export type ConnectionsSection = "work_apps" | "messaging";

type ResolvedAppDestination = {
  item: ResolvedAppSection;
  pathname: "/" | "/channels";
  tasksSection?: TasksSection;
  connectionsSection?: ConnectionsSection;
};

const APP_DESTINATIONS: Record<AppSection, ResolvedAppDestination> = {
  chat: { item: "chat", pathname: "/" },
  projects: { item: "projects", pathname: "/" },
  tasks: { item: "tasks", pathname: "/", tasksSection: "now" },
  artifacts: { item: "artifacts", pathname: "/" },
  connections: { item: "connections", pathname: "/", connectionsSection: "work_apps" },
  mods: { item: "mods", pathname: "/" },
  routines: { item: "tasks", pathname: "/", tasksSection: "scheduled" },
  workflows: { item: "tasks", pathname: "/", tasksSection: "workflows" },
  integrations: { item: "connections", pathname: "/", connectionsSection: "work_apps" },
  channels: { item: "connections", pathname: "/channels", connectionsSection: "messaging" },
  agents: { item: "agents", pathname: "/" },
  hero: { item: "hero", pathname: "/" },
  ledger: { item: "ledger", pathname: "/" },
  settings: { item: "settings", pathname: "/" },
  user_config: { item: "user_config", pathname: "/" },
  developer: { item: "developer", pathname: "/" },
};

export function resolveAppDestination(item: AppSection): ResolvedAppDestination {
  return APP_DESTINATIONS[item];
}

const sidebarItems: readonly SidebarItem[] = [
  { id: "chat", labelKey: "sidebar.chat", icon: <ChatIcon /> },
  { id: "projects", labelKey: "sidebar.projects", icon: <ProjectsIcon /> },
  { id: "connections", labelKey: "sidebar.connections", icon: <ConnectionsIcon /> },
  { id: "mods", labelKey: "sidebar.mods", icon: <ModsIcon /> },
];
type AgentsView = "my_agents" | "template" | "import_agent";
type WorkflowsView = "composer" | "saved_workflows";
export type WorkflowProjectScope = {
  projectId: string;
  projectName: string;
};
type WorkflowDraft = {
  id: string | null;
  name: string;
  description: string;
  workflowIr?: unknown;
  workflowVersion?: number;
  compilationStatus?: "Draft" | "Compiling" | "Compiled" | "Failed";
  createdAt?: number;
  isActive?: boolean;
  lastRunAt?: number;
  projectId?: string | null;
};
type AppShellState = {
  activeItem: ResolvedAppSection;
  globalChatRequestId: number;
  agentsView: AgentsView;
  connectionsSection: ConnectionsSection;
  tasksSection: TasksSection;
  workflowsView: WorkflowsView;
  workflowProjectScope: WorkflowProjectScope | null;
  workflowDraft: WorkflowDraft | null;
  routineDraft: RoutineDraft | null;
  launchOptions: LaunchOptions | null;
  setActiveItem: (item: AppSection) => void;
  setAgentsView: (view: AgentsView) => void;
  setConnectionsSection: (section: ConnectionsSection) => void;
  setTasksSection: (section: TasksSection) => void;
  setWorkflowsView: (view: WorkflowsView) => void;
  setWorkflowProjectScope: (scope: WorkflowProjectScope | null) => void;
  setWorkflowDraft: (draft: WorkflowDraft | null) => void;
  setRoutineDraft: (draft: RoutineDraft | null) => void;
  // A screen with unsaved work can register a guard here. When set, navigation is
  // routed through it (instead of happening immediately) so the screen can confirm
  // before the user loses changes. The guard calls `proceed()` to allow the move.
  navGuardRef: MutableRefObject<((proceed: () => void) => void) | null>;
};

type LaunchOptions = {
  debugMode: boolean;
  safeMode: boolean;
  firstRunSetup: boolean;
  logLevel: string;
  dumpDb: boolean;
  resetState: boolean;
};

function nativeFlagValue(value: boolean | undefined) {
  return value === undefined ? "unknown" : value ? "true" : "false";
}

const OOMU_OPEN_SETTINGS_EVENT = "oomu://open-settings";

const AppShellContext = createContext<AppShellState | null>(null);

export function useAppShell() {
  const context = useContext(AppShellContext);
  if (!context) {
    throw new Error("useAppShell must be used within AppShell.");
  }
  return context;
}

export function AppShell({ children }: { children: ReactNode }) {
  const { isSecureEnvironment, isInitializing } = useAppContext();
  const approvals = useOptionalApproval();
  const { t } = useI18n();
  const pathname = usePathname();
  const router = useRouter();
  const initialDestination = resolveAppDestination(pathname === "/channels" ? "channels" : "chat");
  const [navigationSelection, setNavigationSelection] = useState<{
    item: ResolvedAppSection;
    pathname: string;
  }>(() => ({
    item: initialDestination.item,
    pathname,
  }));
  const [agentsView, setAgentsView] = useState<AgentsView>("my_agents");
  const [selectedConnectionsSection, setConnectionsSection] = useState<ConnectionsSection>(initialDestination.connectionsSection ?? "work_apps");
  const [tasksSection, setTasksSection] = useState<TasksSection>(initialDestination.tasksSection ?? "now");
  const [workflowsView, setWorkflowsView] = useState<WorkflowsView>("composer");
  const [workflowProjectScope, setWorkflowProjectScope] =
    useState<WorkflowProjectScope | null>(null);
  const [workflowDraft, setWorkflowDraft] = useState<WorkflowDraft | null>(null); const [routineDraft, setRoutineDraft] = useState<RoutineDraft | null>(null);
  const [globalChatRequestId, setGlobalChatRequestId] = useState(0);
  const [launchOptions, setLaunchOptions] = useState<LaunchOptions | null>(null);
  const browserEnvironment = useBrowserEnvironment();
  const navGuardRef = useRef<((proceed: () => void) => void) | null>(null);

  const connectionsSection =
    pathname === "/channels" && navigationSelection.pathname !== pathname
      ? "messaging"
      : selectedConnectionsSection;
  const activeItem: ResolvedAppSection =
    navigationSelection.pathname === pathname
      ? navigationSelection.item
      : pathname === "/channels"
        ? "connections"
        : navigationSelection.item === "connections" && connectionsSection === "messaging"
          ? "chat"
          : navigationSelection.item;

  useEffect(() => {
    let cancelled = false;

    async function loadLaunchOptions() {
      try {
        const result = await invoke<LaunchOptions>("get_launch_options");
        if (cancelled) {
          return;
        }

        setLaunchOptions(result);
        document.documentElement.dataset.oomuDebugMode = result.debugMode ? "true" : "false";
        document.documentElement.dataset.oomuSafeMode = result.safeMode ? "true" : "false";
        document.documentElement.dataset.oomuFirstRunSetup = result.firstRunSetup ? "true" : "false";
        document.documentElement.dataset.oomuLogLevel = result.logLevel;
      } catch (error) {
        if (!cancelled) {
          setLaunchOptions(null);
          delete document.documentElement.dataset.oomuLogLevel;
        }
        console.error("Unable to load native launch options.", error);
      }
    }

    void loadLaunchOptions();

    return () => {
      cancelled = true;
    };
  }, []);

  const handleItemSelect = useCallback((
    item: AppSection,
    options?: { startGlobalChat?: boolean },
  ) => {
    const destination = resolveAppDestination(item);
    const proceed = () => {
      if (item === "chat" && options?.startGlobalChat) {
        setGlobalChatRequestId((current) => current + 1);
      }
      setNavigationSelection({ item: destination.item, pathname });
      if (destination.tasksSection) {
        setTasksSection(destination.tasksSection);
      }
      if (destination.connectionsSection) {
        setConnectionsSection(destination.connectionsSection);
      }
      if (item === "agents") {
        setAgentsView("my_agents");
      } else if (item === "workflows") {
        setWorkflowDraft(null);
        setWorkflowProjectScope(null);
        setWorkflowsView("composer");
      }
      if (pathname !== destination.pathname) {
        router.push(destination.pathname, { scroll: false });
      }
    };

    if (navGuardRef.current) {
      navGuardRef.current(proceed);
    } else {
      proceed();
    }
  }, [pathname, router]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    async function registerNativeMenuListeners() {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const settingsUnlisten = await listen(OOMU_OPEN_SETTINGS_EVENT, () => {
          handleItemSelect("settings");
        });
        if (cancelled) {
          settingsUnlisten();
          return;
        }
        unlisteners.push(settingsUnlisten);
      } catch {
        // Native menu events are only available in the Tauri runtime.
      }
    }

    void registerNativeMenuListeners();

    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [handleItemSelect]);

  const value = useMemo<AppShellState>(
    () => ({
      activeItem,
      globalChatRequestId,
      agentsView,
      connectionsSection,
      tasksSection,
      workflowsView,
      workflowProjectScope,
      workflowDraft, routineDraft,
      launchOptions,
      setActiveItem: handleItemSelect,
      setAgentsView,
      setConnectionsSection,
      setTasksSection,
      setWorkflowsView,
      setWorkflowProjectScope,
      setWorkflowDraft, setRoutineDraft,
      navGuardRef,
    }),
    [
      activeItem,
      globalChatRequestId,
      agentsView,
      connectionsSection,
      tasksSection,
      workflowsView,
      workflowProjectScope,
      workflowDraft, routineDraft,
      launchOptions,
      handleItemSelect,
    ],
  );

  return (
    <AppShellContext.Provider value={value}>
      <div
        className={`flex h-screen w-screen overflow-hidden bg-[var(--background)] text-[var(--foreground)] ${
          launchOptions?.debugMode
            ? "outline outline-1 outline-offset-[-1px] outline-[var(--border-strong)]"
            : ""
        }`}
        data-oomu-debug-mode={nativeFlagValue(launchOptions?.debugMode)}
        data-oomu-safe-mode={nativeFlagValue(launchOptions?.safeMode)}
        data-oomu-first-run-setup={nativeFlagValue(launchOptions?.firstRunSetup)}
      >
        <Sidebar
          activeItem={activeItem}
          items={sidebarItems}
          onItemSelect={(item) => handleItemSelect(item, { startGlobalChat: true })}
          onLedgerSelect={() => handleItemSelect("ledger")}
          onSettingsSelect={() => handleItemSelect("settings")}
        />

        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {/* Toolbar: draggable strip with global status and the personalization avatar. */}
          <div
            className="flex h-12 w-full shrink-0 items-center justify-end gap-3 px-4"
            data-tauri-drag-region
          >
            <RecommendedModelInstallIndicator onOpenModels={() => handleItemSelect("settings")} />
            {approvals && approvals.pendingApprovalCount > 0 ? (
              <button
                aria-label={t("approvals.open")}
                className="flex items-center gap-2 rounded-full border border-[var(--warning)]/25 bg-[var(--warning-background)] px-3 py-1.5 text-[11px] font-semibold text-[var(--warning)] shadow-sm transition-colors hover:bg-[var(--fill-hover)]"
                onClick={approvals.focusNextApproval}
                type="button"
              >
                <span aria-hidden="true" className="h-2 w-2 rounded-full bg-[var(--warning)]" />
                {t(
                  approvals.pendingApprovalCount === 1
                    ? "approvals.pending_one"
                    : "approvals.pending_many",
                  { count: approvals.pendingApprovalCount },
                )}
              </button>
            ) : null}
            {browserEnvironment.isRuntimeChecked &&
              browserEnvironment.isUncontainedBrowser ? (
              <div className="flex select-none items-center gap-1.5 rounded-full bg-[var(--warning-background)] px-3 py-1.5 text-[11px] font-medium text-[var(--warning)] transition-all">
                <span className="h-2 w-2 rounded-full bg-[var(--warning)]" aria-hidden="true" />
                {t("status.desktop_required")}
              </div>
            ) : isSecureEnvironment ? (
              <div className="flex select-none items-center gap-1.5 rounded-full bg-[var(--accent-background)] px-3 py-1.5 text-[11px] font-medium text-[var(--foreground-muted)] transition-all">
                <LockIcon />
                {t("status.secure")}
              </div>
            ) : isInitializing ? (
              <div className="flex select-none items-center gap-1.5 rounded-full bg-[var(--accent-background)] px-3 py-1.5 text-[11px] font-medium text-[var(--foreground-subtle)] transition-all">
                <svg
                  aria-hidden="true"
                  className="h-3.5 w-3.5 animate-spin text-[var(--foreground-subtle)]"
                  fill="none"
                  viewBox="0 0 24 24"
                >
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                {t("status.connecting")}
              </div>
            ) : (
              <div className="flex select-none items-center gap-1.5 rounded-full bg-[var(--warning-background)] px-3 py-1.5 text-[11px] font-medium text-[var(--warning)] transition-all">
                <span className="h-2 w-2 rounded-full bg-[var(--warning)]" aria-hidden="true" />
                {t("status.setup_needed")}
              </div>
            )}

            {isDeveloperBuild ? (
              <button
                aria-current={activeItem === "developer" ? "page" : undefined}
                aria-label={t("sidebar.developer")}
                className={`flex h-8 w-8 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] transition-colors ${
                  activeItem === "developer"
                    ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
                    : "bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                }`}
                onClick={() => handleItemSelect("developer")}
                type="button"
              >
                <DeveloperIcon />
              </button>
            ) : null}

            <button
              aria-current={activeItem === "user_config" ? "page" : undefined}
              aria-label={t("sidebar.personalization")}
              className={`flex h-8 w-8 items-center justify-center overflow-hidden rounded-full border border-[var(--border-strong)] transition-colors ${
                activeItem === "user_config"
                  ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
                  : "bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
              }`}
              onClick={() => handleItemSelect("user_config")}
              type="button"
            >
              <UserIcon />
            </button>
          </div>

          <div className="shrink-0 px-4 pb-2"><AppControlMonitor /></div>
          <main className="min-h-0 min-w-0 flex-1 overflow-hidden">{children}</main>
        </div>
        <BrowserEnvironmentGuard />
      </div>
      <ApplicationUpdateCoordinator navigationGuard={navGuardRef} presentationBlocked={Boolean(approvals?.pendingApprovalCount)} />
    </AppShellContext.Provider>
  );
}
