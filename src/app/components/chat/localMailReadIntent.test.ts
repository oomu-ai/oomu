import { describe, expect, it } from "vitest";
import {
  hasPrivateAppMutationIntent,
  isInformationalLocalSystemTopicQuestion,
} from "./localAppIntent";
import { detectDirectLocalMailReadRequest } from "./localMailReadIntent";

describe("detectDirectLocalMailReadRequest", () => {
  it("recognizes a personal unread-mail state question", () => {
    expect(detectDirectLocalMailReadRequest("Do I have any unread emails?"))
      .toMatchObject({ unreadOnly: true, maxMessages: 20, scope: "unread" });
    expect(detectDirectLocalMailReadRequest("How many unread emails do I have?"))
      .toMatchObject({ unreadOnly: true, scope: "unread" });
    expect(detectDirectLocalMailReadRequest("Show my unread emails from today."))
      .toMatchObject({ unreadOnly: true, maxMessages: 20, scope: "unread" });
  });

  it("keeps informational mail questions conversational", () => {
    const prompt = "How many unread emails are normal?";
    expect(isInformationalLocalSystemTopicQuestion(prompt)).toBe(true);
    expect(detectDirectLocalMailReadRequest(prompt)).toBeNull();
  });

  it.each([
    "Do I have any unread emails? Then run npm test.",
    "Do I have any unread emails? Then post the count to Slack.",
    "Can you set up an hourly task to check my email for any unread messages. Only run for today until midnight tonight. Once you set it up, run it once to ensure it’s working properly. If it does not work properly, report back here and let me know the outcome.",
  ])("does not swallow compound work: %s", (prompt) => {
    expect(detectDirectLocalMailReadRequest(prompt)).toBeNull();
  });

  it("keeps a simple unread-Mail check on the direct read path", () => {
    expect(detectDirectLocalMailReadRequest("Check my email for anything unread"))
      .toMatchObject({ unreadOnly: true, scope: "unread" });
  });

  it.each([
    "Do I have any unread emails? Then flag them.",
    "Do I have any unread emails? Then star them.",
  ])("does not downgrade a Mail mutation to a read: %s", (prompt) => {
    expect(hasPrivateAppMutationIntent(prompt)).toBe(true);
    expect(detectDirectLocalMailReadRequest(prompt)).toBeNull();
  });

  it.each([
    "Do I have any unread emails? Do not mark them as read.",
    "Do I have any unread emails? Move nothing; just summarize.",
    "Do I have any unread emails? Don't flag them.",
  ])("honors an explicit mutation prohibition: %s", (prompt) => {
    expect(hasPrivateAppMutationIntent(prompt)).toBe(false);
    expect(detectDirectLocalMailReadRequest(prompt)).toMatchObject({
      unreadOnly: true,
      scope: "unread",
    });
  });
});
