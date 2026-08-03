import { describe, expect, it } from "vitest";
import {
  buildDirectLocalMailReadAssistantText,
  detectDirectLocalAppleAppReadRequest,
  detectDirectLocalAppleAppWriteRequest,
  detectDirectLocalCalendarReadRequest,
  detectDirectLocalMailReadRequest,
  isInternalAgentMemoryRequest,
  isUiSnapshotBlocked,
  mimeTypeForChatFile,
  nativeAppleAppApprovalPresentation,
  shouldAnalyzeVisualChatAttachment,
  visualAnalysisRequestForAttachment,
} from "../ChatScreen";
import {
  isFocusedLocalAppShortcutRequest,
} from "../chat/localAppIntent";

describe("ChatScreen Mail intent", () => {
  it("detects read-only unread Mail requests for the local MCP mail tool", () => {
    const detected = detectDirectLocalMailReadRequest(
      "Please check my email client for unread messages and summarize whether any require a reply.",
    );

    expect(detected).toEqual({
      unreadOnly: true,
      maxMessages: 20,
      replyDraft: false,
      scope: "unread",
    });
    expect(detectDirectLocalMailReadRequest("Please check my unread messages.")).toBeNull();
  });

  it("treats the Scenario 1 Test 7 unread-email question as a direct read", () => {
    expect(detectDirectLocalMailReadRequest("Do I have any unread emails?")).toEqual({
      unreadOnly: true,
      maxMessages: 20,
      replyDraft: false,
      scope: "unread",
    });
  });

  it("keeps nearby informational, mutating, and compound requests off the shortcut", () => {
    expect(
      detectDirectLocalMailReadRequest("Do I have to configure Mail before checking email?"),
    ).toBeNull();
    expect(
      detectDirectLocalMailReadRequest("Do I have any unread emails? Mark them as read."),
    ).toBeNull();
    expect(
      detectDirectLocalMailReadRequest(
        "Do I have any unread emails? Then write a summary to unread-summary.md.",
      ),
    ).toBeNull();
    expect(
      detectDirectLocalMailReadRequest(
        "Do I have any unread emails, and what is on my calendar?",
      ),
    ).toBeNull();
  });

  it("keeps unread-or-today Mail requests broad enough to include today's messages", () => {
    const detected = detectDirectLocalMailReadRequest(
      "Check my emails for anything unread or from earlier today that requires a reply.",
    );

    expect(detected).toEqual({
      unreadOnly: false,
      maxMessages: 50,
      replyDraft: false,
      scope: "unread_or_today",
    });
  });

  it("never treats explicit Apple Messages or iMessage UI as Mail", () => {
    const messagesPrompt = "Review Apple Messages for unread mail UI text.";

    expect(detectDirectLocalMailReadRequest(messagesPrompt)).toBeNull();
    expect(detectDirectLocalAppleAppReadRequest(messagesPrompt)).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel: "Messages",
      attachmentName: "local_messages_ui.json",
    });
    expect(detectDirectLocalMailReadRequest("Check iMessage for unread messages.")).toBeNull();
  });

  it("routes email reply drafting through read-only Mail context", () => {
    const detected = detectDirectLocalMailReadRequest(
      "Please review the unread email and draft a reply but not to send anything.",
    );

    expect(detected).toEqual({
      unreadOnly: true,
      maxMessages: 20,
      replyDraft: true,
      scope: "unread",
    });
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Please review the unread email and draft a reply but not to send anything.",
      ),
    ).toBeNull();
  });

  it("formats empty Mail results as a completed check", () => {
    const detected = detectDirectLocalMailReadRequest("Check my unread email.");

    expect(
      buildDirectLocalMailReadAssistantText(detected!, "[]", undefined, new Date(2026, 6, 7, 12)),
    ).toContain("found no matching messages");
    expect(
      buildDirectLocalMailReadAssistantText(detected!, "[]", undefined, new Date(2026, 6, 7, 12)),
    ).toContain("Nothing in that result appears to require a reply.");
  });

  it("formats AppleScript Mail timeouts as failed reads, not an empty inbox", () => {
    const detected = detectDirectLocalMailReadRequest("Go ahead and try to check my email again.");
    const assistantText = buildDirectLocalMailReadAssistantText(
      detected!,
      JSON.stringify({
        error: "AppleScript execution timed out after 5s.",
        emails: [],
      }),
      undefined,
      new Date(2026, 6, 7, 12),
    );

    expect(assistantText).toContain("Mail did not return usable results");
    expect(assistantText).toContain("AppleScript execution timed out after 5s.");
    expect(assistantText).toContain("not because your inbox is clear");
  });
});

