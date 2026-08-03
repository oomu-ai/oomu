"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import type { BrowserSplitRoute } from "./browserRouting";

type NavigationStatus =
  | "awaiting_consent"
  | "authorizing"
  | "opening"
  | "ready"
  | "blocked"
  | "continuing";
type BlockedReason =
  | "desktop"
  | "route_unavailable"
  | "command_unavailable"
  | "denied"
  | "timeout"
  | "policy"
  | "open"
  | "cancelled"
  | "reload";

export type RecoverableBrowserFailure = Extract<
  BlockedReason,
  "route_unavailable" | "command_unavailable" | "timeout" | "open"
>;

export type BrowserResearchRouteUnavailableHandler = (
  route: BrowserSplitRoute,
  reason: RecoverableBrowserFailure,
) => Promise<boolean>;

type NativeBrowserBridgeReady = {
  status: "ready";
  canonicalUrl: string;
};

type NativeBrowserAuthorization = {
  approvalToken: string;
  canonicalUrl: string;
  canonicalOrigin: string;
  destinationBinding: string;
  expiresAtMs: number;
};

const BROWSER_BRIDGE_ACK_TIMEOUT_MS = 5_000;
const BROWSER_ERROR_REASONS: Partial<Record<string, BlockedReason>> = {
  browser_route_unavailable: "route_unavailable",
  browser_command_unavailable: "command_unavailable",
  browser_authorization_denied: "denied",
  browser_dispatch_timeout: "timeout",
  browser_navigation_blocked: "policy",
  browser_cancelled: "cancelled",
  browser_native_open_failed: "open",
};
const RECOVERABLE_BROWSER_FAILURES = new Set<BlockedReason>([
  "route_unavailable",
  "command_unavailable",
  "timeout",
  "open",
]);

function recoverableResearchHandler(
  notificationSent: boolean,
  handler: BrowserResearchRouteUnavailableHandler | undefined,
  reason: BlockedReason,
) {
  if (notificationSent || !handler || !RECOVERABLE_BROWSER_FAILURES.has(reason)) {
    return null;
  }
  return handler;
}

function bridgeErrorReason(error: unknown): BlockedReason {
  let normalizedError = error;
  if (typeof error === "string") {
    try {
      normalizedError = JSON.parse(error);
    } catch {
      normalizedError = error;
    }
  }
  const code = normalizedError && typeof normalizedError === "object" && "code" in normalizedError
    ? String(normalizedError.code)
    : "";
  return BROWSER_ERROR_REASONS[code] ?? "open";
}

function withBridgeAcknowledgement<T>(operation: Promise<T>) {
  return new Promise<T>((resolve, reject) => {
    const timeout = window.setTimeout(
      () => reject({ code: "browser_dispatch_timeout" }),
      BROWSER_BRIDGE_ACK_TIMEOUT_MS,
    );
    operation.then(
      (result) => {
        window.clearTimeout(timeout);
        resolve(result);
      },
      (error) => {
        window.clearTimeout(timeout);
        reject(error);
      },
    );
  });
}

async function openNativeBrowserNavigation({
  url,
  resolveHost,
  routeIsCurrent,
  setNavigationStatus,
}: {
  url: string;
  resolveHost: () => HTMLDivElement | null;
  routeIsCurrent: () => boolean;
  setNavigationStatus: (status: NavigationStatus) => void;
}) {
  const authorization = await invoke<NativeBrowserAuthorization>(
    "authorize_native_browser_navigation",
    { url },
  );
  if (!routeIsCurrent()) {
    return null;
  }
  const host = resolveHost();
  if (!host) {
    throw new Error("The secure browser panel is no longer available.");
  }
  setNavigationStatus("opening");
  const rect = host.getBoundingClientRect();
  const ready = await withBridgeAcknowledgement(
    invoke<NativeBrowserBridgeReady>("open_authorized_native_browser", {
      approvalToken: authorization.approvalToken,
      bounds: {
        x: Math.max(0, Math.round(rect.left)),
        y: Math.max(0, Math.round(rect.top)),
        width: Math.max(1, Math.round(rect.width)),
        height: Math.max(1, Math.round(rect.height)),
      },
    }),
  );
  if (ready.status !== "ready") {
    throw { code: "browser_native_open_failed" };
  }
  if (!routeIsCurrent()) {
    void invoke("close_native_browser").catch(() => undefined);
    return null;
  }
  return ready.canonicalUrl || authorization.canonicalUrl;
}

function activateNativeBrowserRoute(
  activeRouteGenerationRef: { current: string | null },
  routeGeneration: string,
) {
  activeRouteGenerationRef.current = routeGeneration;
  if (isTauriRuntime) {
    void invoke("close_native_browser").catch(() => undefined);
  }
  return () => {
    activeRouteGenerationRef.current = null;
    if (isTauriRuntime) {
      void invoke("close_native_browser").catch(() => undefined);
    }
  };
}

function RefreshIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
      <path d="M21 12a9 9 0 0 1-15.4 6.4L3 16" />
      <path d="M3 21v-5h5" />
      <path d="M3 12A9 9 0 0 1 18.4 5.6L21 8" />
      <path d="M16 8h5V3" />
    </svg>
  );
}

type BrowserNavigationGateProps = {
  blockedReason: BlockedReason;
  hostLabel: string;
  navigationStatus: NavigationStatus;
  onConfirmNavigation: () => void;
  t: (key: string) => string;
};

function browserStatusTitle(
  navigationStatus: NavigationStatus,
  stoppedByChoice: boolean,
  t: (key: string) => string,
) {
  if (navigationStatus === "continuing") {
    return t("chat.browser.continuing_title");
  }
  if (navigationStatus === "blocked") {
    return t(stoppedByChoice ? "chat.browser.stopped_title" : "chat.browser.blocked_title");
  }
  return t("chat.browser.consent_title");
}

function browserStatusHelp(
  navigationStatus: NavigationStatus,
  blockedReason: BlockedReason,
  t: (key: string) => string,
) {
  if (navigationStatus === "continuing") {
    return t("chat.browser.continuing_help");
  }
  if (navigationStatus === "blocked") {
    return t(`chat.browser.blocked_${blockedReason}`);
  }
  return t("chat.browser.consent_help");
}

function browserActionLabel(
  navigationStatus: NavigationStatus,
  t: (key: string) => string,
) {
  switch (navigationStatus) {
    case "authorizing":
      return t("chat.browser.waiting_for_approval");
    case "opening":
      return t("chat.browser.opening");
    case "blocked":
      return t("chat.browser.retry");
    default:
      return t("chat.browser.open_secure");
  }
}

