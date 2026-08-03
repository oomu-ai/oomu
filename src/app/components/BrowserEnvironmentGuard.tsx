"use client";

import { useSyncExternalStore } from "react";
import { useI18n } from "@/context/I18nContext";
import { isTauriRuntime } from "@/lib/invoke";

type BrowserEnvironmentState = {
  isRuntimeChecked: boolean;
  isUncontainedBrowser: boolean;
};

const serverState: BrowserEnvironmentState = {
  isRuntimeChecked: false,
  isUncontainedBrowser: false,
};
const browserState: BrowserEnvironmentState = {
  isRuntimeChecked: true,
  isUncontainedBrowser: !isTauriRuntime,
};

function subscribeToBrowserEnvironment() {
  return () => undefined;
}

function getBrowserEnvironmentSnapshot() {
  return browserState;
}

function getServerBrowserEnvironmentSnapshot() {
  return serverState;
}

export function useBrowserEnvironment() {
  return useSyncExternalStore(
    subscribeToBrowserEnvironment,
    getBrowserEnvironmentSnapshot,
    getServerBrowserEnvironmentSnapshot,
  );
}

export function BrowserEnvironmentGuard() {
  const { t } = useI18n();
  const environment = useBrowserEnvironment();

  if (!environment.isRuntimeChecked || !environment.isUncontainedBrowser) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-end bg-black/20 p-4 sm:p-5">
      <section
        aria-describedby="browser-environment-guard-description"
        aria-labelledby="browser-environment-guard-title"
        aria-modal="true"
        className="w-full max-w-[30rem] rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5 text-[var(--foreground)] shadow-[var(--shadow-raised)]"
        role="dialog"
      >
        <div className="flex items-start gap-3">
          <div
            aria-hidden="true"
            className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[var(--warning-background)] text-[var(--warning)]"
          >
            <svg
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth="2"
              viewBox="0 0 24 24"
            >
              <path d="M12 9v4" />
              <path d="M12 17h.01" />
              <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71-3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
            </svg>
          </div>

          <div className="min-w-0 flex-1">
            <h2
              className="text-sm font-semibold text-[var(--foreground)]"
              id="browser-environment-guard-title"
            >
              {t("browser_environment.title")}
            </h2>
            <div
              className="mt-2 space-y-2 text-sm leading-6 text-[var(--foreground-muted)]"
              id="browser-environment-guard-description"
            >
              <p>
                {t("browser_environment.description")}
              </p>
              <p>
                {t("browser_environment.open_desktop")}
              </p>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
