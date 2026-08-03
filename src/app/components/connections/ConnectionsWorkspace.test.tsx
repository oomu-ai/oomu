import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConnectionsWorkspace } from "./ConnectionsWorkspace";

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) =>
      ({
        "sidebar.connections": "Connections",
        "connections.sections": "Connection views",
        "connections.section_work_apps": "Apps OOMU can use",
        "connections.section_messaging": "Message OOMU",
      })[key] ?? key,
  }),
}));

vi.mock("../integrations/IntegrationsScreen", () => ({
  IntegrationsScreen: ({ showIntroduction }: { showIntroduction?: boolean }) => (
    <div>work-apps:{String(showIntroduction)}</div>
  ),
}));

vi.mock("../ChannelsDashboard", () => ({
  ChannelsDashboard: () => <div>messaging-dashboard</div>,
}));

afterEach(cleanup);

describe("ConnectionsWorkspace", () => {
  it("mounts work apps without a repeated screen introduction", () => {
    render(
      <ConnectionsWorkspace activeSection="work_apps" onSectionChange={vi.fn()} />,
    );

    expect(screen.getByText("work-apps:false")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Connections" })).toBeNull();
  });

  it("names both sections by what OOMU does", () => {
    const onSectionChange = vi.fn();
    const { rerender } = render(
      <ConnectionsWorkspace
        activeSection="work_apps"
        onSectionChange={onSectionChange}
      />,
    );

    expect(screen.getByRole("tab", { name: "Apps OOMU can use" })).toBeVisible();
    fireEvent.click(screen.getByRole("tab", { name: "Message OOMU" }));
    expect(onSectionChange).toHaveBeenCalledWith("messaging");

    rerender(
      <ConnectionsWorkspace
        activeSection="messaging"
        onSectionChange={onSectionChange}
      />,
    );
    expect(screen.getByText("messaging-dashboard")).toBeVisible();
  });
});
