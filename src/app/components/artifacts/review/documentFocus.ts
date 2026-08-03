const DOCUMENT_FOCUS_KEY = "oomu.documents.focus";

type DocumentFocusKind = "word" | "spreadsheet" | "presentation";

export function requestDocumentFocus(kind: DocumentFocusKind, artifactId: string) {
  if (typeof window === "undefined" || !/^[A-Za-z0-9_-]{1,200}$/.test(artifactId)) return;
  window.sessionStorage.setItem(DOCUMENT_FOCUS_KEY, `${kind}:${artifactId}`);
}

export function consumeDocumentFocus() {
  if (typeof window === "undefined") return "";
  const value = window.sessionStorage.getItem(DOCUMENT_FOCUS_KEY) ?? "";
  window.sessionStorage.removeItem(DOCUMENT_FOCUS_KEY);
  return /^(?:word|spreadsheet|presentation):[A-Za-z0-9_-]{1,200}$/.test(value) ? value : "";
}
