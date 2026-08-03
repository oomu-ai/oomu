import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatAgentSelector } from "./ChatAgentSelector";
import {
  ChatEmptyState,
  type ChatStarterAction,
} from "./ChatEmptyState";
import { FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY } from "./firstRunWelcomeState";

const copy = vi.hoisted(() => ({
  "chat.agent": "Agent",
  "chat.manage_agents": "Manage agents…",
  "chat.agent_ready": "{name} is ready.",
  "chat.pick_agent": "Choose an agent to begin.",
  "chat.welcome.title": "What should OOMU do first?",
  "chat.welcome.subtitle": "Pick one to see OOMU work, or just ask below.",
  "chat.welcome.actions.weekly_brief.title": "Get a weekly brief",
  "chat.welcome.actions.weekly_brief.description": "OOMU reads your project and the latest sources, then writes you a Word and PDF brief every week.",
  "chat.welcome.actions.summarize_folder.title": "Summarize a folder",
  "chat.welcome.actions.summarize_folder.description": "Point OOMU at a folder of notes and text files and get a readable summary.",
  "chat.welcome.actions.help_with_email.title": "Draft replies to my email",
  "chat.welcome.actions.help_with_email.description": "OOMU reads unread Mail and prepares replies — you approve before anything is sent.",
  "chat.welcome.dismiss": "Not now",
  "chat.welcome.action_error": "OOMU couldn't start that yet. Nothing changed. Try again.",
  "chat.welcome.folder_error": "OOMU couldn't prepare that folder. Choose one with readable notes or text files, then try again.",
  "common.oomu": "OOMU",
  "common.results_location": "Anything OOMU makes or runs shows up under Documents and Tasks.",
}));

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: keyof typeof copy, variables?: Record<string, string | number>) => {
      let value = copy[key] ?? key;
      for (const [name, replacement] of Object.entries(variables ?? {})) {
        value = value.replaceAll(`{${name}}`, String(replacement));
      }
      return value;
    },
  }),
}));

const emptyStateProps = {
  agentDescription: "Thoughtful and practical.",
  agentName: "OOMU",
  sessionCount: 0,
  sessionsLoaded: true,
  transcriptEmpty: true,
};

let storage: Map<string, string>;

