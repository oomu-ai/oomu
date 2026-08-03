"use client";

import type { ReactNode } from "react";

interface SettingsHeaderProps {
  title: string;
  showBorder?: boolean;
  children?: ReactNode;
  className?: string;
}

export function SettingsHeader({
  title,
  showBorder = false,
  children,
  className = "",
}: SettingsHeaderProps) {
  return (
    <div
      className={`flex flex-col gap-3 pb-4 lg:flex-row lg:items-end lg:justify-between ${
        showBorder ? "border-b border-[var(--border-strong)]" : ""
      } ${className}`}
    >
      <div>
        <h2 className="text-base font-bold text-[var(--foreground)]">
          {title}
        </h2>
      </div>
      {children && (
        <div className="flex w-full shrink-0 flex-col gap-2 sm:flex-row sm:justify-end lg:w-auto">
          {children}
        </div>
      )}
    </div>
  );
}
