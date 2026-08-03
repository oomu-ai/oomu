import type { Metadata } from "next";
import { AppShell } from "@/components/AppShell";
import { ThemeProvider } from "@/components/ThemeProvider";
import { AppContextProvider } from "@/context/AppContext";
import { I18nProvider } from "@/context/I18nContext";
import { ApprovalProvider } from "@/context/ApprovalContext";
import { McpProvider } from "@/hooks/useMcp";
import "./globals.css";

export const metadata: Metadata = {
  title: "OOMU",
  description: "OOMU native workflow workstation",
  icons: {
    icon: [{ url: "/icon.png", type: "image/png" }],
    apple: [{ url: "/apple-icon.png", type: "image/png" }],
    shortcut: ["/icon.png"],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="h-full antialiased" suppressHydrationWarning style={{ backgroundColor: 'var(--background)' }}>
      <body className="min-h-full bg-[var(--background)] text-[var(--foreground)]" style={{ backgroundColor: 'var(--background)' }}>
        <ThemeProvider>
          <I18nProvider>
            <ApprovalProvider>
              <AppContextProvider>
                <McpProvider>
                  <AppShell>{children}</AppShell>
                </McpProvider>
              </AppContextProvider>
            </ApprovalProvider>
          </I18nProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
