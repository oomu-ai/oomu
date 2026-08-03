import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProjectChatScopeBanner } from "./ProjectChatScopeBanner";

const copy: Record<string, string> = {
  "chat.project_scope.label": "Chat scope",
  "chat.project_scope.active": "Project chat",
  "chat.project_scope.help": "New chats stay connected to this Project.",
  "chat.project_scope.start_global": "Start global chat",
};

describe("ProjectChatScopeBanner", () => {
  it("makes Project scope visible and provides one obvious global-chat escape", () => {
    const onStartGlobalChat = vi.fn();
    render(
      <ProjectChatScopeBanner
        onStartGlobalChat={onStartGlobalChat}
        t={(key) => copy[key] ?? key}
      />,
    );

    expect(screen.getByRole("region", { name: "Chat scope" })).toHaveTextContent(
      "New chats stay connected to this Project.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Start global chat" }));
    expect(onStartGlobalChat).toHaveBeenCalledTimes(1);
  });
});
