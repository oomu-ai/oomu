import type {
  AppSection,
  ConnectionsSection,
  TasksSection,
} from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { useState } from "react";
import type { ChatStarterAction } from "./components/ChatScreen";
import { useDecisionBriefCompletion } from "./components/chat/firstRunWelcomeState";
import type { WorkflowTemplateId } from "./components/workflowLibrary";
import {
  chooseWorkflowSourceFolder,
  type WorkflowSourceFolder,
} from "./components/workflowSourceFolder";

export function resolveHeroDestination(destination: string): AppSection {
  switch (destination) {
    case "projects":
    case "integrations":
    case "settings":
    case "tasks":
    case "artifacts":
    case "routines":
      return destination;
    default:
      return "settings";
  }
}

export function resolveChatStarterDestination(action: ChatStarterAction): {
  item: AppSection;
  templateId: WorkflowTemplateId | null;
} {
  switch (action) {
    case "weekly_brief":
      return { item: "hero", templateId: null };
    case "summarize_folder":
      return { item: "workflows", templateId: "directory-summarizer" };
    case "help_with_email":
      return { item: "workflows", templateId: "email-responder" };
  }
}

export function destinationForTasksSection(section: TasksSection): AppSection {
  return section === "now"
    ? "tasks"
    : section === "scheduled"
      ? "routines"
      : "workflows";
}

export function destinationForConnectionsSection(
  section: ConnectionsSection,
): AppSection {
  return section === "work_apps" ? "integrations" : "channels";
}

export function useHomeWorkspaceNavigation(
  setActiveItem: (item: AppSection) => void,
) {
  const { t } = useI18n();
  const decisionBriefCompletion = useDecisionBriefCompletion();
  const [requestedWorkflowTemplateId, setRequestedWorkflowTemplateId] =
    useState<WorkflowTemplateId | null>(null);
  const [requestedWorkflowSourceFolder, setRequestedWorkflowSourceFolder] =
    useState<WorkflowSourceFolder | null>(null);

  return {
    decisionBriefCompletion,
    async handleChatStarterAction(action: ChatStarterAction) {
      let sourceFolder: WorkflowSourceFolder | null = null;
      if (action === "summarize_folder") {
        sourceFolder = await chooseWorkflowSourceFolder({
          title: t("workflows.templates.directory-summarizer.picker_title"),
          truncationNotice: t(
            "workflows.templates.directory-summarizer.truncation_file_notice",
          ),
        });
        if (!sourceFolder) return false;
      }
      const destination = resolveChatStarterDestination(action);
      setRequestedWorkflowSourceFolder(sourceFolder);
      setRequestedWorkflowTemplateId(destination.templateId);
      setActiveItem(destination.item);
      return true;
    },
    handleConnectionsSectionChange(section: ConnectionsSection) {
      setActiveItem(destinationForConnectionsSection(section));
    },
    handleRequestedWorkflowTemplateLoaded() {
      setRequestedWorkflowTemplateId(null);
      setRequestedWorkflowSourceFolder(null);
    },
    handleTasksSectionChange(section: TasksSection) {
      setActiveItem(destinationForTasksSection(section));
    },
    requestedWorkflowTemplateId,
    requestedWorkflowSourceFolder,
  };
}
