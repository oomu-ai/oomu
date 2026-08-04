import { describe, expect, it } from "vitest";
import {
  calendarToolFailureMessage,
  candidateLocalPathsFromText,
  canSubmitLocalToolWorkflowWhileHydrating,
  chatFailureNotice,
  containsUnverifiedActionClaim,
  detectDirectLocalCalendarReadRequest,
  detectDirectLocalCommand,
  detectDirectLocalMailReadRequest,
  directExecuteCommandText,
  hasLikelyLocalNativeTaskIntent,
  isInformationalLocalSystemTopicQuestion,
  localCalendarToolAttachment,
  localizePersistedAgentExecutionReceipt,
  localSearchFailureMessage,
  localToolFailureCode,
  mcpCapabilitiesForContextualTurn,
  mcpToolResultText,
  musicToolFailureMessage,
  parseConversationalMcpToolRequest,
  parseLogicalCertificate,
  shouldUseConversationalMcpBridge,
  terminalExecutionStatusFromLogs,
  toolErrorMessage,
  workspaceDataResourceForAttachment,
  type ConversationalMcpToolCapability,
} from "../ChatScreen";
import { inferenceProgressStatus } from "../chat/inferenceProgressStatus";

describe("ChatScreen route status and boundary presentation contracts", () => {
  it("keeps Auto-route progress neutral until native execution identity is known", () => {
    const t = (key: string, variables?: Record<string, string | number>) =>
      key === "chat.status.choosing_model"
        ? "Choosing the best model…"
        : `${key === "chat.status.streaming_model" ? "Streaming" : "Contacting"} ${variables?.model}`;
    expect(inferenceProgressStatus("contacting", true, "Gemma 4 E4B", t)).toBe(
      "Choosing the best model…",
    );
    expect(inferenceProgressStatus("streaming", true, "dynamic", t)).toBe(
      "Choosing the best model…",
    );
    expect(inferenceProgressStatus("streaming", false, "Gemini 3.5 Flash", t)).toBe(
      "Streaming Gemini 3.5 Flash",
    );
  });

  it("omits chained credential and URL details at the provider failure boundary", () => {
    const error = new Error(
      "request https://api.telegram.org/bot123456:telegram-canary/getUpdates?token=query-canary",
      { cause: { apiKey: "nested-canary" } },
    );
    Object.assign(error, { code: "provider_network_error", Authorization: "Bearer auth-canary" });
    const notice = chatFailureNotice(error);
    for (const canary of [
      "telegram-canary",
      "query-canary",
      "nested-canary",
      "auth-canary",
    ]) {
      expect(notice.content).not.toContain(canary);
    }
    expect(notice.content).toContain("Your message is saved and nothing was changed");
    expect(notice.content).not.toContain("[redacted]");
  });

  it("maps credential and empty payload failures without exposing raw internals", () => {
    const credentialNotice = chatFailureNotice(
      JSON.stringify({
        code: "credential_unavailable",
        message: "No API key is available for provider_id 'openai'.",
      }),
    );
    const emptyPayloadNotice = chatFailureNotice({
      code: "provider_response_error",
      message: "OpenAI returned an empty response.",
    });

    expect(credentialNotice.status).toBe("Provider credentials required");
    expect(credentialNotice.content).toContain("Provider credentials are missing or invalid");
    expect(emptyPayloadNotice.status).toBe("Provider response issue");
    expect(emptyPayloadNotice.content).toContain("unusable response");
  });

  it("collapses direct-route certificates with token-split section labels", () => {
    const parsed = parseLogicalCertificate(
      [
        "Hello, Alex. I am here.",
        "",
        "***",
        "",
        "### Logical Certificate",
        "1. **Prem ises:** The Principal initiated contact.",
        "2. **Execution Path:** Acknowledged readiness.",
        "3. **Conclusion:** Standing by.",
        "4. **State:** Awaiting Directive.",
      ].join("\n"),
    );

    expect(parsed.mainContent).toBe("Hello, Alex. I am here.");
    expect(parsed.certificate).toContain("Premises:\nThe Principal initiated contact.");
    expect(parsed.certificate).toContain("Formal Conclusion:\nStanding by.");
    expect(parsed.certificate).not.toContain("State:");
  });

  it("formats workspace boundary violations as boundary blocks", () => {
    const notice = chatFailureNotice({
      code: "workspace_boundary_violation",
      message:
        "Cognitive boundary rejected payload. Matched scope: message[0] role=user: Eldris database",
    });

    expect(notice.status).toBe("Request blocked");
    expect(notice.content).toContain(
      "This request tried to leave OOMU's protected workspace, so it was stopped before it ran.",
    );
    expect(notice.content).toContain("Matched scope:");
  });
});

