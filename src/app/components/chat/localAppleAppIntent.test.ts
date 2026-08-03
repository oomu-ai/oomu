import { describe, expect, it } from "vitest";
import {
  detectDirectLocalAppleAppReadRequest,
  detectDirectLocalAppleAppWriteRequest,
  detectDirectLocalCalendarReadRequest,
  detectDirectLocalMailReadRequest,
} from "../ChatScreen";
import {
  hasAmbiguousPrivateAppReadLanguage,
  readOnlyPrivateAppToolForPrompt,
} from "./localAppIntent";

describe("explicit Apple app intent boundaries", () => {
  it("keeps public release notes out of macOS Notes privacy triage", () => {
    const prompt = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";
    expect(hasAmbiguousPrivateAppReadLanguage(prompt)).toBe(false);
    expect(detectDirectLocalAppleAppReadRequest(prompt)).toBeNull();
    expect(readOnlyPrivateAppToolForPrompt(prompt)).toBeNull();
  });

  it.each([
    "Summarize the latest AI news for me.",
    "Review this news article and identify its claims.",
    "Read these meeting notes and summarize them.",
    "List the reminders in this project brief.",
    "Summarize this email text.",
    "Review the project schedule and calendar assumptions.",
    "Show contacts between the two datasets.",
    "Review the photos attached here.",
    "Search for music news.",
    "Show me maps of the affected region.",
    "Summarize these books by publication date.",
    "Review the home page copy.",
    "What is the weather forecast?",
    "Show the stocks mentioned in this report.",
    "Summarize the TV coverage.",
    "Compare these podcasts by topic.",
    "List useful shortcuts for this workflow.",
    "Review these messages before answering.",
    "Review the App Store policy language.",
    "Review FaceTime etiquette for remote teams.",
    "Find my attached report.",
    "Review these freeform notes.",
    "Explain the Keychain Access guidance in this document.",
    "Review the safari itinerary.",
    "Review the System Settings instructions in this guide.",
  ])("keeps ordinary domain language conversational: %s", (prompt) => {
    expect(detectDirectLocalMailReadRequest(prompt)).toBeNull();
    expect(detectDirectLocalCalendarReadRequest(prompt)).toBeNull();
    expect(detectDirectLocalAppleAppReadRequest(prompt)).toBeNull();
    expect(readOnlyPrivateAppToolForPrompt(prompt)).toBeNull();
  });

  it.each([
    ["Open App Store.", "App Store"],
    ["Open Books.", "Books"],
    ["Open Calendar.", "Calendar"],
    ["Open Contacts.", "Contacts"],
    ["Open FaceTime.", "FaceTime"],
    ["Open Find My.", "Find My"],
    ["Open Freeform.", "Freeform"],
    ["Open Home.", "Home"],
    ["Open Keychain Access.", "Keychain Access"],
    ["Open Mail.", "Mail"],
    ["Open Maps.", "Maps"],
    ["Open Messages.", "Messages"],
    ["Open Music.", "Music"],
    ["Launch News.", "News"],
    ["Open Notes.", "Notes"],
    ["Open Photos.", "Photos"],
    ["Open Podcasts.", "Podcasts"],
    ["Open Reminders.", "Reminders"],
    ["Open Safari.", "Safari"],
    ["Open Shortcuts.", "Shortcuts"],
    ["Open Stocks.", "Stocks"],
    ["Open System Settings.", "System Settings"],
    ["Open TV.", "TV"],
    ["Open Weather.", "Weather"],
  ])("retains an explicit Apple app launch: %s", (prompt, appLabel) => {
    expect(detectDirectLocalAppleAppReadRequest(prompt)).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel,
    });
  });

  it.each([
    ["Open the Maps app and show its visible text.", "Maps"],
    ["What does the Weather app show right now?", "Weather"],
    ["Review Apple Books and summarize the visible UI.", "Books"],
    ["Show the Podcasts app UI.", "Podcasts"],
  ])("retains an explicit Apple app UI read: %s", (prompt, appLabel) => {
    expect(detectDirectLocalAppleAppReadRequest(prompt)).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel,
    });
  });

  it.each([
    ["Show my unread emails.", "read_system_emails"],
    ["What is on my calendar?", "read_system_calendar"],
    ["Show my pending reminders.", "read_system_reminders"],
    ["Read my Notes.", "read_system_notes"],
    ["Show my contacts.", "read_system_contacts"],
    ["Show my latest photos.", "read_system_photos"],
    ["List my recently added songs.", "read_system_music"],
    ["What does the Weather app show?", "read_apple_app_ui"],
  ])("binds the private-app bridge for explicit personal/app intent: %s", (prompt, tool) => {
    expect(readOnlyPrivateAppToolForPrompt(prompt)).toBe(tool);
  });
});

describe("explicit Apple app action boundaries", () => {
  it.each([
    "Open the News app and research official web sources for today's headlines.",
    "Show my unread emails and the Weather app.",
    "Show the News app and the Weather app.",
    "Show my unread emails and the Messages app.",
    "Show my calendar and the Messages app.",
    "Show my pending reminders and the Messages app.",
  ])("keeps compound Apple-app work on the planner route: %s", (prompt) => {
    expect(detectDirectLocalMailReadRequest(prompt)).toBeNull();
    expect(detectDirectLocalCalendarReadRequest(prompt)).toBeNull();
    expect(detectDirectLocalAppleAppReadRequest(prompt)).toBeNull();
    expect(readOnlyPrivateAppToolForPrompt(prompt)).toBeNull();
  });

  it("requires an explicit Notes app destination for native note writes", () => {
    expect(
      detectDirectLocalAppleAppWriteRequest("Write these notes into a concise summary."),
    ).toBeNull();
    expect(detectDirectLocalAppleAppWriteRequest("Create notes for the meeting.")).toBeNull();
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Create a note in Apple Notes that says the review is ready.",
      ),
    ).toMatchObject({ toolName: "create_system_note", appLabel: "Notes" });
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Write a note in the Notes app with the text: call Alex.",
      ),
    ).toMatchObject({ toolName: "create_system_note", appLabel: "Notes" });
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Save a note in Notes with the text: call Alex.",
      ),
    ).toMatchObject({ toolName: "create_system_note", appLabel: "Notes" });
  });

  it("routes concise imperative Mail and Calendar checks to native read tools", () => {
    expect(detectDirectLocalMailReadRequest("Check email.")).toMatchObject({
      scope: "recent",
    });
    expect(
      detectDirectLocalCalendarReadRequest(
        "Check the calendar for tomorrow.",
        new Date(2026, 6, 24, 12),
      ),
    ).toMatchObject({ label: "tomorrow" });
  });
});
