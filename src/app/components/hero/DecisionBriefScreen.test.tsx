import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY } from "../chat/firstRunWelcomeState";
import { DecisionBriefScreen } from "./DecisionBriefScreen";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const project = {
  projectId: "project-1",
  name: "Launch plan",
  description: "",
  dataPolicy: "local_only",
  instructions: "",
  archivedAtMs: null,
  createdAtMs: 1,
  updatedAtMs: 1,
  sourceCount: 0,
  conversationCount: 0,
  workflowCount: 0,
  taskCount: 0,
};

const requirements = [
  ["project_knowledge", "ready", "projects"],
  ["instructions", "needs_setup", "projects"],
  ["mail_calendar", "ready", "integrations"],
  ["current_web", "needs_setup", "settings"],
  ["parallel_research", "ready", "tasks"],
  ["verified_artifact", "needs_setup", "artifacts"],
  ["weekly_routine", "ready", "routines"],
  ["delivery", "needs_setup", "routines"],
].map(([id, state, destination]) => ({
  id,
  state,
  destination,
  label: `BACKEND LABEL CANARY ${id}`,
  detail: `BACKEND mutation postcondition CANARY ${id}`,
}));

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [
      {
        id: "en-US",
        label: "English (US)",
        fileName: "en-US.json",
        isDefault: true,
        verified: true,
      },
    ],
    translations: {},
  };
}

let heroStatus: {
  readyOnDemand: boolean;
  readyWeekly: boolean;
  requirements: Array<{
    id: string;
    state: string;
    destination: string;
    label: string;
    detail: string;
  }>;
};
let storage: Map<string, string>;

describe("DecisionBriefScreen", () => {
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
    heroStatus = {
      readyOnDemand: true,
      readyWeekly: false,
      requirements,
    };
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return [project];
      if (command === "get_weekly_decision_brief_status") return heroStatus;
      return null;
    });
  });

  afterEach(cleanup);

  it("maps every shipped requirement and destination without rendering backend prose", async () => {
    const onNavigate = vi.fn();
    render(<DecisionBriefScreen onNavigate={onNavigate} />, { wrapper: I18nProvider });

    expect(
      await screen.findByRole("heading", { name: "Weekly Decision Brief" }),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Every week, OOMU reads your project and the latest sources, writes a Word and PDF brief, and shows you what it used.",
      ),
    ).toBeVisible();
    expect(await screen.findByText("Project sources")).toBeVisible();

    for (const label of [
      "Brief instructions",
      "Mail and Calendar",
      "Latest sources",
      "Research",
      "Word and PDF brief",
      "Weekly schedule",
      "Delivery",
    ]) {
      expect(screen.getByText(label)).toBeVisible();
    }

    for (const detail of [
      "OOMU has Project sources ready to read.",
      "Tell OOMU which decisions, risks, and format matter.",
      "Mail and Calendar are connected to this Project.",
      "Allow web research in Settings so the brief can include the latest information.",
      "OOMU has completed research it can use in the brief.",
      "Create the Word and PDF brief after the research is ready.",
      "A weekly schedule is active.",
      "Turn on background work and connect Telegram, Discord, or Slack.",
    ]) {
      expect(screen.getByText(detail)).toBeVisible();
    }

    expect(screen.getByText("On demand: Ready")).toBeVisible();
    expect(screen.getByText("Weekly: Needs setup")).toBeVisible();
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBe("1");
    expect(screen.queryByText(/BACKEND|mutation|postcondition|Hero workflow/i)).toBeNull();

    for (const [label, count] of [
      ["Open Projects", 2],
      ["Open Connections", 1],
      ["Open Settings", 1],
      ["Open Tasks", 1],
      ["Open Documents", 1],
      ["Open Scheduled", 2],
    ] as const) {
      const buttons = screen.getAllByRole("button", { name: label });
      expect(buttons).toHaveLength(count);
      buttons.forEach((button) => fireEvent.click(button));
    }

    expect(onNavigate.mock.calls.map(([destination]) => destination)).toEqual([
      "projects",
      "projects",
      "integrations",
      "settings",
      "tasks",
      "artifacts",
      "routines",
      "routines",
    ]);
  });

  it("uses honest generic copy for an unknown requirement and destination", async () => {
    heroStatus = {
      readyOnDemand: false,
      readyWeekly: false,
      requirements: [
        {
          id: "future_requirement",
          state: "ready",
          destination: "future_destination",
          label: "BACKEND FUTURE LABEL",
          detail: "BACKEND FUTURE DETAIL",
        },
      ],
    };
    const onNavigate = vi.fn();
    render(<DecisionBriefScreen onNavigate={onNavigate} />, { wrapper: I18nProvider });

    expect(await screen.findByText("Next step")).toBeVisible();
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBeNull();
    expect(screen.getByText("This step is ready.")).toBeVisible();
    expect(screen.queryByText(/BACKEND FUTURE/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Set this up" }));
    expect(onNavigate).toHaveBeenCalledWith("future_destination");
  });

  it("shows a calm localized error instead of the backend failure", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return [project];
      if (command === "get_weekly_decision_brief_status") {
        throw new Error("PRIVATE BACKEND FAILURE CANARY");
      }
      return null;
    });

    render(<DecisionBriefScreen onNavigate={vi.fn()} />, { wrapper: I18nProvider });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU couldn't check this Project right now. Try again in a moment.",
    );
    expect(screen.queryByText(/PRIVATE BACKEND FAILURE CANARY/)).toBeNull();
  });

  it("gives an empty account a direct path to Projects", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_projects") return [];
      return null;
    });
    const onNavigate = vi.fn();
    render(<DecisionBriefScreen onNavigate={onNavigate} />, { wrapper: I18nProvider });

    expect(
      await screen.findByText("Create a Project first so OOMU knows what to include."),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Open Projects" }));
    expect(onNavigate).toHaveBeenCalledWith("projects");
  });
});
