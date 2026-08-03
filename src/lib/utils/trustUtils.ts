"use client";

import { useI18n } from "@/context/I18nContext";

function humanizeEnum(value: string) {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function lookupTrustLabel(
  t: (key: string) => string,
  key: string,
  fallback: string,
) {
  const localized = t(key);
  return localized !== key ? localized : humanizeEnum(fallback);
}

export function useHumanTrust() {
  const { t } = useI18n();

  const getToolKindLabel = (kind: string): string => {
    const normalized = kind.toLowerCase();
    return lookupTrustLabel(t, `trust.tool_kind.${normalized}`, kind);
  };

  const getRiskLevelLabel = (risk: string): string => {
    const normalized = risk.toLowerCase();
    return lookupTrustLabel(t, `trust.risk_level.${normalized}`, risk);
  };

  const getPhaseLabel = (phase: string): string => {
    const normalized = phase.toLowerCase();
    return lookupTrustLabel(t, `trust.phases.${normalized}`, phase);
  };

  const getPermissionLabel = (permission: string): string => {
    const normalized = permission.toLowerCase();
    return lookupTrustLabel(t, `trust.permissions.${normalized}_label`, permission);
  };

  return {
    getToolKindLabel,
    getRiskLevelLabel,
    getPhaseLabel,
    getPermissionLabel,
  };
}
