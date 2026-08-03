"use client";

import { useEffect, useState, type ReactNode } from "react";
import { useI18n } from "@/context/I18nContext";
import { OomuRaven } from "./OomuRaven";
import type { PrimaryAppSection, ResolvedAppSection } from "./appNavigation";

export type SidebarItem = {
  id: PrimaryAppSection;
  labelKey: string;
  icon: ReactNode;
};

type SidebarProps = {
  activeItem: ResolvedAppSection;
  items: readonly SidebarItem[];
  onItemSelect: (itemId: PrimaryAppSection) => void;
  onSettingsSelect: () => void;
  onLedgerSelect: () => void;
};

function GearIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" />
      <path d="M19.43 12.98c.04-.32.07-.65.07-.98s-.02-.66-.07-.98l2.11-1.65a.5.5 0 0 0 .12-.64l-2-3.46a.5.5 0 0 0-.61-.22l-2.49 1a7.47 7.47 0 0 0-1.69-.98L14.5 2.42A.5.5 0 0 0 14 2h-4a.5.5 0 0 0-.49.42L9.13 5.07c-.6.24-1.16.57-1.69.98l-2.49-1a.5.5 0 0 0-.61.22l-2 3.46a.5.5 0 0 0 .12.64l2.11 1.65a7.93 7.93 0 0 0 0 1.96l-2.11 1.65a.5.5 0 0 0-.12.64l2 3.46c.13.22.39.31.61.22l2.49-1c.52.4 1.09.73 1.69.98l.38 2.65c.04.24.25.42.49.42h4c.24 0 .45-.18.49-.42l.38-2.65c.6-.24 1.16-.57 1.69-.98l2.49 1c.22.09.48 0 .61-.22l2-3.46a.5.5 0 0 0-.12-.64l-2.11-1.65Z" />
    </svg>
  );
}

function LedgerIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="M4 19V5" />
      <path d="M4 19h16" />
      <path d="M8 16V9" />
      <path d="M12 16V7" />
      <path d="M16 16v-5" />
      <path d="M20 16v-3" />
    </svg>
  );
}

export function Sidebar({ activeItem, items, onItemSelect, onSettingsSelect, onLedgerSelect }: SidebarProps) {
  const { t } = useI18n();
  const [isMac, setIsMac] = useState(false);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setIsMac(typeof window !== "undefined" && /Mac|iPhone|iPod|iPad/.test(navigator.userAgent));
  }, []);

  // Main nav items read at text-base for comfortable desktop scanning.
  const itemClass = (selected: boolean, textSize = "text-base") =>
    `flex h-9 w-full items-center gap-3 rounded-[var(--radius-sm)] px-3 ${textSize} font-medium text-[var(--foreground)] transition-colors ${
      selected ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"
    }`;

  // Footer utilities are icon-only: a gear reads as Settings without a label,
  // and the Ledger sits beside it as a second quiet destination.
  const footerIconClass = (selected: boolean) =>
    `flex h-9 w-9 items-center justify-center rounded-[var(--radius-sm)] text-[var(--accent)] transition-colors ${
      selected ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"
    }`;

  return (
    <aside
      aria-label={t("sidebar.primary")}
      className={`flex h-full w-60 shrink-0 flex-col border-r border-[var(--border-soft)] ${
        isMac ? "bg-[var(--background-translucent)] backdrop-blur-[20px]" : "bg-[var(--background)]"
      }`}
    >
      {/* Traffic-light inset; the strip doubles as a window drag region. */}
      <div className="h-12 w-full shrink-0" data-tauri-drag-region />

      <div className="flex items-center gap-3 px-4 pb-4" data-tauri-drag-region>
        <OomuRaven className="h-[1.75rem] w-[1.75rem] shrink-0 text-[var(--foreground)]" />
        <span className="text-[1.294rem] font-semibold leading-none tracking-tight text-[var(--foreground)]">
          {t("common.oomu")}
          <sup className="ml-[1px] text-[0.6em] font-normal text-[var(--foreground-muted)]">®</sup>
        </span>
      </div>

      <nav aria-label={t("sidebar.menu")} className="flex flex-col gap-0.5 px-3">
        {items.map((item) => {
          const isActive = item.id === activeItem;

          return (
            <button
              aria-current={isActive ? "page" : undefined}
              className={itemClass(isActive)}
              key={item.id}
              onClick={() => onItemSelect(item.id)}
              type="button"
            >
              <span className="shrink-0 text-[var(--accent)]">{item.icon}</span>
              {t(item.labelKey)}
            </button>
          );
        })}
      </nav>

      <div className="mt-auto flex items-center gap-1 px-3 pb-4">
        <button
          aria-current={activeItem === "settings" ? "page" : undefined}
          aria-label={t("sidebar.settings")}
          className={footerIconClass(activeItem === "settings")}
          id="oomu-sidebar-settings"
          onClick={onSettingsSelect}
          title={t("sidebar.settings")}
          type="button"
        >
          <GearIcon />
        </button>
        <button
          aria-current={activeItem === "ledger" ? "page" : undefined}
          aria-label={t("sidebar.ledger")}
          className={footerIconClass(activeItem === "ledger")}
          onClick={onLedgerSelect}
          title={t("sidebar.ledger")}
          type="button"
        >
          <LedgerIcon />
        </button>
      </div>
    </aside>
  );
}
