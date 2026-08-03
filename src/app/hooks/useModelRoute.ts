"use client";

import { useSyncExternalStore } from "react";
import { invoke } from "@/lib/invoke";

type RoutingPreferenceRecord = {
  key: string;
  value: string;
  updatedAt?: number;
  updated_at?: number;
  routeKey?: string;
  route_key?: string;
  providerId?: string;
  provider_id?: string;
  providerConfigId?: string;
  provider_config_id?: string;
  modelId?: string;
  model_id?: string;
  label?: string;
};

export type ModelRouteSlot = "primary" | "fallback";

export type PersistedModelRoute = {
  providerConfigId: string;
  providerId: string;
  modelId: string;
  label: string;
  updatedAt: number;
};

type ModelRoutingSnapshot = {
  primaryRoute: PersistedModelRoute | null;
  fallbackRoute: PersistedModelRoute | null;
  loaded: boolean;
};

const MODEL_PRIMARY_ROUTE_KEY = "oomu-primary-route";
const MODEL_FALLBACK_ROUTE_KEY = "oomu-fallback-route";

const routingPreferenceListeners = new Set<() => void>();
const defaultRoutingSnapshot: ModelRoutingSnapshot = {
  primaryRoute: null,
  fallbackRoute: null,
  loaded: false,
};
let cachedRoutingSnapshot: ModelRoutingSnapshot = defaultRoutingSnapshot;
let routingPreferencesLoadPromise: Promise<void> | null = null;

function routeKeyForSlot(slot: ModelRouteSlot) {
  return slot;
}

function storageKeyForSlot(slot: ModelRouteSlot) {
  return slot === "primary" ? MODEL_PRIMARY_ROUTE_KEY : MODEL_FALLBACK_ROUTE_KEY;
}

function getRoutingPreferencesSnapshot() {
  return cachedRoutingSnapshot;
}

function getServerRoutingPreferencesSnapshot() {
  return defaultRoutingSnapshot;
}

function subscribeToRoutingPreferences(onStoreChange: () => void) {
  routingPreferenceListeners.add(onStoreChange);

  void hydrateRoutingPreferences();

  return () => {
    routingPreferenceListeners.delete(onStoreChange);
  };
}

function emitRoutingPreferencesChange() {
  routingPreferenceListeners.forEach((listener) => listener());
}

function parsePersistedRoute(value: string | null | undefined): PersistedModelRoute | null {
  if (!value || value === "null") {
    return null;
  }

  try {
    const parsed = JSON.parse(value) as Partial<PersistedModelRoute>;
    const providerConfigId = typeof parsed.providerConfigId === "string" ? parsed.providerConfigId.trim() : "";
    const parsedProviderId = typeof parsed.providerId === "string" ? parsed.providerId.trim() : "";
    const providerId = providerConfigId || parsedProviderId;
    const modelId = typeof parsed.modelId === "string" ? parsed.modelId.trim() : "";
    if (!providerConfigId || !providerId || !modelId) {
      return null;
    }
    return {
      providerConfigId,
      providerId,
      modelId,
      label: typeof parsed.label === "string" && parsed.label.trim() ? parsed.label.trim() : `${providerId} / ${modelId}`,
      updatedAt: typeof parsed.updatedAt === "number" ? parsed.updatedAt : Date.now(),
    };
  } catch {
    return null;
  }
}

function parsePersistedRouteRecord(record: RoutingPreferenceRecord | null | undefined): PersistedModelRoute | null {
  if (!record) {
    return null;
  }
  const parsed = parsePersistedRoute(record.value);
  if (parsed) {
    return parsed;
  }

  const providerConfigId =
    typeof record.providerConfigId === "string"
      ? record.providerConfigId.trim()
      : typeof record.provider_config_id === "string"
        ? record.provider_config_id.trim()
        : "";
  const providerId =
    typeof record.providerId === "string"
      ? record.providerId.trim()
      : typeof record.provider_id === "string"
        ? record.provider_id.trim()
        : providerConfigId;
  const modelId =
    typeof record.modelId === "string"
      ? record.modelId.trim()
      : typeof record.model_id === "string"
        ? record.model_id.trim()
        : "";
  if (!providerConfigId || !providerId || !modelId) {
    return null;
  }

  return {
    providerConfigId,
    providerId,
    modelId,
    label:
      typeof record.label === "string" && record.label.trim()
        ? record.label.trim()
        : `${providerId} / ${modelId}`,
    updatedAt:
      typeof record.updatedAt === "number"
        ? record.updatedAt
        : typeof record.updated_at === "number"
          ? record.updated_at
          : Date.now(),
  };
}

function routeIdForPersistedRoute(route: PersistedModelRoute | null) {
  if (!route) {
    return null;
  }
  return `${route.providerConfigId}:${route.modelId}`;
}

async function hydrateRoutingPreferences() {
  if (cachedRoutingSnapshot.loaded) {
    return;
  }

  if (!routingPreferencesLoadPromise) {
    routingPreferencesLoadPromise = Promise.all([
      invoke<RoutingPreferenceRecord | null>("get_routing_preference", {
        routeKey: routeKeyForSlot("primary"),
      }),
      invoke<RoutingPreferenceRecord | null>("get_routing_preference", {
        routeKey: routeKeyForSlot("fallback"),
      }),
    ])
      .then(([primary, fallback]) => {
        cachedRoutingSnapshot = {
          primaryRoute: parsePersistedRouteRecord(primary),
          fallbackRoute: parsePersistedRouteRecord(fallback),
          loaded: true,
        };
        emitRoutingPreferencesChange();
      })
      .catch(() => {
        cachedRoutingSnapshot = {
          ...cachedRoutingSnapshot,
          loaded: true,
        };
        emitRoutingPreferencesChange();
      })
      .finally(() => {
        routingPreferencesLoadPromise = null;
      });
  }

  await routingPreferencesLoadPromise;
}

export function useModelRoutingPreferences() {
  const snapshot = useSyncExternalStore(
    subscribeToRoutingPreferences,
    getRoutingPreferencesSnapshot,
    getServerRoutingPreferencesSnapshot,
  );

  function setRoutePreference(slot: ModelRouteSlot, route: PersistedModelRoute | null) {
    const nextSnapshot =
      slot === "primary"
        ? { ...cachedRoutingSnapshot, primaryRoute: route, loaded: true }
        : { ...cachedRoutingSnapshot, fallbackRoute: route, loaded: true };
    cachedRoutingSnapshot = nextSnapshot;
    emitRoutingPreferencesChange();

    const save = route
      ? invoke<void>("save_routing_preference", {
          routeKey: routeKeyForSlot(slot),
          modelId: route.modelId,
          providerId: route.providerId,
          providerConfigId: route.providerConfigId,
          label: route.label,
        })
      : invoke<void>("set_routing_preference", {
          key: storageKeyForSlot(slot),
          value: "null",
        });
    void save.catch(() => {
      // Keep the responsive UI value; native errors surface in the console through invoke().
    });

    const primaryRouteId = routeIdForPersistedRoute(nextSnapshot.primaryRoute);
    const fallbackRouteId = routeIdForPersistedRoute(nextSnapshot.fallbackRoute);
    if (primaryRouteId && fallbackRouteId) {
      void invoke<void>("save_routing_preference", {
        primaryRouteId,
        primary_route_id: primaryRouteId,
        fallbackRouteId,
        fallback_route_id: fallbackRouteId,
      }).catch(() => {
        // Structured slot persistence above remains authoritative if pair mirroring fails.
      });
    }
  }

  return {
    ...snapshot,
    setRoutePreference,
  };
}