describe("ChatScreen conversational MCP capability contracts", () => {
  it("parses only explicit conversational MCP tool-call fences", () => {
    const parsed = parseConversationalMcpToolRequest(
      [
        "I need local context.",
        "```oomu_mcp_tool_call",
        "{\"serverName\":\"local_filesystem\",\"toolName\":\"read_file\",\"arguments\":{\"path\":\"notes.txt\"}}",
        "```",
      ].join("\n"),
    );

    expect(parsed?.call).toEqual({
      serverName: "local_filesystem",
      toolName: "read_file",
      argumentsValue: { path: "notes.txt" },
    });
    expect(parseConversationalMcpToolRequest("{\"toolName\":\"read_file\"}")).toBeNull();
  });

  it("keeps informational local app questions conversational", () => {
    expect(isInformationalLocalSystemTopicQuestion("How do I configure my email application?")).toBe(true);
    expect(isInformationalLocalSystemTopicQuestion("What is a calendar?")).toBe(true);
    expect(detectDirectLocalMailReadRequest("How do I configure my email application?")).toBeNull();
    expect(detectDirectLocalCalendarReadRequest("What is a calendar?", new Date(2026, 6, 7))).toBeNull();
    expect(detectDirectLocalCalendarReadRequest("What is on my calendar today?", new Date(2026, 6, 7))).toEqual(
      expect.objectContaining({
        label: "today",
      }),
    );
  });

  it("exposes the connected MCP catalog on every route while suppressing duplicate private reads", () => {
    const capabilities: ConversationalMcpToolCapability[] = [
      {
        serverName: "macos_applescript",
        toolName: "read_system_emails",
        description: "Read Mail",
        inputSchema: {},
      },
      {
        serverName: "macos_applescript",
        toolName: "read_system_calendar",
        description: "Read Calendar",
        inputSchema: {},
      },
      {
        serverName: "local_filesystem",
        toolName: "read_file",
        description: "Read file",
        inputSchema: {},
      },
    ];
    const conversationalDecision = {
      route: "conversational_stream" as const,
      requires_local_access: false,
      decision_source: "contextual_informational_topic_filter",
      confidence: 0.95,
      reason: "informational",
      matched_signals: [],
      status_label: "OOMU is typing...",
    };
    const actionDecision = {
      ...conversationalDecision,
      route: "agentic_planner" as const,
      requires_local_access: true,
      confidence: 0.99,
    };

    expect(mcpCapabilitiesForContextualTurn(capabilities, conversationalDecision, [])).toEqual(
      capabilities,
    );
    expect(workspaceDataResourceForAttachment({ name: "local_mail.json", text: "[]" })).toBe("mail");
    expect(
      workspaceDataResourceForAttachment({
        name: "local_photos.json",
        text: "Source: native_photos/read_system_photos",
      }),
    ).toBe("photos");
    expect(
      workspaceDataResourceForAttachment({
        name: "local_music.json",
        text: "Source: native_music/read_system_music",
      }),
    ).toBe("music");
    expect(
      mcpCapabilitiesForContextualTurn(capabilities, actionDecision, [
        { name: "local_mail.json", text: "[]" },
      ]).map((capability) => capability.toolName),
    ).toEqual(["read_system_calendar", "read_file"]);

    expect(
      mcpCapabilitiesForContextualTurn(capabilities, {
        ...conversationalDecision,
        decision_source: "frontend_ambiguous_local_app_filter",
      }, []).map((capability) => capability.toolName),
    ).toEqual(["read_system_emails", "read_system_calendar", "read_file"]);
  });
});

