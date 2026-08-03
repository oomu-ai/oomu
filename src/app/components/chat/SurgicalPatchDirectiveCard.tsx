"use client";

import { useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";

export type SurgicalPatchDirective = {
  rawDiff: string;
  files: Array<{ path: string; additions: number; deletions: number }>;
};

type SurgicalPatchApplyResponse = {
  status?: string;
  message?: string;
  verified?: boolean;
};

export function SurgicalPatchDirectiveCard({
  patch,
  index,
}: {
  patch: SurgicalPatchDirective;
  index: number;
}) {
  const { t } = useI18n();
  const [status, setStatus] = useState<"idle" | "applying" | "applied" | "failed">("idle");
  const [message, setMessage] = useState("");
  async function handleApplyPatch() {
    setStatus("applying");
    setMessage("");
    try {
      await invoke<SurgicalPatchApplyResponse>("apply_surgical_patch_directive", {
        request: { diff: patch.rawDiff },
      });
      setStatus("applied");
      setMessage(t("chat.patch.applied"));
    } catch {
      setStatus("failed");
      setMessage(t("chat.patch.failed"));
    }
  }

  function lineClassName(line: string) {
    if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("@@")) {
      return "bg-[var(--accent-background)] text-[var(--accent)]";
    }
    if (line.startsWith("+")) return "bg-[var(--success-background)] text-[var(--success)]";
    if (line.startsWith("-")) return "bg-[var(--destructive-background)] text-[var(--destructive)]";
    return "text-[var(--foreground-muted)]";
  }

  return (
    <div className="mt-4 overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)]">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2">
        <div>
          <p className="text-xs font-semibold text-[var(--foreground)]">{t("chat.patch.title", { count: index + 1 })}</p>
          <p className="mt-0.5 text-[11px] text-[var(--foreground-muted)]">
            {t(patch.files.length === 1 ? "chat.patch.file_one" : "chat.patch.file_many", { count: patch.files.length })}
          </p>
        </div>
        <button className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-1.5 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-60" disabled={status === "applying"} onClick={handleApplyPatch} type="button">
          {status === "applying" ? t("chat.patch.applying") : status === "applied" ? t("chat.patch.applied_button") : t("chat.patch.apply")}
        </button>
      </div>
      <div className="grid gap-2 px-3 py-3">
        {patch.files.map((file) => (
          <div className="grid gap-2 border-l-2 border-[var(--border-strong)] pl-3 font-mono text-xs leading-5 text-[var(--foreground-muted)]" key={file.path}>
            <span className="break-all text-[var(--foreground)]">{file.path}</span>
            <span><span className="text-[var(--success)]">+{file.additions}</span><span className="mx-2 text-[var(--foreground-subtle)]">/</span><span className="text-[var(--destructive)]">-{file.deletions}</span></span>
          </div>
        ))}
      </div>
      <pre className="max-h-80 overflow-auto border-t border-[var(--border-soft)] bg-[var(--background)] p-0 font-mono text-xs leading-5">
        {patch.rawDiff.split("\n").map((line, lineIndex) => (
          <code className={`block min-w-max whitespace-pre px-3 ${lineClassName(line)}`} key={`${lineIndex}-${line}`}>{line || " "}</code>
        ))}
      </pre>
      {message ? <p className={`border-t border-[var(--border-soft)] px-3 py-2 text-xs ${status === "failed" ? "text-[var(--destructive)]" : "text-[var(--foreground-muted)]"}`}>{message}</p> : null}
    </div>
  );
}
