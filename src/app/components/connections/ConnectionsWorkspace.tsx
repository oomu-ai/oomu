"use client";

import type { ConnectionsSection } from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { ChannelsDashboard } from "../ChannelsDashboard";
import { IntegrationsScreen } from "../integrations/IntegrationsScreen";

const sections: readonly ConnectionsSection[] = ["work_apps", "messaging"];

export function ConnectionsWorkspace({
  activeSection,
  onSectionChange,
}: {
  activeSection: ConnectionsSection;
  onSectionChange: (section: ConnectionsSection) => void;
}) {
  const { t } = useI18n();

  return (
    <section
      aria-label={t("sidebar.connections")}
      className="flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="shrink-0 border-b border-[var(--border-soft)] px-6 py-3">
        <div
          aria-label={t("connections.sections")}
          className="inline-flex rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-0.5"
          role="tablist"
        >
          {sections.map((section) => {
            const selected = section === activeSection;
            return (
              <button
                aria-selected={selected}
                className={`rounded-[var(--radius-sm)] px-4 py-1.5 text-sm font-medium transition-colors ${
                  selected
                    ? "bg-[var(--background)] text-[var(--foreground)] shadow-[var(--shadow-card)]"
                    : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"
                }`}
                key={section}
                onClick={() => onSectionChange(section)}
                role="tab"
                type="button"
              >
                {t(`connections.section_${section}`)}
              </button>
            );
          })}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {activeSection === "work_apps" ? (
          <IntegrationsScreen
            onTurnOnMessaging={() => onSectionChange("messaging")}
            showIntroduction={false}
          />
        ) : (
          <ChannelsDashboard />
        )}
      </div>
    </section>
  );
}