function BrowserNavigationGate({
  blockedReason,
  hostLabel,
  navigationStatus,
  onConfirmNavigation,
  t,
}: BrowserNavigationGateProps) {
  if (navigationStatus === "ready") {
    return null;
  }

  const stoppedByChoice = blockedReason === "denied" || blockedReason === "cancelled";
  const continuingHeadlessly = navigationStatus === "continuing";

  return (
    <div className="flex h-full min-h-48 items-center justify-center p-6">
      <div className="max-w-md rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5 text-center shadow-sm">
        <p className="text-sm font-semibold text-[var(--foreground)]">
          {browserStatusTitle(navigationStatus, stoppedByChoice, t)}
        </p>
        <p className="mt-2 break-words text-sm font-medium text-[var(--foreground-muted)]">
          {hostLabel}
        </p>
        <p className="mt-3 text-xs leading-5 text-[var(--foreground-subtle)]">
          {browserStatusHelp(navigationStatus, blockedReason, t)}
        </p>
        {!stoppedByChoice && !continuingHeadlessly ? (
          <button
            className="mt-4 inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-60"
            disabled={navigationStatus === "authorizing" || navigationStatus === "opening"}
            onClick={onConfirmNavigation}
            type="button"
          >
            {browserActionLabel(navigationStatus, t)}
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function BrowserModPanel({
  route,
  onResearchRouteUnavailable,
}: {
  route: BrowserSplitRoute;
  onResearchRouteUnavailable?: BrowserResearchRouteUnavailableHandler;
}) {
  const routeGeneration = `${route.messageId}:${route.url}`;
  return (
    <BrowserModPanelRoute
      key={routeGeneration}
      onResearchRouteUnavailable={onResearchRouteUnavailable}
      route={route}
      routeGeneration={routeGeneration}
    />
  );
}

type BrowserModPanelRouteProps = {
  route: BrowserSplitRoute;
  routeGeneration: string;
  onResearchRouteUnavailable?: BrowserResearchRouteUnavailableHandler;
};

function BrowserModPanelRoute({
  route,
  routeGeneration,
  onResearchRouteUnavailable,
}: BrowserModPanelRouteProps) {
  const { t } = useI18n();
  const previewRef = useRef<HTMLDivElement>(null);
  const activeRouteGenerationRef = useRef<string | null>(routeGeneration);
  const fallbackNotificationSentRef = useRef(false);
  const [navigationStatus, setNavigationStatus] = useState<NavigationStatus>("awaiting_consent");
  const [approvedUrl, setApprovedUrl] = useState<string | null>(null);
  const [blockedReason, setBlockedReason] = useState<BlockedReason>("open");
  const hostLabel = useMemo(
    () => safeBrowserDestination(approvedUrl ?? route.url, t("common.unknown")),
    [approvedUrl, route.url, t],
  );

  useEffect(
    () => activateNativeBrowserRoute(activeRouteGenerationRef, routeGeneration),
    [routeGeneration],
  );

  useEffect(() => {
    if (!isTauriRuntime || navigationStatus !== "ready") {
      return;
    }
    const host = previewRef.current;
    if (!host) {
      return;
    }
    const syncBounds = () => {
      const rect = host.getBoundingClientRect();
      void invoke("resize_native_browser", {
        bounds: {
          x: Math.max(0, Math.round(rect.left)),
          y: Math.max(0, Math.round(rect.top)),
          width: Math.max(1, Math.round(rect.width)),
          height: Math.max(1, Math.round(rect.height)),
        },
      }).catch(() => undefined);
    };
    const observer = new ResizeObserver(syncBounds);
    observer.observe(host);
    window.addEventListener("resize", syncBounds);
    const frame = window.requestAnimationFrame(syncBounds);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", syncBounds);
      window.cancelAnimationFrame(frame);
    };
  }, [navigationStatus]);

  const confirmNavigation = useCallback(async () => {
    if (!route.url || navigationStatus === "authorizing" || navigationStatus === "opening") {
      return;
    }
    setBlockedReason("open");
    if (!isTauriRuntime) {
      setApprovedUrl(null);
      setNavigationStatus("blocked");
      setBlockedReason("desktop");
      return;
    }
    const requestedRouteGeneration = routeGeneration;
    const routeIsCurrent = () => activeRouteGenerationRef.current === requestedRouteGeneration;
    try {
      setNavigationStatus("authorizing");
      const canonicalUrl = await openNativeBrowserNavigation({
        url: route.url,
        resolveHost: () => previewRef.current,
        routeIsCurrent,
        setNavigationStatus,
      });
      if (!canonicalUrl) {
        return;
      }
      setApprovedUrl(canonicalUrl);
      setNavigationStatus("ready");
    } catch (error) {
      if (!routeIsCurrent()) {
        return;
      }
      void invoke("close_native_browser").catch(() => undefined);
      const reason = bridgeErrorReason(error);
      setNavigationStatus("blocked");
      setApprovedUrl(null);
      setBlockedReason(reason);
      const continuationHandler = recoverableResearchHandler(
        fallbackNotificationSentRef.current,
        onResearchRouteUnavailable,
        reason,
      );
      if (continuationHandler) {
        fallbackNotificationSentRef.current = true;
        setNavigationStatus("continuing");
        const accepted = await continuationHandler(
          route,
          reason as RecoverableBrowserFailure,
        );
        if (routeIsCurrent() && !accepted) {
          setNavigationStatus("blocked");
        }
      }
    }
  }, [navigationStatus, onResearchRouteUnavailable, route, routeGeneration]);

  const reloadRoute = useCallback(async () => {
    if (navigationStatus !== "ready") {
      return;
    }
    if (!isTauriRuntime) {
      setNavigationStatus("blocked");
      setApprovedUrl(null);
      setBlockedReason("desktop");
      return;
    }
    try {
      await invoke("reload_native_browser");
    } catch {
      void invoke("close_native_browser").catch(() => undefined);
      setNavigationStatus("blocked");
      setApprovedUrl(null);
      setBlockedReason("reload");
    }
  }, [navigationStatus]);

  return (
    <>
      <header className="shrink-0 border-b border-[var(--border-soft)] bg-[var(--background)] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase text-[var(--foreground-subtle)]">
              {t("chat.browser.eyebrow")}
            </p>
            {navigationStatus === "ready" ? (
              <h2 className="mt-1 truncate text-sm font-semibold text-[var(--foreground)]">{hostLabel}</h2>
            ) : null}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <button
              aria-label={t("chat.browser.reload")}
              className="inline-flex h-8 w-8 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
              disabled={navigationStatus !== "ready"}
              onClick={() => void reloadRoute()}
              title={t("chat.browser.reload")}
              type="button"
            >
              <RefreshIcon />
            </button>
          </div>
        </div>
        <div className="mt-3 flex items-center justify-between gap-3 text-[11px] font-medium text-[var(--foreground-subtle)]">
          <span>{isTauriRuntime && navigationStatus === "ready" ? t("chat.browser.native_webview") : t("chat.browser.preview")}</span>
          <span className="min-w-0 truncate">{t("chat.browser.suggested_destination")}</span>
        </div>
      </header>

      <div className="relative min-h-0 flex-1 bg-[var(--background)]" ref={previewRef}>
        <BrowserNavigationGate
          blockedReason={blockedReason}
          hostLabel={hostLabel}
          navigationStatus={navigationStatus}
          onConfirmNavigation={() => void confirmNavigation()}
          t={t}
        />
      </div>
    </>
  );
}

function safeBrowserDestination(value: string, fallback: string) {
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") return fallback;
    return url.hostname || fallback;
  } catch {
    return fallback;
  }
}
