import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ProjectsScreen } from "./ProjectsScreen";
import type { ProjectRecord } from "./projectClient";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const alpha = {
  projectId: "project_11111111-1111-4111-8111-111111111111",
  name: "Alpha",
  description: "",
  dataPolicy: "local_only",
  instructions: "",
  archivedAtMs: null,
  createdAtMs: 1,
  updatedAtMs: 2,
  sourceCount: 0,
  conversationCount: 1,
  workflowCount: 2,
  taskCount: 3,
} as ProjectRecord;

const beta = {
  ...alpha,
  projectId: "project_22222222-2222-4222-8222-222222222222",
  name: "Beta",
  updatedAtMs: 1,
} as ProjectRecord;

const preview = {
  projectId: alpha.projectId,
  conversationsToDetach: 1,
  workflowsToDetach: 2,
  schedulesToDetach: 0,
  taskRunsToDetach: 3,
  sourcesToRemove: 0,
  userFilesToDelete: 4,
  defaultAction: "permanent_delete",
};

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }],
    translations: {},
  };
}

function renderProjects(onOpenChat = vi.fn(), onOpenWorkflows = vi.fn()) {
  return render(<ProjectsScreen onOpenChat={onOpenChat} onOpenWorkflows={onOpenWorkflows} />, { wrapper: I18nProvider });
}

