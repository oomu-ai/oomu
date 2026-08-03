import type { HeadlessSearchDebug } from "./localSearchContext";

type WozniakSearchDebugProps = {
  debug: HeadlessSearchDebug;
  translate: (key: string) => string;
};

export function WozniakSearchDebug({
  debug,
  translate,
}: WozniakSearchDebugProps) {
  const rows = [
    ["chat.drawer.wozniak_query", debug.query],
    ["chat.drawer.wozniak_sources", debug.resultCount],
    ["chat.drawer.wozniak_pages", debug.domPageCount],
    ["chat.drawer.wozniak_browser_fallbacks", debug.headlessFallbackCount],
    ["chat.drawer.wozniak_elapsed", `${debug.retrievalElapsedMs} ms`],
  ] as const;

  return (
    <details className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
      <summary className="cursor-pointer text-xs font-semibold text-[var(--foreground)]">
        {translate("chat.drawer.wozniak_title")}
      </summary>
      <p className="mt-2 text-[11px] leading-4 text-[var(--foreground-subtle)]">
        {translate("chat.drawer.wozniak_help")}
      </p>
      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-[11px]">
        {rows.map(([label, value], index) => (
          <div className="contents" key={label}>
            <dt className="font-medium text-[var(--foreground-subtle)]">
              {translate(label)}
            </dt>
            <dd
              className={
                index === 0
                  ? "min-w-0 truncate text-right text-[var(--foreground)]"
                  : "text-right text-[var(--foreground)]"
              }
            >
              {value}
            </dd>
          </div>
        ))}
      </dl>
    </details>
  );
}
