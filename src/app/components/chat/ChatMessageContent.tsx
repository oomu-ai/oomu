"use client";

import { memo, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useI18n } from "@/context/I18nContext";
import { stripOomuSplitViewDirectives } from "./browserRouting";
import {
  SurgicalPatchDirectiveCard,
  type SurgicalPatchDirective,
} from "./SurgicalPatchDirectiveCard";
import { GroundingProvenance } from "./GroundingProvenance";
import { ContextCondensationNotice } from "./ContextCondensationNotice";
import type { ChatMessageMetadata, PublicGroundingProvenance } from "./messageMetadata";
import { canonicalAssistantDisplayText } from "./canonicalAssistantDisplay";

type ChatMessageRole = "user" | "assistant" | "system";

export type ParsedLogicalCertificate = {
  mainContent: string;
  certificate: string | null;
};

const logicalCertificateHeadingPattern =
  /(?:^|\n)(?:#{1,6}[ \t]*)?(?:\*{0,2})[ \t]*Logical Certificate[^\n]*/i;
const logicalCertificateDividerPattern = /(?:^|\n)---[ \t]*\n[ \t]*(?:#{1,6}[ \t]*)?Prem\s*ises\s*:?/i;
const logicalCertificateDirectSectionsPattern =
  /(?:^|\n)\s*(?:#{1,6}[ \t]*)?(?:\d+\.[ \t]*)?(?:[-*\u2022][ \t]*)?(?:\*{0,3})[ \t]*Prem\s*ises[ \t]*(?:\*{0,3})[ \t]*:?/i;
const logicalCertificateSectionPattern =
  /(?:^|\n)\s*(?:#{1,6}[ \t]*)?(?:\d+\.[ \t]*)?(?:[-*\u2022][ \t]*)?(?:\*{0,3})[ \t]*(Prem\s*ises|Exec\s*ution\s+Path|Formal\s+Con\s*clusion|Con\s*clusion)[ \t]*(?:\*{0,3})[ \t]*:?[ \t]*(?:\*{0,3})?/gi;
const logicalCertificateInlineHeadingRepairPattern =
  /(Logical Certificate)\s*(?=(?:\d+\.[ \t]*)?(?:\*{0,3})[ \t]*Prem\s*ises\b)/i;
const logicalCertificateParseCache = new Map<string, ParsedLogicalCertificate>();
const LOGICAL_CERTIFICATE_PARSE_CACHE_LIMIT = 500;

export function parseLogicalCertificate(content: string): ParsedLogicalCertificate {
  const cached = logicalCertificateParseCache.get(content);
  if (cached) return cached;

  const parsed = parseLogicalCertificateUncached(content);
  logicalCertificateParseCache.set(content, parsed);
  if (logicalCertificateParseCache.size > LOGICAL_CERTIFICATE_PARSE_CACHE_LIMIT) {
    const oldestKey = logicalCertificateParseCache.keys().next().value;
    if (oldestKey !== undefined) logicalCertificateParseCache.delete(oldestKey);
  }
  return parsed;
}

function parseLogicalCertificateUncached(content: string): ParsedLogicalCertificate {
  const headingMatch = content.match(logicalCertificateHeadingPattern);
  const headingIndex = matchContentStart(content, headingMatch);
  if (headingIndex !== null && hasLogicalCertificateSections(content.slice(headingIndex))) {
    return splitLogicalCertificate(content, headingIndex);
  }

  const dividerMatch = content.match(logicalCertificateDividerPattern);
  const dividerIndex = matchContentStart(content, dividerMatch);
  if (dividerIndex !== null && hasLogicalCertificateSections(content.slice(dividerIndex))) {
    return splitLogicalCertificate(content, dividerIndex);
  }

  const directSectionsMatch = content.match(logicalCertificateDirectSectionsPattern);
  const directSectionsIndex = matchContentStart(content, directSectionsMatch);
  if (
    directSectionsIndex !== null &&
    directSectionsIndex > 0 &&
    hasLogicalCertificateSections(content.slice(directSectionsIndex))
  ) {
    return splitLogicalCertificate(content, directSectionsIndex);
  }

  if (headingIndex !== null) return splitLogicalCertificate(content, headingIndex);
  return { mainContent: content, certificate: null };
}

function matchContentStart(content: string, match: RegExpMatchArray | null): number | null {
  if (!match || match.index === undefined) return null;
  return content[match.index] === "\n" ? match.index + 1 : match.index;
}

function hasLogicalCertificateSections(content: string) {
  return extractLogicalCertificateSections(content) !== null;
}

function splitLogicalCertificate(content: string, index: number): ParsedLogicalCertificate {
  return {
    mainContent: content.slice(0, index).replace(/\n\s*(?:---|\*\*\*)\s*$/, "").trim(),
    certificate: normalizeLogicalCertificate(content.slice(index).trim()),
  };
}

type LogicalCertificateSections = {
  premises: string;
  executionPath: string;
  formalConclusion: string;
};

export function normalizeLogicalCertificate(content: string) {
  const sections = extractLogicalCertificateSections(content);
  if (!sections) return content.trim();
  const parts: string[] = ["---"];
  if (sections.premises) parts.push("Premises:", sections.premises, "");
  if (sections.executionPath) parts.push("Execution Path:", sections.executionPath, "");
  if (sections.formalConclusion) {
    parts.push("Formal Conclusion:", sections.formalConclusion, "");
  }
  return parts.join("\n").trim();
}

function extractLogicalCertificateSections(content: string): LogicalCertificateSections | null {
  const prepared = content.replace(logicalCertificateInlineHeadingRepairPattern, "$1\n");
  const matches: Array<{
    key: "premises" | "executionPath" | "formalConclusion" | "conclusion";
    headerStart: number;
    contentStart: number;
  }> = [];
  logicalCertificateSectionPattern.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = logicalCertificateSectionPattern.exec(prepared)) !== null) {
    const rawHeader = (match[1] ?? "").replace(/\s+/g, "").toLowerCase();
    const key = rawHeader === "premises"
      ? "premises"
      : rawHeader === "executionpath"
        ? "executionPath"
        : rawHeader === "formalconclusion"
          ? "formalConclusion"
          : "conclusion";
    matches.push({
      key,
      headerStart: match.index + (match[0].startsWith("\n") ? 1 : 0),
      contentStart: match.index + match[0].length,
    });
  }
  if (matches.length < 2) return null;

  const sectionText = new Map<string, string>();
  matches.forEach((section, index) => {
    if (sectionText.has(section.key)) return;
    const nextStart = matches[index + 1]?.headerStart ?? prepared.length;
    sectionText.set(
      section.key,
      cleanLogicalCertificateSection(prepared.slice(section.contentStart, nextStart)),
    );
  });

  const premises = sectionText.get("premises") ?? "";
  const executionPath = sectionText.get("executionPath") ?? "";
  const formalConclusion =
    sectionText.get("formalConclusion") ?? sectionText.get("conclusion") ?? "";
  const populatedSections = [premises, executionPath, formalConclusion].filter(Boolean).length;
  if (!premises || populatedSections < 2) return null;
  return { premises, executionPath, formalConclusion };
}

function cleanLogicalCertificateSection(content: string) {
  return content
    .replace(
      /(?:^|\n)\s*(?:\d+\.[ \t]*)?(?:[-*][ \t]*)?(?:\*{0,3})[ \t]*(?:State|RAG Decision)[ \t]*(?:\*{0,3})[ \t]*:([\s\S]*)$/i,
      "",
    )
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

type SurgicalPatchParseResult = {
  displayContent: string;
  patches: SurgicalPatchDirective[];
};

function parseSurgicalPatchDirectives(content: string): SurgicalPatchParseResult {
  const patches: SurgicalPatchDirective[] = [];
  const displayContent = content
    .replace(/```diff[^\n]*\n([\s\S]*?)```/gi, (_match, rawDiff: string) => {
      const diff = rawDiff.trim();
      if (diff) patches.push({ rawDiff: diff, files: summarizeUnifiedDiff(diff) });
      return "";
    })
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return { displayContent, patches };
}

function summarizeUnifiedDiff(diff: string): SurgicalPatchDirective["files"] {
  const summaries = new Map<string, { path: string; additions: number; deletions: number }>();
  let activePath = "patch";
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++ ")) {
      activePath = cleanUnifiedDiffPath(line.slice(4)) ?? activePath;
      if (!summaries.has(activePath)) {
        summaries.set(activePath, { path: activePath, additions: 0, deletions: 0 });
      }
      continue;
    }
    if (!summaries.has(activePath)) {
      summaries.set(activePath, { path: activePath, additions: 0, deletions: 0 });
    }
    const summary = summaries.get(activePath);
    if (!summary) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) summary.additions += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) summary.deletions += 1;
  }
  return [...summaries.values()];
}

