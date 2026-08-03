"use client";

import { useAppShell, type TasksSection } from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { RoutinesScreen } from "../routines/RoutinesScreen";
import { SavedWorkflows } from "../SavedWorkflows";
import {
  WorkflowComposer,
  type WorkflowComposerProps,
} from "../WorkflowComposer";
import type { WorkflowSourceFolder } from "../workflowSourceFolder";
import { TaskCenter } from "./TaskCenter";

const sections: readonly TasksSection[] = ["now", "scheduled", "workflows"];

export function TasksWorkspace({
  activeSection,
  onRequestedTemplateLoaded,
  onSectionChange,
  onStartInChat,
  requestedTemplateId,
  requestedTemplateSourceFolder,
}: {
  activeSection: TasksSection;
  onRequestedTemplateLoaded?: WorkflowComposerProps["onRequestedTemplateLoaded"];
  onSectionChange: (section: TasksSection) => void;
  onStartInChat?: () => void;
  requestedTemplateId?: WorkflowComposerProps["requestedTemplateId"];
  requestedTemplateSourceFolder?: WorkflowSourceFolder | null;
}) {
  const { workflowsView } = useAppShell();
  const { t } = useI18n();

  return (
    <section
      aria-label={t("sidebar.tasks")}
      className="flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="shrink-0 border-b border-[var(--border-soft)] px-6 py-3">
        <div
          aria-label={t("tasks.sections")}
          className="inline-flex rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-0.5"
          role="tablist"
        >
          {sections.map((section) => {
            const selected = section === activeSection;
            return (
              <div
                className={section === "workflows" ? "ml-1 border-l border-[var(--border-strong)] pl-2" : ""}
                key={section}
                role="presentation"
              >
                <button
                  aria-selected={selected}
                  className={`rounded-[var(--radius-sm)] px-4 py-1.5 text-sm font-medium transition-colors ${
                    selected
                      ? "bg-[var(--background)] text-[var(--foreground)] shadow-[var(--shadow-card)]"
                      : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"
                  }`}
                  onClick={() => onSectionChange(section)}
                  role="tab"
                  type="button"
                >
                  {t(`tasks.section_${section}`)}
                </button>
              </div>
            );
          })}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {activeSection === "now" ? (
          <TaskCenter onStartInChat={onStartInChat} showIntroduction={false} />
        ) : activeSection === "scheduled" ? (
          <RoutinesScreen showIntroduction={false} />
        ) : (
          <div className="flex h-full min-h-0 flex-col">
            {workflowsView === "saved_workflows" ? (
              <SavedWorkflows />
            ) : (
              <WorkflowComposer
                onRequestedTemplateLoaded={onRequestedTemplateLoaded}
                requestedTemplateId={requestedTemplateId}
                requestedTemplateSourceFolder={requestedTemplateSourceFolder}
              />
            )}
          </div>
        )}
      </div>
    </section>
  );
}
