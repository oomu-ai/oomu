import { useState } from "react";
import { projectApi, type ProjectSource } from "./projectClient";

type Translate = (key: string, values?: Record<string, string | number>) => string;

export function ProjectFolderPanel({
  busy,
  folder,
  onChanged,
  onFailed,
  projectId,
  t,
}: {
  busy: boolean;
  folder: ProjectSource | null;
  onChanged: (sources: ProjectSource[]) => Promise<void>;
  onFailed: () => void;
  projectId: string;
  t: Translate;
}) {
  const [choosing, setChoosing] = useState(false);

  async function choose() {
    setChoosing(true);
    try {
      const selected = await projectApi.chooseRoot(projectId);
      if (selected) await onChanged(await projectApi.sources(projectId));
    } catch {
      onFailed();
    } finally {
      setChoosing(false);
    }
  }

  return (
    <section className="border-t border-[var(--border-soft)] pt-6">
      <div className="flex items-start justify-between gap-5">
        <div>
          <h3 className="font-semibold">{t("projects.folder_title")}</h3>
          <p className="mt-1 max-w-2xl text-sm text-[var(--foreground-muted)]">
            {t("projects.folder_help")}
          </p>
        </div>
        <button
          className="shrink-0 rounded-[var(--radius-sm)] border px-3 py-2 text-sm font-semibold"
          disabled={busy || choosing}
          onClick={() => void choose()}
          type="button"
        >
          {t(folder ? "projects.folder_change" : "projects.folder_choose")}
        </button>
      </div>
      <p className="mt-4 break-all rounded-[var(--radius-sm)] border border-dashed p-4 text-sm text-[var(--foreground-muted)]">
        {folder?.canonicalPath ?? t("projects.folder_none")}
      </p>
    </section>
  );
}
