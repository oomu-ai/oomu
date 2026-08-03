import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TasksWorkspace } from "./TasksWorkspace";

let workflowsView: "composer" | "saved_workflows" = "composer";

vi.mock("@/components/AppShell", () => ({
  useAppShell: () => ({ workflowsView }),
}));

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) =>
      ({
        "sidebar.tasks": "Tasks",
        "tasks.sections": "Task views",
        "tasks.section_now": "Now",
        "tasks.section_scheduled": "Scheduled",
        "tasks.section_workflows": "Build",
      })[key] ?? key,
  }),
}));

vi.mock("./TaskCenter", () => ({
  TaskCenter: ({ onStartInChat, showIntroduction }: { onStartInChat?: () => void; showIntroduction?: boolean }) => (
    <div>
      <span>now:{String(showIntroduction)}</span>
      {onStartInChat ? <button onClick={onStartInChat} type="button">mock-chat</button> : null}
    </div>
  ),
}));

vi.mock("../routines/RoutinesScreen", () => ({
  RoutinesScreen: ({ showIntroduction }: { showIntroduction?: boolean }) => (
    <div>scheduled:{String(showIntroduction)}</div>
  ),
}));

vi.mock("../WorkflowComposer", () => ({
  WorkflowComposer: ({ requestedTemplateId, requestedTemplateSourceFolder }: {
    requestedTemplateId?: string;
    requestedTemplateSourceFolder?: { folderName: string } | null;
  }) => (
    <div>composer:{requestedTemplateId}:{requestedTemplateSourceFolder?.folderName}</div>
  ),
}));

vi.mock("../SavedWorkflows", () => ({
  SavedWorkflows: () => <div>saved-workflows</div>,
}));

afterEach(cleanup);

describe("TasksWorkspace", () => {
  it("mounts Now and Scheduled without repeated introductions", () => {
    const onSectionChange = vi.fn();
    const { rerender } = render(
      <TasksWorkspace activeSection="now" onSectionChange={onSectionChange} />,
    );

    expect(screen.getByText("now:false")).toBeVisible();
    fireEvent.click(screen.getByRole("tab", { name: "Scheduled" }));
    expect(onSectionChange).toHaveBeenCalledWith("scheduled");

    rerender(
      <TasksWorkspace activeSection="scheduled" onSectionChange={onSectionChange} />,
    );
    expect(screen.getByText("scheduled:false")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Tasks" })).toBeNull();
  });

  it("passes the Chat starting path into Now", () => {
    const onStartInChat = vi.fn();
    render(
      <TasksWorkspace
        activeSection="now"
        onSectionChange={vi.fn()}
        onStartInChat={onStartInChat}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "mock-chat" }));
    expect(onStartInChat).toHaveBeenCalledTimes(1);
  });

  it("opens an externally requested starter template in Workflows", () => {
    workflowsView = "composer";
    render(
      <TasksWorkspace
        activeSection="workflows"
        onSectionChange={vi.fn()}
        requestedTemplateId="directory-summarizer"
        requestedTemplateSourceFolder={{
          fileCount: 3,
          folderName: "Case files",
          folderPath: "workspace/selections/selection-case-files",
          totalBytes: 1200,
          truncated: false,
        }}
      />,
    );

    expect(screen.getByText("composer:directory-summarizer:Case files")).toBeVisible();
    expect(screen.getByRole("tab", { name: "Build" })).toHaveAttribute("aria-selected", "true");
  });

  it("preserves the saved workflow view", () => {
    workflowsView = "saved_workflows";
    render(
      <TasksWorkspace activeSection="workflows" onSectionChange={vi.fn()} />,
    );

    expect(screen.getByText("saved-workflows")).toBeVisible();
    workflowsView = "composer";
  });
});
