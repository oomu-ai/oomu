import { describe, expect, it } from "vitest";
import {
  browserFeedbackIndicatesFailedNavigation,
  browserNavigationBlockPayload,
  browserNavigationBlockedNotice,
  browserSearchFallbackQuery,
  chatFailureNotice,
  contextBudgetBoundsForProvider,
  contextBudgetToneForValue,
  latestBrowserSplitRoute,
  latestVerticalTemplateRoute,
  localCommandFailureText,
  normalizeLogicalCertificate,
  oomuBypassNotice,
  parseBrowserSplitViewPayload,
  parseLogicalCertificate,
  parseVerticalTemplatePayload,
  planningPreferenceForProvider,
  releaseAttachmentPayloads,
} from "../ChatScreen";
import {
  cloudConfiguredProviders,
  configuredProviders,
  geminiConfiguredProviders,
} from "./ChatScreen.fixtures";

describe("ChatScreen presentation contracts", () => {
  it("releases attachment payload references after terminal outcomes", () => {
    const attachment = {
      name: "private.txt",
      mime_type: "text/plain",
      byte_count: 14,
      text: "private-canary",
      data_base64: "encoded-canary",
    };
    const attachments = [attachment];
    releaseAttachmentPayloads(attachments);
    expect(attachments).toHaveLength(0);
    expect(attachment.text).toBeUndefined();
    expect(attachment.data_base64).toBeUndefined();
  });
  it("maps configured Gemini providers to the Gemini planner preference", () => {
    expect(planningPreferenceForProvider(geminiConfiguredProviders, "gemini-provider-1")).toBe(
      "gemini_pro",
    );
    expect(planningPreferenceForProvider(cloudConfiguredProviders, "cloud-provider-1")).toBe(
      "chat_gpt",
    );
  });

  it("maps provider network failures to a clean warning state", () => {
    const notice = chatFailureNotice({
      code: "provider_network_error",
      message: "reqwest error: operation timed out",
    });

    expect(notice.status).toBe("Provider temporarily unavailable");
    expect(notice.content).toContain("after several attempts");
    expect(notice.content).toContain("Your message is saved and nothing was changed");
    expect(notice.content).not.toContain("reqwest");
    expect(notice.content).not.toContain("provider_network_error");
  });

  it("maps an interrupted provider stream to calm network guidance", () => {
    const notice = chatFailureNotice({
      code: "provider_stream_interrupted_after_tokens",
      message:
        "The remote provider connection closed before the response finished after OOMU received 142 stream events.",
    });

    expect(notice.status).toBe("Provider temporarily unavailable");
    expect(notice.content).toContain("after several attempts");
    expect(notice.content).not.toContain("provider_stream_interrupted_after_tokens");
    expect(notice.content).not.toContain("error decoding response body");
    expect(notice.content).not.toContain("token(s)");
  });

  it("keeps exhausted same-provider stream retries in the network guidance", () => {
    const notice = chatFailureNotice({
      code: "inference_retry_exhausted",
      message:
        "Transient inference failed after 3 attempts. Final error code=provider_stream_interrupted_after_tokens message=The remote provider connection closed before the response finished.",
    });

    expect(notice.status).toBe("Provider temporarily unavailable");
    expect(notice.content).toContain("after several attempts");
    expect(notice.content).not.toContain("inference_retry_exhausted");
  });

  it("maps provider rate limits to retry guidance without raw provider URLs", () => {
    const notice = chatFailureNotice({
      code: "provider_rate_limited",
      message: "The remote provider rate limit was reached (HTTP 429 Too Many Requests).",
    });

    expect(notice.status).toBe("Provider rate limited");
    expect(notice.content).toContain(
      "The provider is limiting requests right now. Wait a moment, then try again, or choose another route.",
    );
    expect(notice.content).not.toContain("HTTP 429");
    expect(notice.content).not.toContain("provider_rate_limited");
  });

  it("keeps local-only Project policy understandable without backend details", () => {
    const translations: Record<string, string> = {
      "chat.errors.project_provider_blocked.status": "This Project stays on your Mac",
      "chat.errors.project_provider_blocked.content":
        "This Project is set to local only. Choose a local model, or change its privacy setting in Projects. Nothing was sent.",
    };
    const notice = chatFailureNotice(
      {
        code: "project_provider_blocked",
        message: "internal destination_origin=prov-private-canary",
      },
      (key) => translations[key] ?? key,
    );

    expect(notice.status).toBe("This Project stays on your Mac");
    expect(notice.content).toContain("Choose a local model");
    expect(notice.content).not.toContain("prov-private-canary");
    expect(notice.content).not.toContain("project_provider_blocked");
  });

  it("turns an out-of-band Project consent error into a safe prompt reminder", () => {
    const translations: Record<string, string> = {
      "chat.errors.project_provider_consent.status": "Cloud approval needed",
      "chat.errors.project_provider_consent.content":
        "Review the Project cloud prompt to continue. Nothing was sent.",
    };
    const notice = chatFailureNotice(
      {
        code: "project_provider_confirmation_invalid",
        message: "challenge=private-canary",
      },
      (key) => translations[key] ?? key,
    );

    expect(notice.status).toBe("Cloud approval needed");
    expect(notice.content).not.toContain("private-canary");
  });

  it("keeps exhausted provider-response details out of the conversation", () => {
    const notice = chatFailureNotice({
      code: "inference_retry_exhausted",
      message:
        "Transient inference failed after 3 attempts. Final error code=provider_response_error message=raw-provider-body-canary",
    });

    expect(notice.status).toBe("Provider response issue");
    expect(notice.content).toContain("unusable response");
    expect(notice.content).not.toContain("raw-provider-body-canary");
    expect(notice.content).not.toContain("provider_response_error");
    expect(notice.content).not.toContain("inference_retry_exhausted");
  });

  it.each([
    "planner_connector_binding_mismatch",
    "connector_planned_account_reconnect_required",
    "connector_planned_project_authorization_required",
    "connector_planned_capability_consent_required",
  ])("turns %s into clear connection guidance without internal details", (code) => {
    const notice = chatFailureNotice({
      code,
      message: `backend ${code} credential_ref=private-canary`,
    });

    expect(notice.status).toBe("Check this connection");
    expect(notice.content).toContain("Open Connections");
    expect(notice.content).toContain("Nothing was sent or changed");
    expect(notice.content).not.toContain(code);
    expect(notice.content).not.toContain("credential_ref");
    expect(notice.content).not.toContain("private-canary");
  });

  it("localizes contextual and external write failures without exposing backend prose", () => {
    const contextual = chatFailureNotice({
      code: "contextual_output_name_invalid",
      message: "OOMU couldn't identify one requested output filename safely.",
    });
    expect(contextual.status).toBe("Choose a clear file destination");
    expect(contextual.content).toContain("inside your Project folder");
    expect(contextual.content).not.toContain("contextual_output_name_invalid");

    const external = chatFailureNotice({
      code: "agent_execution_failed",
      message: "The approved external file write failed. The original file was not changed.",
    });
    expect(external.status).toBe("File change paused safely");
    expect(external.content).toContain("original or newer file was kept");
    expect(external.content).not.toContain("approved external file write failed");
    expect(external.content).not.toContain("agent_execution_failed");
  });

  it("asks only for the missing Markdown filename without validator jargon", () => {
    const notice = chatFailureNotice({
      code: "contextual_filename_required",
      message: "internal planner detail",
    });

    expect(notice).toEqual({
      status: "Ready",
      content: "What should I name the Markdown file?",
    });
  });

  it("keeps local permission denials and prompt failures calm and hides backend detail", () => {
    expect(localCommandFailureText({
      code: "shield_approval_denied",
      message: "MCP tool was not approved: RAW_DENIAL_DETAIL",
    })).toBe("Permission wasn’t granted. Nothing was changed.");
    expect(localCommandFailureText({
      code: "shield_approval_timeout",
      message: "RAW_TIMEOUT_DETAIL",
    })).toBe("OOMU couldn’t ask for permission. Nothing was changed. Try again.");
    expect(localCommandFailureText(new Error("Disk is full."))).toBe(
      "Local command failed. Disk is full.",
    );
  });

  it("maps identity ledger failures to calm secure-memory guidance", () => {
    const notice = chatFailureNotice({
      code: "ledger_integrity_violation",
      message:
        "Ledger Integrity Violation: signature public key does not match local Sovereign Identity.",
    });

    expect(notice.status).toBe("Secure memory paused");
    expect(notice.content).toContain("You can keep chatting");
    expect(notice.content).toContain("Device Identity");
    expect(notice.content).not.toContain("signature public key");
    expect(notice.content).not.toContain("ledger_integrity_violation");
  });

  it("parses prescriptive vertical templates into operation panel sections", () => {
    const parsed = parseVerticalTemplatePayload([
      "### CLIENT PROFILE STATE",
      "*   State: Jordan is frustrated.",
      "*   Issues: Duplicate invoice.",
      "",
      "### RECOMMENDED RESOLUTION PATHS",
      "1. Check the billing RAG article.",
      "2. Verify account status.",
      "",
      "### EXPERIENCE ENHANCEMENT CHECKS",
      "*   Calibrated Tone: Start with acknowledgment.",
      "*   Pitfalls to Avoid: No refund promises before verification.",
    ].join("\n"));

    expect(parsed.isVertical).toBe(true);
    expect(parsed.completionRatio).toBe(1);
    expect(parsed.sections[0].content).toContain("Jordan is frustrated");
    expect(parsed.sections[1].content).toContain("billing RAG article");
    expect(parsed.sections[2].content).toContain("No refund promises");

    const fenced = parseVerticalTemplatePayload([
      "```markdown",
      "###CLIENT PROFILE STATE",
      "* State: Confused",
      "```",
    ].join("\n"));

    expect(fenced.isVertical).toBe(true);
    expect(fenced.sections[0].content).toContain("Confused");
  });

  it("routes the newest tolerant vertical assistant payload", () => {
    const route = latestVerticalTemplateRoute([
      { id: 1, role: "assistant", content: "Ordinary answer." },
      {
        id: 2,
        role: "assistant",
        content: [
          "CLIENT PROFILE STATE",
          "* State: Confused",
          "",
          "### RECOMMENDED RESOLUTION PATHS",
          "1. Verify the account.",
        ].join("\n"),
      },
    ]);

    expect(route?.messageId).toBe(2);
    expect(route?.parsed.isVertical).toBe(true);
    expect(route?.parsed.missingSections).toEqual(["Experience Checks"]);
  });

  it("parses browser split-view navigation directives", () => {
    const payload = [
      "<OomuSplitView>",
      "<mod_id>ai.eldris.mods.browser</mod_id>",
      "<action>NAVIGATE</action>",
      "<url>https://www.google.com</url>",
      "<reason>User requested to open google.com using browser capabilities.</reason>",
      "</OomuSplitView>",
    ].join(" ");

    const parsed = parseBrowserSplitViewPayload(payload);
    expect(parsed?.modId).toBe("ai.eldris.mods.browser");
    expect(parsed?.action).toBe("NAVIGATE");
    expect(parsed?.url).toBe("https://www.google.com/");

    const route = latestBrowserSplitRoute([
      { id: 1, role: "assistant", content: "Ordinary response." },
      { id: 2, role: "user", content: "Open google.com in the browser." },
      { id: 3, role: "assistant", content: payload },
    ]);
    expect(route?.messageId).toBe(3);
    expect(route?.url).toBe("https://www.google.com/");
  });

  it("detects browser navigation failure feedback without inventing a search fallback", () => {
    const feedback = "I don't think the Red Sox schedule is on the browser page you just opened.";
    const route = {
      messageId: 2,
      sessionId: "session-1",
      modId: "ai.eldris.mods.browser",
      action: "NAVIGATE",
      url: "https://www.mlb.com/",
      reason: "Opening requested URL in the live browser panel.",
      rawDirective: "",
    };

    expect(browserFeedbackIndicatesFailedNavigation(feedback)).toBe(true);
    expect(
      browserSearchFallbackQuery(
        feedback,
        [
          { id: 1, role: "user", content: "Check the Red Sox schedule today." },
          { id: 2, role: "assistant", content: "Opening MLB." },
        ],
        route,
      ),
    ).toBe("");
    expect(browserNavigationBlockPayload(route.url)).toEqual({
      status: "navigation_blocked",
      reason:
        "The user marked this URL incorrect. Do not reopen it or search automatically. Ask for an explicit destination, or ask the user to enable Search and request an external search.",
      url: "https://www.mlb.com/",
      suggested_action: "request_explicit_search",
    });
    const notice = browserNavigationBlockedNotice((key) => ({
      "chat.browser.navigation_blocked": "That page was marked incorrect. OOMU won't reopen it or search automatically.",
      "chat.browser.navigation_blocked_status": "Page marked incorrect.",
    })[key] ?? key);
    expect(notice).toEqual({ message: "That page was marked incorrect. OOMU won't reopen it or search automatically.", status: "Page marked incorrect." });
    expect(notice.message).not.toMatch(/^\s*[{[]/);
  });

  it("formats payload-size preflight bypass telemetry for chat disclosure", () => {
    const notice = oomuBypassNotice({
      kind: "bypassed",
      reason: "payload_size",
      estimatedTokens: 150_000,
      localContextMaxTokens: 6_000,
      providerId: "gemini",
      modelId: "gemini-3.5-flash",
      securityDisclaimer: "Local security preflight did not complete.",
      occurredAtMs: 1,
    });

    expect(notice.title).toBe("Security preflight");
    expect(notice.body).toContain("Bypassed local check due to payload size (150K tokens");
    expect(notice.body).toContain("Routed directly to gemini-3.5-flash");
    expect(notice.detail).toContain("Local security preflight did not complete");
  });

  it("formats timed-out preflight telemetry as degraded execution", () => {
    const notice = oomuBypassNotice({
      kind: "timeout",
      reason: "preflight_timeout",
      estimatedTokens: 5_500,
      localContextMaxTokens: 6_000,
      providerId: "gemini",
      modelId: "gemini-3.5-flash",
      occurredAtMs: 1,
    });

    expect(notice.body).toContain("Preflight timeout: Degraded execution");
    expect(notice.body).toContain("Routed directly to gemini-3.5-flash");
    expect(notice.detail).toContain("remote execution continued");
  });

  it("parses logical certificates out of the main chat response", () => {
    const parsed = parseLogicalCertificate(
      "Done.\n\n---\nPremises:\nEvidence.\nExecution Path:\nChecked it.\nFormal Conclusion:\nComplete.",
    );

    expect(parsed.mainContent).toBe("Done.");
    expect(parsed.certificate).toContain("Premises:");
    expect(parsed.certificate).toContain("Formal Conclusion:");
  });

  it("parses logical certificates that use conclusion without formal", () => {
    const parsed = parseLogicalCertificate(
      "Done.\n\nLogical Certificate:\nPremises:\nEvidence.\nExecution Path:\nChecked it.\nConclusion:\nComplete.",
    );

    expect(parsed.mainContent).toBe("Done.");
    expect(parsed.certificate).toContain("Formal Conclusion:");
  });

  it("normalizes malformed logical certificate sections to canonical headers", () => {
    const parsed = parseLogicalCertificate(
      [
        "It feels excellent.",
        "",
        "***",
        "",
        "### Logical Certificate1. **Premises:**",
        "   * Runtime changed.",
        "",
        "2. **Execution Path:*** Checked the route.",
        "",
        "3. **Conclusion:**",
        "   * Complete.",
        "",
        "4. **State:** Active.",
      ].join("\n"),
    );

    expect(parsed.mainContent).toBe("It feels excellent.");
    expect(parsed.certificate).toBe(
      [
        "---",
        "Premises:",
        "* Runtime changed.",
        "",
        "Execution Path:",
        "Checked the route.",
        "",
        "Formal Conclusion:",
        "* Complete.",
      ].join("\n"),
    );
    expect(normalizeLogicalCertificate(parsed.certificate ?? "")).toBe(parsed.certificate);
  });

  it("parses bullet-prefixed logical certificate sections", () => {
    const parsed = parseLogicalCertificate(
      [
        "Done.",
        "",
        "---",
        "\u2022 Premises: Evidence was checked.",
        "\u2022 Execution Path: The renderer scanned the sections.",
        "\u2022 Formal Conclusion: The certificate is canonical.",
      ].join("\n"),
    );

    expect(parsed.mainContent).toBe("Done.");
    expect(parsed.certificate).toBe(
      [
        "---",
        "Premises:",
        "Evidence was checked.",
        "",
        "Execution Path:",
        "The renderer scanned the sections.",
        "",
        "Formal Conclusion:",
        "The certificate is canonical.",
      ].join("\n"),
    );
  });

  it("expands local context steps and warning tones for high-end hardware", () => {
    const bounds = contextBudgetBoundsForProvider(configuredProviders, "provider-1", {
      physicalMemoryGb: 64,
      processorTier: "High (Metal, 32K local context)",
      cpuArch: "aarch64",
      cpuCores: 12,
      osName: "macos",
      metalSupported: true,
      maxLocalContextBudget: 32_768,
    });

    expect(bounds.steps).toEqual([4096, 8192, 12_288, 16_384, 20_480, 24_576, 28_672, 32_768]);
    expect(bounds.max).toBe(32_768);
    expect(contextBudgetToneForValue(bounds, 16_384)).toBe("emerald");
    expect(contextBudgetToneForValue(bounds, 24_576)).toBe("amber");
    expect(contextBudgetToneForValue(bounds, 32_768)).toBe("rose");
  });
  it.each([
    ["permission_denied", "Permission not granted", "Permission wasn’t granted. Nothing was changed."],
    ["shield_approval_denied", "Permission not granted", "Permission wasn’t granted. Nothing was changed."],
    ["permission_request_failed", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["permission_prompt_unavailable", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["permission_check_failed", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["shield_approval_not_found", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["shield_approval_event_failed", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["shield_approval_channel_closed", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
    ["shield_approval_timeout", "Couldn’t ask for permission", "OOMU couldn’t ask for permission. Nothing was changed. Try again."],
  ])("maps %s to fixed permission guidance without raw detail", (code, status, content) => {
    const notice = chatFailureNotice({ code, message: "RAW_PERMISSION_BACKEND_DETAIL" });
    expect(notice).toEqual({ status, content });
    expect(notice.content).not.toContain(code);
    expect(notice.content).not.toContain("RAW_PERMISSION_BACKEND_DETAIL");
  });
});