describe("ChatScreen Apple app write intent", () => {
  it("presents direct Mail approval as exact native fields, not a generic MCP call", () => {
    const argumentsValue = {
      subject: "Draft from OOMU",
      body: "I am running late.",
    };

    expect(
      nativeAppleAppApprovalPresentation("draft_system_email", argumentsValue),
    ).toEqual({
      actionType: "draft_system_email",
      actionLabel: "Save a Mail draft",
      preview: JSON.stringify(argumentsValue),
    });
    expect(
      nativeAppleAppApprovalPresentation("create_system_note", argumentsValue),
    ).toBeNull();
  });

  it("detects common approval-gated Apple app write requests", () => {
    expect(
      detectDirectLocalAppleAppWriteRequest("Write a new note that says hello."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppWriteRequest("Add a reminder to call Alex."),
    ).toMatchObject({
      toolName: "add_system_reminder",
      appLabel: "Reminders",
    });
    expect(detectDirectLocalAppleAppWriteRequest("Draft an email saying I am running late.")).toBeNull();
    expect(
      detectDirectLocalAppleAppWriteRequest("Open a Mail draft saying I am running late."),
    ).toMatchObject({
      toolName: "draft_system_email",
      appLabel: "Mail",
    });
  });

  it("keeps compound cross-surface work out of the one-action Mail shortcut", () => {
    const prompt =
      "Read mock_data/supplier_proposals.json, reconcile every amount, research current official web sources, create supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md, then create a tentative event in my OOMU Test calendar and create a Mail draft listing the files.";

    expect(isFocusedLocalAppShortcutRequest(prompt, "mail")).toBe(false);
    expect(detectDirectLocalAppleAppWriteRequest(prompt)).toBeNull();
    expect(
      isFocusedLocalAppShortcutRequest(
        "Open a Mail draft saying I am running late.",
        "mail",
      ),
    ).toBe(true);
  });

  it("treats a bare directory path as independent work before a Mail action", () => {
    const prompt =
      "Use /Users/example/testing/mock_data to inform yourself about the inputs, then create a Mail draft saying the review is ready.";

    expect(isFocusedLocalAppShortcutRequest(prompt, "mail")).toBe(false);
    expect(detectDirectLocalAppleAppWriteRequest(prompt)).toBeNull();
  });

  it("keeps OOMU memory requests out of Apple Notes", () => {
    const memoryRequest = "Yes, call me Alex and make a note of that in your memories";

    expect(isInternalAgentMemoryRequest(memoryRequest)).toBe(true);
    expect(detectDirectLocalAppleAppWriteRequest(memoryRequest)).toBeNull();
    expect(detectDirectLocalAppleAppReadRequest(memoryRequest)).toBeNull();
    expect(isInternalAgentMemoryRequest("Call me Alex")).toBe(true);
    expect(isInternalAgentMemoryRequest("Please make a note of that for next time")).toBe(true);
    expect(isInternalAgentMemoryRequest("Please make note of that for next time")).toBe(true);
    expect(isInternalAgentMemoryRequest("Save to your memory that I use Apple Notes")).toBe(true);
    expect(isInternalAgentMemoryRequest("What do you remember about me?")).toBe(false);
    expect(isInternalAgentMemoryRequest("Do you remember my birthday?")).toBe(false);
    expect(
      detectDirectLocalAppleAppWriteRequest("Save to your memory that I use Apple Notes"),
    ).toBeNull();
    expect(
      isInternalAgentMemoryRequest("Remember to set a reminder in Reminders to buy milk"),
    ).toBe(false);
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Remember to set a reminder in Reminders to buy milk",
      ),
    ).toMatchObject({ toolName: "add_system_reminder" });
    expect(isInternalAgentMemoryRequest("Create a note in my Apple Notes app saying hello")).toBe(false);
    expect(
      detectDirectLocalAppleAppWriteRequest("Create a note in my Apple Notes app saying hello"),
    ).toMatchObject({ toolName: "create_system_note" });
    expect(
      detectDirectLocalAppleAppWriteRequest("Make a note of this in the Notes app"),
    ).toMatchObject({ toolName: "create_system_note" });
  });

  it("keeps OOMU project tasks out of Reminders writes unless Reminders is explicit", () => {
    expect(
      detectDirectLocalAppleAppWriteRequest("Create a task in this project."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppWriteRequest(
        "Create a task for this project in Apple Reminders.",
      ),
    ).toMatchObject({
      toolName: "add_system_reminder",
      appLabel: "Reminders",
    });
  });
});