describe("ChatScreen direct local intent contracts", () => {
  it("detects standard macOS user folders as local context paths", () => {
    expect(candidateLocalPathsFromText("List my Downloads folder.")).toEqual(["~/Downloads"]);
    expect(candidateLocalPathsFromText("Inspect the Desktop directory.")).toEqual(["~/Desktop"]);
    expect(candidateLocalPathsFromText("Summarize the document I attached.")).toEqual([]);
  });

  it("does not treat literal XML-like tag text as a local context path", () => {
    expect(candidateLocalPathsFromText("The literal </text> tag leaked into the chat.")).toEqual([]);
    expect(candidateLocalPathsFromText('Please display "</text>" as normal text.')).toEqual([]);
    expect(candidateLocalPathsFromText("Now inspect /Users/example/Project.")).toEqual([
      "/Users/example/Project",
    ]);
  });

  it("does not treat single-token slash commands as local context paths", () => {
    expect(candidateLocalPathsFromText("/compact")).toEqual([]);
    expect(candidateLocalPathsFromText("Ask OOMU /help before answering.")).toEqual([]);
    expect(candidateLocalPathsFromText("Now inspect /tmp/report.md.")).toEqual([
      "/tmp/report.md",
    ]);
  });

  it("detects direct local read, list, write, delete, and shell commands before inference", () => {
    expect(
      detectDirectLocalCommand(
        "Can you view this file? '/Users/example/Desktop/Screenshot 2026-07-13 at 21.39.23.png'",
      ),
    ).toEqual({
      kind: "read",
      path: "/Users/example/Desktop/Screenshot 2026-07-13 at 21.39.23.png",
    });
    expect(
      detectDirectLocalCommand("Review the file '/Users/example/Desktop/calendar.pdf'."),
    ).toEqual({
      kind: "read",
      path: "/Users/example/Desktop/calendar.pdf",
    });
    expect(detectDirectLocalCommand("run command ls")).toEqual({
      kind: "list",
      path: "",
    });
    expect(detectDirectLocalCommand("List the files in ~/Downloads.")).toEqual({
      kind: "list",
      path: "~/Downloads",
    });
    const explicitTerminalListing =
      "Go into terminal and run a directory listing of my Downloads directory using the ls command";
    expect(detectDirectLocalCommand(explicitTerminalListing)).toEqual({
      kind: "shell",
      command: "ls ~/Downloads",
    });
    expect(
      detectDirectLocalCommand(
        String.raw`List the contents of this directory: /Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/Eldris`,
      ),
    ).toEqual({
      kind: "list",
      path: "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/Eldris",
    });
    expect(detectDirectLocalCommand("delete the file test-output.txt")).toEqual({
      kind: "delete",
      path: "test-output.txt",
    });
    expect(
      detectDirectLocalCommand("Please delete the file test-output.txt."),
    ).toEqual({
      kind: "delete",
      path: "test-output.txt",
    });
    expect(detectDirectLocalCommand("Delete report-before-final.md.")).toEqual({
      kind: "delete",
      path: "report-before-final.md",
    });
    for (const prompt of [
      "Do not delete the file test-output.txt.",
      "Never delete /Users/example/Desktop/report.md.",
      "Delete test-output.txt and then write replacement.txt.",
      "Delete test-output.txt, but first review it.",
      "Review the folder and delete test-output.txt.",
      "Delete test-output.txt? Actually, no.",
    ]) {
      expect(detectDirectLocalCommand(prompt), prompt).toBeNull();
    }
    expect(
      detectDirectLocalCommand(
        'Create a new markdown file in my Downloads directory called Hello World.md with the content "Hello World".',
      ),
    ).toEqual({
      kind: "write",
      path: "~/Downloads/Hello World.md",
      content: "Hello World",
    });
    expect(
      detectDirectLocalCommand('run command echo "Hello World" > "/Users/example/Downloads/Hello World.md"'),
    ).toEqual({
      kind: "write",
      path: "/Users/example/Downloads/Hello World.md",
      content: "Hello World",
    });
    expect(detectDirectLocalCommand("run command npm test")).toEqual({
      kind: "shell",
      command: "npm test",
    });
    expect(detectDirectLocalCommand("Run npm test in the workspace.")).toEqual({
      kind: "shell",
      command: "npm test",
    });
    expect(
      detectDirectLocalCommand(
        "Ask Alex to review /Users/example/Missing.md before answering.",
      ),
    ).toBeNull();
    expect(
      detectDirectLocalCommand("Can you view this file? https://example.test/report.png"),
    ).toBeNull();
  });
});

