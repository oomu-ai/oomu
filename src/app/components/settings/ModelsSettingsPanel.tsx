"use client";

import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { ModelsScreen } from "../ModelsScreen";

type ModelsSettingsPanelProps = {
  configuredProviders: ConfiguredProvider[];
  onConfiguredProvidersChange: (providers: ConfiguredProvider[]) => void;
};

export function ModelsSettingsPanel({
  configuredProviders,
  onConfiguredProvidersChange,
}: ModelsSettingsPanelProps) {
  return (
    <ModelsScreen
      configuredProviders={configuredProviders}
      onConfiguredProvidersChange={onConfiguredProvidersChange}
    />
  );
}