describe("ChatScreen Calendar intent", () => {
  it("detects read-only Calendar requests for tomorrow", () => {
    const detected = detectDirectLocalCalendarReadRequest(
      "Check my calendar to see what events I have going on tomorrow.",
      new Date(2026, 6, 2, 9, 30, 0),
    );

    expect(detected).toEqual({
      calendarName: "",
      startDate: "2026-07-03T00:00:00",
      endDate: "2026-07-03T23:59:59",
      label: "tomorrow",
    });
  });

  it("does not auto-run mutating Calendar requests", () => {
    expect(
      detectDirectLocalCalendarReadRequest("Schedule a meeting with Nora tomorrow."),
    ).toBeNull();
    expect(
      detectDirectLocalCalendarReadRequest("Cancel tomorrow's calendar event."),
    ).toBeNull();
    expect(
      detectDirectLocalCalendarReadRequest("Invite Nora to a calendar event tomorrow."),
    ).toBeNull();
  });

  it("detects availability reads that draft calendar invite text", () => {
    const detected = detectDirectLocalCalendarReadRequest(
      "OOMU, read the project_milestones.json file inside our testing/mock_data/ folder to identify our next pending major product milestone. Then, query my macOS Calendar to find a free 2-hour block next Tuesday, July 7, or Wednesday, July 8, 2026, during standard business hours (9 AM to 5 PM) to schedule an executive review preparation block. Draft a localized calendar invite and meeting brief in both English (en-US) and Spanish (es-ES) for that slot, and save it as calendar_draft.md in the testing folder.",
      new Date(2026, 6, 5, 12, 0, 0),
    );

    expect(detected).toEqual({
      calendarName: "",
      startDate: "2026-07-07T09:00:00",
      endDate: "2026-07-08T17:00:00",
      label: "July 7-8, 2026",
    });
  });
});