describe("ChatScreen deterministic local execution contracts", () => {
  it("allows deterministic local tool work while the local model is hydrating", () => {
    expect(canSubmitLocalToolWorkflowWhileHydrating("Run npm test in the workspace.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Check my unread email.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Open Safari and show me what is visible.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Create a reminder to call Alex tomorrow.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Find the screenshot on my Desktop.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Run system diagnostics.")).toBe(true);
    expect(canSubmitLocalToolWorkflowWhileHydrating("Tell me a story about the release.")).toBe(false);
  });

  it("recognizes common local Mac task intent without treating ordinary writing as local work", () => {
    expect(hasLikelyLocalNativeTaskIntent("Open Safari and show me what is visible.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Create a note saying buy milk.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("What is on my calendar today?")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Take a screenshot of my screen.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Record my screen.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Copy this text to my clipboard.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Launch the Calculator app.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Run npm test in the workspace.")).toBe(true);
    expect(hasLikelyLocalNativeTaskIntent("Write a poem about the release.")).toBe(false);
    expect(hasLikelyLocalNativeTaskIntent("Tell me a story about the release.")).toBe(false);
  });

  it("identifies completed first-person actuation claims without blocking hypotheticals", () => {
    expect(containsUnverifiedActionClaim("I've committed your profile to durable memory.")).toBe(true);
    expect(containsUnverifiedActionClaim("I successfully wrote the report to Downloads.")).toBe(true);
    expect(containsUnverifiedActionClaim("If I wrote the report, I would ask for approval first.")).toBe(false);
    expect(containsUnverifiedActionClaim("You can run npm test from Terminal.")).toBe(false);
  });

  it("never manufactures command success when the native receipt has no result", () => {
    expect(
      directExecuteCommandText(
        {
          operation: "file_write",
          status: "completed",
          message: "",
          verified: true,
        },
        "No verified file result was returned.",
        {
          failurePrefix: "Local command failed.",
          receiptPrefix: "Verified native receipt",
        },
      ),
    ).toBe("Local command failed. No verified file result was returned.");

    expect(
      directExecuteCommandText(
        {
          operation: "shell_command",
          status: "completed",
          message: "",
          verified: true,
          claims: ["CLAIM command_exit status=0"],
        },
        "No verified command result was returned.",
        {
          failurePrefix: "Local command failed.",
          receiptPrefix: "Verified native receipt",
        },
      ),
    ).toContain("CLAIM command_exit status=0");
  });

  it("replaces native file-write internals with localized recovery guidance", () => {
    expect(
      directExecuteCommandText(
        {
          operation: "file_write",
          status: "failed",
          message:
            "The approved file changed before OOMU could save it. Nothing was changed.",
          verified: false,
        },
        "No verified file result was returned.",
        {
          failurePrefix: "Local command failed.",
          receiptPrefix: "Verified native receipt",
          fileChangedBeforeSave:
            "The file changed before OOMU could save it. Review the current file, then try again. The newer file was not replaced.",
        },
      ),
    ).toBe(
      "Local command failed. The file changed before OOMU could save it. Review the current file, then try again. The newer file was not replaced.",
    );
  });
});

describe("ChatScreen verified MCP failure contracts", () => {
  it("fails closed when an MCP result has no verifiable output", () => {
    expect(() =>
      mcpToolResultText({
        content: [],
        isError: false,
      }, {
        toolFailureWithoutDetails: "Local tool reported a failure without details.",
        toolResultMissing: "Local tool returned no verifiable result.",
      }),
    ).toThrow("Local tool returned no verifiable result.");
  });

  it("reduces MCP timeout envelopes to typed, user-safe Calendar copy", () => {
    const result = {
      content: [
        {
          type: "text",
          text: JSON.stringify({
            content: [{ type: "text", text: { error: "AppleScript execution timed out after 20s.", events: [] } }],
            isError: true,
            structuredContent: {
              warning: "timeout",
              error: "AppleScript execution timed out after 20s.",
              events: [],
            },
          }),
        },
      ],
      structuredContent: null,
      isError: true,
    };

    expect(localToolFailureCode(result)).toBe("timeout");
    expect(() =>
      mcpToolResultText(result, {
        toolFailureWithoutDetails: "Local tool failed safely.",
        toolResultMissing: "Local tool returned no result.",
      }),
    ).toThrow("Local tool failed safely.");
    expect(calendarToolFailureMessage(result)).toBe(
      "Calendar took too long to respond. Try again.",
    );
    expect(calendarToolFailureMessage(result)).not.toContain("structuredContent");
    expect(calendarToolFailureMessage(result)).not.toContain("null");
  });

  it("recognizes native privacy failures from bounded structured codes", () => {
    expect(
      localToolFailureCode({
        isError: true,
        structuredContent: {
          code: "photos_permission_denied",
          message: "Photos access is off.",
        },
      }),
    ).toBe("permission");
    expect(
      localToolFailureCode({
        isError: true,
        structuredContent: {
          code: "music_authorization_timeout",
          message: "Music took too long to respond.",
        },
      }),
    ).toBe("timeout");
    expect(
      musicToolFailureMessage({
        isError: true,
        structuredContent: {
          code: "music_authorization_timeout",
          message: "Music took too long to respond.",
        },
      }),
    ).toBe("Music took too long to respond. Try again.");
    expect(
      musicToolFailureMessage({
        isError: true,
        structuredContent: {
          code: "music_permission_denied",
          message: "Media & Apple Music access is off.",
        },
      }),
    ).toBe(
      "Media & Apple Music access is off. Allow OOMU in System Settings, then try again.",
    );
    expect(
      localToolFailureCode({
        isError: true,
        structuredContent: {
          code: "contacts_permission_restricted",
          message: "Contacts access is restricted on this Mac.",
        },
      }),
    ).toBe("permission");
  });
});

