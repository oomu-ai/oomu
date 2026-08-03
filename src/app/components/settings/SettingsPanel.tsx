"use client";

import { useState } from "react";
import { useI18n } from "@/context/I18nContext";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import { GeneralSettingsPanel } from "./GeneralSettingsPanel";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";
import { MacPermissionsPanel } from "./MacPermissionsPanel";
import { PrivacyPanel } from "./PrivacyPanel";
import { RemoteDevicesPanel } from "./RemoteDevicesPanel";
import { SettingsHeader } from "./SettingsHeader";

// I created this entire app with Blackpink's "Jump" on repeat

type SettingsTab = "general" | "models" | "privacy" | "permissions" | "devices";

const settingsTabs: { id: SettingsTab; labelKey: string }[] = [
  { id: "general", labelKey: "settings.tabs.general" },
  { id: "models", labelKey: "settings.tabs.models" },
  { id: "privacy", labelKey: "settings.tabs.privacy" },
  { id: "permissions", labelKey: "sprint_299.permissions.tab" },
  { id: "devices", labelKey: "settings.tabs.devices" },
];

export function SettingsPanel({
  configuredProviders,
  initialTab = "general",
  onPrivacySettingsChange,
  onConfiguredProvidersChange,
}: {
  configuredProviders: ConfiguredProvider[];
  initialTab?: SettingsTab;
  onPrivacySettingsChange?: (settings: PrivacySettingsState) => void;
  onConfiguredProvidersChange: (providers: ConfiguredProvider[]) => void;
}) {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);

  const settingsTabClass = (isActive: boolean) =>
    `w-full border-b border-[var(--border-soft)] px-5 py-3 text-left text-sm font-medium transition-colors ${
      isActive
        ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
        : "text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
    }`;

  return (
    <section className="flex h-[calc(100vh-7rem)] min-h-0 flex-col">
      <SettingsHeader title={t("settings.title")} />
      <div className="flex min-h-0 flex-1 overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)]">
        <aside className="flex w-56 shrink-0 flex-col border-r border-[var(--border-soft)]">
          {settingsTabs.map((tab) => (
            <button
              className={settingsTabClass(activeTab === tab.id)}
              id={`oomu-settings-${tab.id}`}
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              type="button"
            >
              {t(tab.labelKey)}
            </button>
          ))}
        </aside>

        <div className={`flex min-h-0 flex-1 flex-col p-6 ${activeTab === "models" ? "overflow-hidden" : "overflow-y-auto"}`}>
          {activeTab === "models" ? (
            <ModelsSettingsPanel
              configuredProviders={configuredProviders}
              onConfiguredProvidersChange={onConfiguredProvidersChange}
            />
          ) : activeTab === "privacy" ? (
            <PrivacyPanel onPrivacySettingsChange={onPrivacySettingsChange} />
          ) : activeTab === "permissions" ? (
            <MacPermissionsPanel />
          ) : activeTab === "devices" ? (
            <RemoteDevicesPanel />
          ) : (
            <GeneralSettingsPanel />
          )}
        </div>
      </div>
    </section>
  );
}