describe("first-run chat presentation", () => {
  beforeEach(() => {
    storage = new Map();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: vi.fn((key: string) => storage.get(key) ?? null),
        removeItem: vi.fn((key: string) => storage.delete(key)),
        setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
      },
    });
  });
  afterEach(cleanup);

  it("does not flash the starter welcome before sessions are loaded", async () => {
    const view = render(
      <ChatEmptyState {...emptyStateProps} sessionsLoaded={false} />,
    );

    expect(screen.queryByRole("heading", { name: "What should OOMU do first?" })).toBeNull();
    expect(screen.getByText(/OOMU is ready\./)).toBeVisible();

    view.rerender(
      <ChatEmptyState {...emptyStateProps} onStarterAction={vi.fn()} sessionsLoaded />,
    );

    expect(
      await screen.findByRole("heading", { name: "What should OOMU do first?" }),
    ).toBeVisible();
    expect(screen.getByText("Anything OOMU makes or runs shows up under Documents and Tasks.")).toBeVisible();
  });

  it("does not flash for a returning session while its transcript hydrates", () => {
    const view = render(
      <ChatEmptyState
        {...emptyStateProps}
        onStarterAction={vi.fn()}
        transcriptLoaded={false}
      />,
    );

    expect(screen.queryByText("What should OOMU do first?")).toBeNull();
    expect(screen.queryByText(/OOMU is ready\./)).toBeNull();

    view.rerender(
      <ChatEmptyState
        {...emptyStateProps}
        onStarterAction={vi.fn()}
        transcriptEmpty={false}
        transcriptLoaded
      />,
    );
    expect(screen.queryByText("What should OOMU do first?")).toBeNull();
  });

  it("never nags when the weekly brief is already complete", async () => {
    render(
      <ChatEmptyState
        {...emptyStateProps}
        decisionBriefCompletion="complete"
        onStarterAction={vi.fn()}
      />,
    );

    await waitFor(() => expect(screen.getByText(/OOMU is ready\./)).toBeVisible());
    expect(screen.queryByText("What should OOMU do first?")).toBeNull();
  });

  it.each<[string, ChatStarterAction]>([
    ["Get a weekly brief", "weekly_brief"],
    ["Summarize a folder", "summarize_folder"],
    ["Draft replies to my email", "help_with_email"],
  ])("routes %s through the typed starter callback and remembers the choice", async (label, action) => {
    const onStarterAction = vi.fn();
    render(
      <ChatEmptyState {...emptyStateProps} onStarterAction={onStarterAction} />,
    );

    fireEvent.click(await screen.findByRole("button", { name: new RegExp(`^${label}`) }));

    await waitFor(() => {
      expect(onStarterAction).toHaveBeenCalledWith(action);
      expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBe("1");
      expect(screen.queryByRole("heading", { name: "What should OOMU do first?" })).toBeNull();
      expect(screen.getByText(/OOMU is ready\./)).toBeVisible();
    });
  });

  it("keeps the welcome open when the folder picker is cancelled", async () => {
    const onStarterAction = vi.fn().mockResolvedValue(false);
    render(<ChatEmptyState {...emptyStateProps} onStarterAction={onStarterAction} />);

    fireEvent.click(await screen.findByRole("button", { name: /^Summarize a folder/ }));

    await waitFor(() => expect(onStarterAction).toHaveBeenCalledWith("summarize_folder"));
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBeNull();
    expect(screen.getByRole("heading", { name: "What should OOMU do first?" })).toBeVisible();
  });

  it("shows a calm retry message when a starter cannot launch", async () => {
    render(
      <ChatEmptyState
        {...emptyStateProps}
        onStarterAction={vi.fn().mockRejectedValue(new Error("BACKEND CANARY"))}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: /^Summarize a folder/ }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU couldn't prepare that folder. Choose one with readable notes or text files, then try again.",
    );
    expect(screen.queryByText(/BACKEND CANARY/)).toBeNull();
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBeNull();
  });

  it("persists an explicit dismissal across mounts", async () => {
    const first = render(<ChatEmptyState {...emptyStateProps} onStarterAction={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Not now" }));
    first.unmount();

    render(<ChatEmptyState {...emptyStateProps} onStarterAction={vi.fn()} />);
    await waitFor(() => {
      expect(screen.queryByRole("heading", { name: "What should OOMU do first?" })).toBeNull();
      expect(screen.getByText(/OOMU is ready\./)).toBeVisible();
      expect(screen.getByRole("button", { name: "Get a weekly brief" })).toBeVisible();
    });
  });

  it("keeps the weekly brief reachable after dismissal in a later session", async () => {
    window.localStorage.setItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY, "1");
    const onStarterAction = vi.fn();
    render(
      <ChatEmptyState
        {...emptyStateProps}
        onStarterAction={onStarterAction}
        sessionCount={2}
      />,
    );

    const doorway = await screen.findByRole("button", { name: "Get a weekly brief" });
    expect(screen.queryByRole("heading", { name: "What should OOMU do first?" })).toBeNull();
    fireEvent.click(doorway);

    await waitFor(() => expect(onStarterAction).toHaveBeenCalledWith("weekly_brief"));
  });

  it("uses the quiet fallback outside the first-run window and renders nothing over a transcript", async () => {
    const view = render(<ChatEmptyState {...emptyStateProps} />);
    await waitFor(() => expect(screen.getByText(/OOMU is ready\./)).toBeVisible());
    expect(screen.queryByText("What should OOMU do first?")).toBeNull();

    view.rerender(
      <ChatEmptyState {...emptyStateProps} onStarterAction={vi.fn()} sessionCount={2} />,
    );
    expect(screen.queryByText("What should OOMU do first?")).toBeNull();

    view.rerender(
      <ChatEmptyState {...emptyStateProps} onStarterAction={vi.fn()} transcriptEmpty={false} />,
    );
    expect(screen.queryByText(/OOMU is ready\./)).toBeNull();

    view.rerender(
      <ChatEmptyState
        {...emptyStateProps}
        agentDescription={null}
        agentName={null}
        sessionCount={2}
      />,
    );
    expect(screen.getByText("Choose an agent to begin.")).toBeVisible();
  });
});

describe("chat agent doorway", () => {
  afterEach(cleanup);

  it("keeps agent selection and management as adjacent typed actions", () => {
    const onAgentChange = vi.fn();
    const onManageAgents = vi.fn();
    render(
      <ChatAgentSelector
        activeAgentId="agent-1"
        agents={[
          { id: "agent-1", name: "OOMU" },
          { id: "agent-2", name: "Scout" },
        ]}
        controlClassName="control"
        onAgentChange={onAgentChange}
        onManageAgents={onManageAgents}
      />,
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Agent" }), {
      target: { value: "agent-2" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Manage agents…" }));

    expect(onAgentChange).toHaveBeenCalledWith("agent-2");
    expect(onManageAgents).toHaveBeenCalledTimes(1);
  });

  it("does not offer a dead management doorway without its optional callback", () => {
    render(
      <ChatAgentSelector
        activeAgentId="agent-1"
        agents={[{ id: "agent-1", name: "OOMU" }]}
        controlClassName="control"
        onAgentChange={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "Manage agents…" })).toBeNull();
  });
});