function cleanUnifiedDiffPath(rawPath: string) {
  const path = rawPath.trim().split("\t")[0]?.replace(/^"|"$/g, "");
  if (!path || path === "/dev/null") return null;
  return path.replace(/^[ab]\//, "");
}

export const ChatMessageContent = memo(function ChatMessageContent({
  accessibilityId,
  content,
  role,
  sources,
  metadata,
}: {
  accessibilityId?: string;
  content: string;
  role: ChatMessageRole;
  sources?: PublicGroundingProvenance[];
  metadata?: ChatMessageMetadata | null;
}) {
  const canonicalContent = useMemo(
    () => role === "assistant" ? canonicalAssistantDisplayText(content) : content,
    [content, role],
  );
  const { mainContent, certificate } = useMemo(
    () => role === "assistant"
      ? parseLogicalCertificate(canonicalContent)
      : { mainContent: content, certificate: null },
    [canonicalContent, content, role],
  );
  const visibleMainContent = useMemo(
    () => role === "assistant" ? stripOomuSplitViewDirectives(mainContent) : mainContent,
    [mainContent, role],
  );
  const { displayContent, patches } = useMemo(
    () => role === "assistant"
      ? parseSurgicalPatchDirectives(visibleMainContent)
      : { displayContent: visibleMainContent, patches: [] },
    [role, visibleMainContent],
  );

  return (
    <div className="chat-message-content mt-2 text-sm leading-6">
      <div id={accessibilityId} role={accessibilityId ? "document" : undefined}>
        {role === "user" ? (
          <div className="whitespace-pre-wrap break-words">{displayContent}</div>
        ) : (
          displayContent && (
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                a: ({ children, ...props }) => (
                  <a {...props} className="underline underline-offset-4" rel="noreferrer" target="_blank">
                    {children}
                  </a>
                ),
                code: ({ children, className, ...props }) => (
                  <code {...props} className={className}>{children}</code>
                ),
              }}
            >
              {displayContent}
            </ReactMarkdown>
          )
        )}
        {role === "assistant" && <GroundingProvenance sources={sources} />}
      </div>
      {patches.map((patch, index) => (
        <SurgicalPatchDirectiveCard
          index={index}
          key={`${patch.files.map((file) => file.path).join(":")}-${index}`}
          patch={patch}
        />
      ))}
      {certificate && <LogicalCertificateDisclosure certificate={certificate} />}
      {role === "assistant" && metadata && <ContextCondensationNotice metadata={metadata} />}
    </div>
  );
});