describe("ChatScreen nested MCP envelope contracts", () => {
  it("rejects a serialized nested MCP error even when the outer envelope says success", () => {
    const result = {
      content: [{
        type: "text",
        text: JSON.stringify({
          content: [{ type: "text", text: "AppleScript execution timed out after 20s." }],
          structuredContent: { warning: "timeout", events: [] },
          isError: true,
        }),
      }],
      structuredContent: null,
      isError: false,
    };

    expect(() =>
      mcpToolResultText(result, {
        toolFailureWithoutDetails: "Local tool failed safely.",
        toolResultMissing: "Local tool returned no result.",
      }),
    ).toThrow("Local tool failed safely.");
    expect(calendarToolFailureMessage(result)).toBe(
      "Calendar took too long to respond. Try again.",
    );
  });

  it("unwraps successful nested MCP envelopes without exposing their transport fields", () => {
    const result = {
      content: [{
        type: "text",
        text: JSON.stringify({
          content: [{ type: "text", text: "One verified event." }],
          structuredContent: null,
          isError: false,
        }),
      }],
      structuredContent: null,
      isError: false,
    };

    const text = mcpToolResultText(result, {
      toolFailureWithoutDetails: "Local tool failed safely.",
      toolResultMissing: "Local tool returned no result.",
    });
    expect(text).toBe("One verified event.");
    expect(text).not.toContain("structuredContent");
    expect(text).not.toContain("isError");
  });

  it("treats null-only MCP content as missing instead of displaying null", () => {
    expect(() =>
      mcpToolResultText(
        {
          content: [{ type: "text", text: "null" }],
          structuredContent: null,
          isError: false,
        },
        {
          toolFailureWithoutDetails: "Local tool failed safely.",
          toolResultMissing: "Local tool returned no result.",
        },
      ),
    ).toThrow("Local tool returned no result.");
  });

  it("replaces prefixed raw MCP error envelopes with safe copy", () => {
    const detail = `Calendar failed: ${JSON.stringify({
      content: [{ type: "text", text: "timeout" }],
      structuredContent: { warning: "timeout" },
      isError: true,
    })}`;
    expect(toolErrorMessage(new Error(detail))).toBe("Local tool was unavailable.");
    expect(toolErrorMessage(new Error("null"))).toBe("Local tool was unavailable.");
  });
});

describe("ChatScreen Calendar provenance contracts", () => {
  it("labels Calendar provenance from the backend that actually produced the result", () => {
    const request = {
      calendarName: "",
      startDate: "2026-07-13T00:00:00",
      endDate: "2026-07-14T00:00:00",
      label: "tomorrow",
    };
    const native = localCalendarToolAttachment(request, "[]", { backend: "eventkit" });
    const fallback = localCalendarToolAttachment(request, "[]", { backend: "applescript" });

    expect(native.text).toContain("Source: Local Calendar (EventKit)");
    expect(native.text).not.toContain("macos_applescript/read_system_calendar");
    expect(fallback.text).toContain("Source: Local Calendar (AppleScript fallback)");
  });

  it("uses one canonical Calendar payload instead of duplicating content and structured events", () => {
    const resultText = mcpToolResultText(
      {
        content: [{
          type: "text",
          text: JSON.stringify([{ name: "Design review", startTime: "2026-07-13T10:00:00-04:00" }]),
        }],
        structuredContent: {
          backend: "eventkit",
          timeZone: "America/New_York",
          events: [{ name: "Design review", startTime: "2026-07-13T10:00:00-04:00" }],
          returnedCount: 1,
          matchedCount: 1,
          truncated: false,
        },
        isError: false,
      },
      {
        toolFailureWithoutDetails: "Local tool failed safely.",
        toolResultMissing: "Local tool returned no result.",
      },
    );

    expect(resultText.match(/Design review/g)).toHaveLength(1);
    expect(resultText).toContain('"backend": "eventkit"');
    expect(resultText).toContain('"truncated": false');
  });
});