describe("ProjectsScreen permanent deletion", () => {
  let projects: ProjectRecord[];

  beforeEach(() => {
    projects = [alpha, beta];
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return projects;
      if (command === "list_project_sources") return [];
      if (command === "get_project_memory_summary") return { memoryCount: 0, sourceSessions: [] };
      if (command === "preview_project_deletion") return preview;
      if (command === "delete_project") {
        const request = args?.request as { projectId?: string } | undefined;
        projects = projects.filter((project) => project.projectId !== request?.projectId);
        return preview;
      }
      return null;
    });
  });

  afterEach(cleanup);

  it("keeps the Project definition visible once projects exist", async () => {
    const { container } = renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });

    expect(screen.getByText("One place per case, matter, client, or job — everything about it (conversations, files, instructions, privacy) lives together.")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Projects" })).toBeNull();
    expect(container.querySelector("section")).toHaveClass("grid-cols-[19rem_minmax(0,1fr)]");
  });

  it("opens Chat in the exact selected Project", async () => {
    const onOpenChat = vi.fn();
    renderProjects(onOpenChat);
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Open conversations" }));

    expect(onOpenChat).toHaveBeenCalledWith(alpha.projectId);
  });

  it("opens Workflow authoring in the exact selected Project", async () => {
    const onOpenWorkflows = vi.fn();
    renderProjects(vi.fn(), onOpenWorkflows);
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Open workflows" }));

    expect(onOpenWorkflows).toHaveBeenCalledWith({
      projectId: alpha.projectId,
      projectName: alpha.name,
    });
  });

  it("presents cloud use as a separated privacy choice with a recommended default", async () => {
    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "New" }));

    const policy = screen.getByRole("combobox", { name: "Can OOMU use cloud AI for this work?" });
    expect(policy).toHaveValue("ask_before_cloud");
    expect(within(policy).getByRole("option", { name: "No — keep everything on my Mac" })).toBeVisible();
    expect(within(policy).getByRole("option", { name: "Ask me first (recommended)" })).toBeVisible();
    expect(within(policy).getByRole("option", { name: "Yes — use the cloud models I've set up" })).toBeVisible();
    expect(screen.getByText("Cloud AI is more powerful; on-device keeps this work fully private. You can change this anytime.")).toBeVisible();
    expect(screen.queryByText("Data policy")).toBeNull();
    expect(policy.closest("div")).toHaveClass("bg-[var(--accent-background)]");
  });

  it("keeps an empty knowledge folder attached and explains the next startup scan", async () => {
    const emptySource = {
      sourceId: "source_33333333-3333-4333-8333-333333333333",
      projectId: alpha.projectId,
      sourceKind: "knowledge_directory",
      canonicalPath: "/Users/example/Empty Knowledge",
      grantState: "active",
      indexingState: "ready",
      fileCount: 0,
      lastIndexedAtMs: 1,
      failureCode: null,
      updatedAtMs: 1,
    };
    let sources: typeof emptySource[] = [];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return projects;
      if (command === "list_project_sources") return sources;
      if (command === "get_project_memory_summary") return { memoryCount: 0, sourceSessions: [] };
      if (command === "choose_knowledge_ingest_directory") {
        return {
          grantId: "a".repeat(64),
          directoryName: "Empty Knowledge",
          canonicalPath: emptySource.canonicalPath,
          fileCount: 0,
        };
      }
      if (command === "attach_project_source") {
        sources = [{ ...emptySource, indexingState: "pending" }];
        return sources[0];
      }
      if (command === "ingest_knowledge") {
        return { indexedFiles: 0, indexedChunks: 0, skippedFiles: 0 };
      }
      if (command === "refresh_project_source") {
        sources = [emptySource];
        return emptySource;
      }
      return null;
    });

    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    expect(screen.getByText(/Up to 512 KB per file, 240 files, or 20 MB per folder/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    expect(await screen.findByText("Folder added. It is empty now; OOMU will check it again the next time it starts.")).toBeVisible();
    expect(screen.getByText("This folder is empty. Files you add will appear here the next time OOMU starts.")).toBeVisible();
    expect(screen.queryByText(/knowledge_invalid_request/)).toBeNull();
  });

  it("warns explicitly, starts on Cancel, and closes without deleting", async () => {
    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    const trigger = screen.getByRole("button", { name: "Delete Project…" });
    fireEvent.click(trigger);

    const dialog = await screen.findByRole("dialog", { name: "Delete “Alpha” permanently?" });
    expect(within(dialog).getByText("This permanently deletes the Project and files OOMU created for it. This can’t be undone.")).toBeVisible();
    expect(within(dialog).getByText("Linked source folders and files exported elsewhere stay on your Mac.")).toBeVisible();
    expect(within(dialog).getByText("Conversations, workflows, and task history stay in OOMU without this Project.")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(invokeMock.mock.calls.filter(([command]) => command === "preview_project_deletion")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "delete_project")).toHaveLength(0);

    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(invokeMock.mock.calls.filter(([command]) => command === "delete_project")).toHaveLength(0);
  });

  it("permanently deletes only after confirmation and opens the next Project", async () => {
    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Delete Project…" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete “Alpha” permanently?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete Project" }));

    await screen.findByRole("heading", { name: "Beta" });
    expect(screen.getByText("Project deleted.")).toBeVisible();
    expect(invokeMock.mock.calls.filter(([command]) => command === "delete_project")).toEqual([["delete_project", {
      request: {
        projectId: alpha.projectId,
        permanentlyRemoveProjectRecord: true,
        detachDependents: true,
        deleteProjectFiles: true,
      },
    }]]);
  });

  it("keeps the confirmation open and reports a clear failure", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return projects;
      if (command === "list_project_sources") return [];
      if (command === "get_project_memory_summary") return { memoryCount: 0, sourceSessions: [] };
      if (command === "preview_project_deletion") return preview;
      if (command === "delete_project") throw new Error("BACKEND DELETE CANARY");
      return null;
    });
    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Delete Project…" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete “Alpha” permanently?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete Project" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("OOMU couldn’t delete this Project. Try again.");
    expect(screen.getByRole("dialog", { name: "Delete “Alpha” permanently?" })).toBeVisible();
    expect(screen.queryByText("BACKEND DELETE CANARY")).toBeNull();
  });

  it("locks both choices while permanent deletion is running", async () => {
    let finishDelete!: () => void;
    const pending = new Promise<void>((resolve) => { finishDelete = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return projects;
      if (command === "list_project_sources") return [];
      if (command === "get_project_memory_summary") return { memoryCount: 0, sourceSessions: [] };
      if (command === "preview_project_deletion") return preview;
      if (command === "delete_project") { await pending; projects = [beta]; return preview; }
      return null;
    });
    renderProjects();
    await screen.findByRole("heading", { name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Delete Project…" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete “Alpha” permanently?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete Project" }));

    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Deleting…" })).toBeDisabled();
    expect(dialog).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeVisible();

    await act(async () => finishDelete());
    await screen.findByRole("heading", { name: "Beta" });
  });
});