const LogicalCertificateDisclosure = memo(function LogicalCertificateDisclosure({
  certificate,
}: {
  certificate: string;
}) {
  const { t } = useI18n();
  const sections = useMemo(() => extractLogicalCertificateSections(certificate), [certificate]);
  return (
    <details className="group mt-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-sm transition-all duration-150 hover:border-[var(--border-strong)]">
      <summary className="flex cursor-pointer select-none items-center justify-between gap-2 font-medium text-[var(--foreground-muted)] outline-none transition-colors hover:text-[var(--foreground)]">
        <span className="inline-flex items-center gap-2">
          <svg aria-hidden="true" className="h-3.5 w-3.5 shrink-0" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
            <path d="M12 3 4 6v5c0 4.5 3.2 7.9 8 9 4.8-1.1 8-4.5 8-9V6l-8-3Z" />
            <path d="m9 12 2 2 4-4" />
          </svg>
          {t("chat.certificate.title")}
        </span>
        <svg aria-hidden="true" className="h-4 w-4 shrink-0 text-[var(--foreground-subtle)] transition-transform duration-150 group-open:rotate-90" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
          <path d="m9 6 6 6-6 6" />
        </svg>
      </summary>
      <div className="mt-3 space-y-3 border-t border-[var(--border-soft)] pt-3">
        {sections ? (
          <>
            <LogicalCertificateSection body={sections.premises} label={t("chat.certificate.premises")} />
            <LogicalCertificateSection body={sections.executionPath} label={t("chat.certificate.execution_path")} />
            <LogicalCertificateSection body={sections.formalConclusion} label={t("chat.certificate.formal_conclusion")} />
          </>
        ) : (
          <div className="leading-relaxed text-[var(--foreground-muted)]">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{certificate}</ReactMarkdown>
          </div>
        )}
      </div>
    </details>
  );
});

export const CompactionSummaryDisclosure = memo(function CompactionSummaryDisclosure({
  content,
}: {
  content: string;
}) {
  const { t } = useI18n();
  return (
    <details className="group mt-2 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-3 text-sm transition-all duration-150 hover:border-[var(--border-strong)]">
      <summary className="flex cursor-pointer select-none items-center justify-between gap-2 font-medium text-[var(--foreground-muted)] outline-none transition-colors hover:text-[var(--foreground)]">
        <span className="inline-flex items-center gap-2">
          <svg aria-hidden="true" className="h-3.5 w-3.5 shrink-0" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
            <path d="M5 4h14v16H5z" />
            <path d="M8 8h8M8 12h8M8 16h5" />
          </svg>
          {t("chat.compaction.title")}
        </span>
        <svg aria-hidden="true" className="h-4 w-4 shrink-0 text-[var(--foreground-subtle)] transition-transform duration-150 group-open:rotate-90" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
          <path d="m9 6 6 6-6 6" />
        </svg>
      </summary>
      <div className="mt-3 border-t border-[var(--border-soft)] pt-1 text-[var(--foreground-muted)]">
        <ChatMessageContent content={content} role="system" />
      </div>
    </details>
  );
});

function LogicalCertificateSection({ body, label }: { body: string; label: string }) {
  if (!body.trim()) return null;
  return (
    <div>
      <p className="text-[11px] font-semibold uppercase tracking-wide text-[var(--foreground-subtle)]">
        {label}
      </p>
      <div className="mt-1 leading-relaxed text-[var(--foreground-muted)]">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{body}</ReactMarkdown>
      </div>
    </div>
  );
}