describe("ChatScreen atomic execution boundary contracts", () => {
  it("treats a terminal execution batch without a terminal receipt as failed", () => {
    expect(
      terminalExecutionStatusFromLogs([
        {
          id: 1,
          executionId: "execution-1",
          planId: "plan-1",
          level: "info",
          phase: "running",
          message: "Last observed step was still running.",
          createdAtMs: 1,
        },
      ]),
    ).toBe("failed");
  });

  it("keeps compound file-and-calendar requests on the generic request-driven path", () => {
    const prompt = "Read /Users/demo/planning/roadmap.json, check my calendar next Thursday, and draft the result at /Users/demo/out/review.md.";

    expect(detectDirectLocalCommand(prompt)).toBeNull();
    expect(candidateLocalPathsFromText(prompt)).toEqual([
      "/Users/demo/planning/roadmap.json",
      "/Users/demo/out/review.md",
    ]);
  });

  it("never turns the Scenario 6 recovery workflow into a direct write", () => {
    const prompt = String.raw`Read /Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json. Retrieve one current primary or official public source relevant to US freight or fuel conditions. Create ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md containing the local variances, live source URL/access time, risk assessment, and next actions. If any supplier's active quote exceeds its historical settled rate, create one 30-minute event titled Supplier Exception Follow-up in the OOMU Test calendar on the next conflict-free weekday at 2:00 PM or later, and send one email to recipient@example.com with subject OOMU Test — Supplier Exception and the report attached or linked. These Calendar and send actions require explicit user approval. If approval is pending, preserve the run and resume from that exact step after approval. Never create duplicate events, messages, reports, or deliveries when retrying or recovering. Finally, deliver the run result and exact report filename to the configured private channel.`;

    expect(detectDirectLocalCommand(prompt)).toBeNull();
  });

  it("keeps research-and-write turns out of the atomic direct-write shortcut", () => {
    const prompt =
      "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to ship_test_04/background_agent_comparison.md in my testing folder. Include URLs, access times, explicit limitations, and OOMU implications. Read the file back before claiming completion.";

    expect(detectDirectLocalCommand(prompt)).toBeNull();
  });

  it("requires one output and explicit content for an atomic direct write", () => {
    expect(
      detectDirectLocalCommand(
        'Create a markdown file called result.md with the content "Ready".',
      ),
    ).toEqual({ kind: "write", path: "result.md", content: "Ready" });
    expect(
      detectDirectLocalCommand(
        "Read input.json and create result.md with the content \"Ready\".",
      ),
    ).toBeNull();
    expect(detectDirectLocalCommand("Create result.md.")).toBeNull();
    expect(detectDirectLocalCommand("Create an empty file called placeholder.txt.")).toEqual({
      kind: "write",
      path: "placeholder.txt",
      content: "",
    });
  });
});

