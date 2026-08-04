"use client";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ExternalBrowserLink } from "@/components/ExternalBrowserLink";

export function LicenseMarkdown({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        h1: ({ children }) => (
          <h2 className="mb-3 text-lg font-semibold tracking-tight text-[var(--foreground)]">
            {children}
          </h2>
        ),
        h2: ({ children }) => (
          <h3 className="mb-2 mt-6 text-base font-semibold text-[var(--foreground)] first:mt-0">
            {children}
          </h3>
        ),
        h3: ({ children }) => (
          <h4 className="mb-2 mt-5 text-sm font-semibold text-[var(--foreground)]">
            {children}
          </h4>
        ),
        p: ({ children }) => (
          <p className="my-3 text-sm leading-6 text-[var(--foreground)]">{children}</p>
        ),
        ul: ({ children }) => (
          <ul className="my-3 list-disc space-y-1.5 pl-6 text-sm leading-6">{children}</ul>
        ),
        ol: ({ children }) => (
          <ol className="my-3 list-decimal space-y-1.5 pl-6 text-sm leading-6">{children}</ol>
        ),
        blockquote: ({ children }) => (
          <blockquote className="my-4 border-l-2 border-[var(--border-strong)] pl-4 text-[var(--foreground-muted)]">
            {children}
          </blockquote>
        ),
        a: ({ children, ...props }) => (
          <ExternalBrowserLink
            {...props}
            className="font-medium text-[var(--accent)] underline underline-offset-4"
          >
            {children}
          </ExternalBrowserLink>
        ),
        hr: () => <hr className="my-6 border-[var(--border-soft)]" />,
        table: ({ children }) => (
          <div className="my-4 overflow-x-auto rounded-[var(--radius-sm)] border border-[var(--border-soft)]">
            <table className="w-full border-collapse text-left text-xs">{children}</table>
          </div>
        ),
        th: ({ children }) => (
          <th className="border-b border-[var(--border-soft)] bg-[var(--background)] px-3 py-2 font-semibold">
            {children}
          </th>
        ),
        td: ({ children }) => (
          <td className="border-b border-[var(--border-soft)] px-3 py-2 align-top last:border-b-0">
            {children}
          </td>
        ),
        code: ({ children, className, ...props }) => (
          <code
            {...props}
            className={`${className ?? ""} rounded bg-[var(--background)] px-1 py-0.5 font-mono text-[0.9em]`}
          >
            {children}
          </code>
        ),
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