describe("ChatScreen Apple app read intent", () => {
  it("detects read-only Reminders, Notes, Contacts, and Weather requests", () => {
    expect(
      detectDirectLocalAppleAppReadRequest("Check my reminders for anything open."),
    ).toMatchObject({
      toolName: "read_system_reminders",
      appLabel: "Reminders",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Do I have any outstanding reminders or tasks?"),
    ).toMatchObject({
      toolName: "read_system_reminders",
      appLabel: "Reminders",
      argumentsValue: {
        completed_only: false,
      },
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Read my Notes and summarize them."),
    ).toMatchObject({
      toolName: "read_system_notes",
      appLabel: "Notes",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Show my contacts."),
    ).toMatchObject({
      toolName: "read_system_contacts",
      appLabel: "Contacts",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Check my Contacts for Kiana Allan."),
    ).toMatchObject({
      toolName: "read_system_contacts",
      appLabel: "Contacts",
      source: "native_contacts/read_system_contacts",
      argumentsValue: {
        max_contacts: 20,
        search_text: "Kiana Allan",
      },
    });
    expect(
      detectDirectLocalAppleAppReadRequest(
        "No, try again to see if you can find Maya Allan in my contacts.",
      ),
    ).toMatchObject({
      toolName: "read_system_contacts",
      appLabel: "Contacts",
      source: "native_contacts/read_system_contacts",
      argumentsValue: {
        max_contacts: 20,
        search_text: "Maya Allan",
      },
    });
    expect(
      detectDirectLocalAppleAppReadRequest("What does the Weather app show right now?"),
    ).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel: "Weather",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Summarize my Messages app UI."),
    ).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel: "Messages",
      attachmentName: "local_messages_ui.json",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Review Apple Messages for unread UI text."),
    ).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel: "Messages",
    });
    expect(
      detectDirectLocalAppleAppReadRequest("Check iMessage for the active thread."),
    ).toMatchObject({
      toolName: "read_apple_app_ui",
      appLabel: "Messages",
    });
  });

  it("keeps OOMU project tasks out of Reminders while preserving personal task shorthand", () => {
    expect(
      detectDirectLocalAppleAppReadRequest("What are my tasks in this project?"),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("Show the tasks in my OOMU workflow."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("Show my pending tasks."),
    ).toMatchObject({
      toolName: "read_system_reminders",
      appLabel: "Reminders",
    });
    expect(
      detectDirectLocalAppleAppReadRequest(
        "What are my tasks in this project in Apple Reminders?",
      ),
    ).toMatchObject({
      toolName: "read_system_reminders",
      appLabel: "Reminders",
    });
  });

  it.each([
    [
      "Search my email and see if there is anything from Maya that needs a reply.",
      "read_system_emails",
    ],
    [
      "Search my calendar and see whether I have anything tomorrow afternoon.",
      "read_system_calendar",
    ],
    [
      "Search my reminders and see if anything for the launch is still open.",
      "read_system_reminders",
    ],
    [
      "Search my Notes and find anything I saved about the compiler review.",
      "read_system_notes",
    ],
    [
      "Search my contacts and see if you can find Maya Allan",
      "read_system_contacts",
    ],
    [
      "Search my photo library and show me the newest process diagram.",
      "read_system_photos",
    ],
    [
      "Search my Apple Music library and list my recently added songs.",
      "read_system_music",
    ],
  ])("keeps private-app search wording on the native route: %s", (prompt, expectedTool) => {
    const toolName = detectDirectLocalMailReadRequest(prompt)
      ? "read_system_emails"
      : detectDirectLocalCalendarReadRequest(prompt)
        ? "read_system_calendar"
        : detectDirectLocalAppleAppReadRequest(prompt)?.toolName;

    expect(toolName).toBe(expectedTool);
  });

  it("extracts the requested person from a natural Contacts search", () => {
    expect(
      detectDirectLocalAppleAppReadRequest(
        "Search my contacts and see if you can find Maya Allan",
      ),
    ).toMatchObject({
      toolName: "read_system_contacts",
      argumentsValue: {
        max_contacts: 20,
        search_text: "Maya Allan",
      },
    });
  });

  it("routes singular Photos-library questions to the native protected-library reader", () => {
    expect(
      detectDirectLocalAppleAppReadRequest(
        "What is the newest photo in my photo albums?",
      ),
    ).toMatchObject({
      toolName: "read_system_photos",
      appLabel: "Photos",
      argumentsValue: {
        max_photos: 1,
      },
      attachmentName: "local_photos.json",
    });

    expect(
      detectDirectLocalAppleAppReadRequest(
        "How does Photos organize shared albums?",
      ),
    ).toBeNull();
  });

  it("routes personal Music-library questions to the native metadata reader", () => {
    expect(
      detectDirectLocalAppleAppReadRequest(
        "Which song did I add most recently to Apple Music?",
      ),
    ).toMatchObject({
      toolName: "read_system_music",
      appLabel: "Music",
      argumentsValue: {
        max_songs: 1,
      },
      source: "native_music/read_system_music",
      attachmentName: "local_music.json",
    });
    expect(
      detectDirectLocalAppleAppReadRequest(
        "Show my recently added songs in the Music app.",
      ),
    ).toMatchObject({
      toolName: "read_system_music",
      argumentsValue: {
        max_songs: 10,
      },
    });
    expect(
      detectDirectLocalAppleAppReadRequest("How does Apple Music organize albums?"),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("What is Apple Music?"),
    ).toBeNull();
  });

  it("does not treat conversational message references as Messages app UI requests", () => {
    expect(
      detectDirectLocalAppleAppReadRequest("Check my last message for details."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("Please review these messages before answering."),
    ).toBeNull();
  });

  it("does not auto-read mutating Apple app requests", () => {
    expect(
      detectDirectLocalAppleAppReadRequest("Write a new note that says hello."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("Add a reminder to call Alex."),
    ).toBeNull();
    expect(
      detectDirectLocalAppleAppReadRequest("Send a message to Sam."),
    ).toBeNull();
  });
});

describe("ChatScreen visual attachment handling", () => {
  it("detects blocked Apple UI snapshots without flagging normal UI payloads", () => {
    expect(isUiSnapshotBlocked("[\"missing value\"]")).toBe(true);
    expect(isUiSnapshotBlocked("[\"missing value\", \"missing value\"]")).toBe(true);
    expect(isUiSnapshotBlocked("[]")).toBe(false);
    expect(isUiSnapshotBlocked("[\"Inbox\", \"missing value\"]")).toBe(false);
    expect(isUiSnapshotBlocked("Tool completed without text output.")).toBe(false);
  });

  it("infers common visual media MIME types when the browser reports no type", () => {
    expect(mimeTypeForChatFile({ name: "screenshot.HEIC", type: "" })).toBe(
      "image/heic",
    );
    expect(mimeTypeForChatFile({ name: "scan.webp", type: "" })).toBe(
      "image/webp",
    );
    expect(mimeTypeForChatFile({ name: "brief.pdf", type: "" })).toBe(
      "application/pdf",
    );
  });

  it("prepares binary visual attachments for local analysis", () => {
    const attachment = {
      name: "screenshot.png",
      mime_type: "image/png",
      byte_count: 128,
      data_base64: "abc123",
    };

    expect(shouldAnalyzeVisualChatAttachment(attachment)).toBe(true);
    expect(visualAnalysisRequestForAttachment(attachment)).toEqual({
      dataBase64: "abc123",
      fileName: "screenshot.png",
      mimeType: "image/png",
    });
  });

  it("does not re-analyze visual attachments that already have extracted text", () => {
    expect(
      shouldAnalyzeVisualChatAttachment({
        name: "scan.pdf",
        mime_type: "application/pdf",
        byte_count: 512,
        data_base64: "abc123",
        text: "Visual analysis for scan.pdf",
      }),
    ).toBe(false);
  });
});