describe("ChatScreen request-driven tool catalog contracts", () => {
  it("uses connected catalog membership instead of English tool phrases for non-structural work", () => {
    const routeDecision = {
      route: "agentic_planner" as const,
      requires_local_access: true,
      decision_source: "heuristic_filter" as const,
      confidence: 1,
      reason: "test",
      matched_signals: [],
      status_label: "test",
    };
    const capabilities: ConversationalMcpToolCapability[] = [
      {
        serverName: "local_filesystem",
        toolName: "list_directory",
        description: "List files",
        inputSchema: {},
      },
      {
        serverName: "local_filesystem",
        toolName: "read_file",
        description: "Read file",
        inputSchema: {},
      },
    ];

    expect(
      shouldUseConversationalMcpBridge(
        "Run a listing of my Downloads folder.",
        routeDecision,
        capabilities,
      ),
    ).toBe(true);
    expect(
      shouldUseConversationalMcpBridge(
        "Execute a list of the Desktop folder.",
        routeDecision,
        capabilities,
      ),
    ).toBe(true);
    expect(
      shouldUseConversationalMcpBridge(
        "Run npm test in the workspace.",
        routeDecision,
        capabilities,
      ),
    ).toBe(false);
    expect(
      shouldUseConversationalMcpBridge(
        "Delete my Downloads folder.",
        routeDecision,
        capabilities,
      ),
    ).toBe(true);

    for (const prompt of [
      'Create a PDF document containing "Hello World".',
      'Create a Word doc with "Hello World".',
      'Create a PowerPoint presentation containing "Hello World".',
      'Create an Excel spreadsheet containing "Hello World".',
    ]) {
      expect(
        shouldUseConversationalMcpBridge(
          prompt,
          { ...routeDecision, decision_source: "native_artifact_creation_filter" },
          capabilities,
        ),
        `${prompt} must stay on the verified native artifact path`,
      ).toBe(false);
    }

    const mailCapabilities: ConversationalMcpToolCapability[] = [{
      serverName: "macos_applescript",
      toolName: "read_system_emails",
      description: "Read Mail",
      inputSchema: {},
    }];
    expect(
      shouldUseConversationalMcpBridge(
        "Do I have any unread emails?",
        routeDecision,
        mailCapabilities,
      ),
    ).toBe(true);
    expect(
      shouldUseConversationalMcpBridge(
        "Do I have any unread emails?",
        routeDecision,
        [{ ...mailCapabilities[0], toolName: "read_system_calendar" }],
      ),
    ).toBe(true);
    expect(
      shouldUseConversationalMcpBridge(
        "Do I have any unread emails? Mark them as read.",
        routeDecision,
        mailCapabilities,
      ),
    ).toBe(true);
    expect(
      shouldUseConversationalMcpBridge(
        "Do I have any unread emails, and what is on my calendar?",
        routeDecision,
        mailCapabilities,
      ),
    ).toBe(true);

    expect(
      shouldUseConversationalMcpBridge(
        "¿Qué tengo en mi calendario mañana?",
        routeDecision,
        [{
          serverName: "macos_applescript",
          toolName: "read_system_calendar",
          description: "Read Calendar",
          inputSchema: {},
        }],
      ),
    ).toBe(true);
  });
});

describe("ChatScreen search and planner recovery presentation contracts", () => {
  it("maps native search failures to concise localized copy without raw internals", () => {
    const copy: Record<string, string> = {
      "chat.search_errors.not_authorized": "Search was not sent.",
      "chat.search_errors.query_invalid": "Name a search topic.",
      "chat.search_errors.unavailable": "Web search is unavailable.",
    };
    const translate = (key: string) => copy[key] ?? key;

    expect(localSearchFailureMessage("search_not_authorized", translate)).toBe(
      "Search was not sent.",
    );
    expect(localSearchFailureMessage("search_query_invalid", translate)).toBe(
      "Name a search topic.",
    );
    expect(localSearchFailureMessage("socket failed: secret=abc", translate)).toBe(
      "Web search is unavailable.",
    );
  });

  it("keeps response-claim conflicts simple and hides persistence internals", () => {
    const duplicateNotice = chatFailureNotice({
      code: "chat_turn_already_running",
      message: "This message is already being answered.",
    });
    const persistenceNotice = chatFailureNotice({
      code: "chat_turn_persistence_failed",
      message: "OOMU could not reserve this response. Try again.",
    });

    expect(duplicateNotice).toEqual({
      status: "Reply already in progress",
      content: "OOMU is already working on this message. Its reply will appear here when it is ready.",
    });
    expect(persistenceNotice).toEqual({
      status: "Couldn't start reply",
      content: "OOMU couldn't start this reply safely. Try again.",
    });
    expect(duplicateNotice.content).not.toContain("chat_turns");
    expect(persistenceNotice.content).not.toContain("SQLite");
  });

  it("keeps Auto-route failures actionable without exposing classifier internals", () => {
    const notice = chatFailureNotice({
      code: "classifier_inference_failed",
      message: "llama.cpp worker 4 leaked prompt canary and raw grammar state",
    });

    expect(notice).toEqual({
      status: "Auto-route needs attention",
      content: "Auto-route couldn’t choose a model. Nothing was sent.",
    });
    expect(notice.content).not.toContain("llama.cpp");
    expect(notice.content).not.toContain("prompt canary");
  });

  it("turns unusable planner output into calm guidance without planner internals", () => {
    const notice = chatFailureNotice({
      code: "planner_output_unusable",
      message:
        "Gateway response did not contain parseable ActionPlan JSON. Local planner prompt safety valve compressed 25122 characters into the 6000 character envelope before inference.",
    });

    expect(notice).toEqual({
      status: "Couldn't prepare this action",
      content: "OOMU couldn't prepare a safe action plan. Nothing was changed. Try again.",
    });
    expect(notice.content).not.toContain("Gateway");
    expect(notice.content).not.toContain("ActionPlan JSON");
    expect(notice.content).not.toContain("25122");
    expect(notice.content).not.toContain("planner_output_unusable");
  });

  it("keeps planner compilation and size failures concise and private", () => {
    const compilationNotice = chatFailureNotice({
      code: "planner_prompt_compilation_failed",
      message: "Planner prompt exceeded an internal post-compilation invariant.",
    });
    const oversizedNotice = chatFailureNotice({
      code: "planner_objective_too_large",
      message: "Mandatory prompt was 9124 characters for a 6000 character envelope.",
    });

    expect(compilationNotice).toEqual({
      status: "Couldn't prepare this action",
      content: "OOMU couldn't prepare a safe action plan. Nothing was changed. Try again.",
    });
    expect(oversizedNotice).toEqual({
      status: "Request is too long",
      content:
        "This request is too long to plan safely. Shorten it and try again. Nothing was changed.",
    });
    expect(compilationNotice.content).not.toContain("planner_prompt_compilation_failed");
    expect(compilationNotice.content).not.toContain("invariant");
    expect(oversizedNotice.content).not.toContain("6000");
    expect(oversizedNotice.content).not.toContain("planner_objective_too_large");
  });
});

