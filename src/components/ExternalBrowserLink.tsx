"use client";

import type { AnchorHTMLAttributes, MouseEvent } from "react";
import { invoke } from "@/lib/invoke";

const MAX_EXTERNAL_HTTP_URL_BYTES = 8 * 1024;

export function safeExternalHttpUrl(href: string | undefined): string | null {
  if (!href || href.length > MAX_EXTERNAL_HTTP_URL_BYTES || href.trim() !== href) return null;
  try {
    const url = new URL(href);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (!url.hostname || url.username || url.password) return null;
    return href;
  } catch {
    return null;
  }
}

type ExternalBrowserLinkProps = Omit<
  AnchorHTMLAttributes<HTMLAnchorElement>,
  "href" | "onClick" | "rel" | "target"
> & {
  href?: string;
};

export function ExternalBrowserLink({ href, children, ...props }: ExternalBrowserLinkProps) {
  const safeHref = safeExternalHttpUrl(href);
  if (!safeHref) {
    return <span className={props.className}>{children}</span>;
  }

  const openInDefaultBrowser = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    event.stopPropagation();
    void invoke("open_external_http_url", { url: safeHref }).catch(() => undefined);
  };

  return (
    <a
      {...props}
      href={safeHref}
      onClick={openInDefaultBrowser}
      rel="noreferrer noopener"
    >
      {children}
    </a>
  );
}
