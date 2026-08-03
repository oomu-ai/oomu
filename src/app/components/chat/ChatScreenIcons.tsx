export function NewChatIcon() {
  return <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></svg>;
}

export function PencilIcon() {
  return <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></svg>;
}

export function StatusIndicator({ available, className, label }: { available: boolean; className: string; label: string }) {
  return <span className={className}><span className={`h-2 w-2 rounded-full ${available ? "bg-[var(--success)]" : "bg-[var(--warning)]"}`} aria-hidden="true" />{label}</span>;
}

export function SlidersIcon() {
  return <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><line x1="21" x2="14" y1="6" y2="6" /><line x1="10" x2="3" y1="6" y2="6" /><line x1="21" x2="12" y1="12" y2="12" /><line x1="8" x2="3" y1="12" y2="12" /><line x1="21" x2="16" y1="18" y2="18" /><line x1="12" x2="3" y1="18" y2="18" /><line x1="14" x2="14" y1="4" y2="8" /><line x1="8" x2="8" y1="10" y2="14" /><line x1="16" x2="16" y1="16" y2="20" /></svg>;
}

export function SplitPaneIcon() {
  return <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="18" rx="2" width="18" x="3" y="3" /><path d="M12 3v18" /></svg>;
}

export function CopyIcon() {
  return <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="14" rx="2" ry="2" width="14" x="8" y="8" /><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" /></svg>;
}

export function CheckIcon() {
  return <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24"><path d="m20 6-11 11-5-5" /></svg>;
}