describe("ChatScreen persistence and verification recovery contracts", () => {
  it("keeps a rejected local action from exposing planner control-flow details", () => {
    const notice = chatFailureNotice({
      code: "agent_objective_not_executable",
      message: "This private app read is handled directly and does not require an ActionPlan.",
    });

    expect(notice).toEqual({
      status: "Couldn't prepare this action on your Mac",
      content:
        "OOMU couldn't prepare this request safely on your Mac. Nothing was changed. Try again.",
    });
    expect(notice.content).not.toContain("ActionPlan");
    expect(notice.content).not.toContain("agent_objective_not_executable");
  });

  it("keeps file-creation failures simple and hides writer internals", () => {
    const notice = chatFailureNotice({
      code: "file_creation_failed",
      message:
        "artifact renderer exited 17 while validating /private/internal/verification-canary.pdf",
    });

    expect(notice).toEqual({
      status: "Couldn’t create the file",
      content:
        "OOMU couldn’t create and check this file. Nothing was changed. Try a different file name or format.",
    });
    expect(notice.content).not.toContain("artifact renderer");
    expect(notice.content).not.toContain("/private/internal");
    expect(notice.content).not.toContain("file_creation_failed");
  });

  it("keeps final verification failures simple and hides certificate internals", () => {
    const notice = chatFailureNotice({
      code: "mlc_verification_failed",
      message:
        "MLC verification failed for 1 claim(s). Unknown MLC claim: local_file_created",
    });

    expect(notice).toEqual({
      status: "Couldn’t confirm the result",
      content:
        "OOMU couldn’t safely confirm the result, so it stopped. Check the file, then try again.",
    });
    expect(notice.content).not.toContain("MLC");
    expect(notice.content).not.toContain("claim");
  });

  it("replaces persisted recovery diagnostics with localized user copy", () => {
    expect(
      localizePersistedAgentExecutionReceipt(
        [
          "Recovery Loop",
          "Agent: OOMU",
          "Boundary: MlcVerifier",
          "Reason: MLC verification failed for 1 claim(s).",
        ].join("\n"),
      ),
    ).toBe(
      "OOMU couldn’t safely confirm the result, so it stopped. Check the file, then try again.",
    );
    expect(
      localizePersistedAgentExecutionReceipt(
        [
          "Recovery Loop",
          "Agent: OOMU",
          "Boundary: Calendar",
          "Reason: Calendar was unavailable.",
        ].join("\n"),
      ),
    ).toBe(
      "OOMU couldn’t safely confirm the result, so it stopped. Review the task, then try again.",
    );
  });
});
