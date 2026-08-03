(() => {
  "use strict";

  if (window.__oomuScenarioOneE2eDriver) return;
  window.__oomuScenarioOneE2eDriver = true;

  const TRACE_PREFIX = "OOMU_SCENARIO_ONE_E2E_TRACE";
  const POLL_MS = 250;
  const RECOVERY_CARD_RENDER_TIMEOUT_MS = 120_000;
  const PERMISSION_RECOVERY_TIMEOUT_MS = 600_000;
  const PERMISSION_RECOVERY_RETRY_INTERVAL_MS = 5_000;
  const CALENDAR_FULL_ACCESS_RECOVERY_CODES = new Set([
    "calendar_authorization_timeout",
    "calendar_permission_denied",
    "calendar_permission_restricted",
    "calendar_permission_unavailable",
    "calendar_permission_write_only",
  ]);
  const OUTPUT_DIRECTORY = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";
  const INPUT_PATHS = [
    "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json",
    "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt",
  ];
  const CALENDAR_NAME = "OOMU Test";
  const EVENT_TITLE = "Supplier Decision Review";
  const MAIL_RECIPIENT = "recipient@example.com";
  const REQUIRED_LOCAL_MODEL = "gemma-4-E4B-it-qat-q4_0-gguf";
  const PROMPT = [
    "prepare a board-ready supplier decision pack.",
    "Read /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json and /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt from my testing folder.",
    "Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation.",
    "Cite every web claim with its URL and access time.",
    "Create a new ship_test_01 folder in the testing folder and deliver four real files: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md.",
    "The workbook must contain source data, formulas, exception flags, and a recommendation sheet.",
    "The presentation and PDF must be executive-ready and mutually consistent.",
    "Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to recipient@example.com summarizing the recommendation and listing the four output files.",
    "Do not send the email.",
    "Ask for any required approvals and continue from the exact stopped step after I approve.",
    "Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.",
  ].join(" ");

  const startedAt = Date.now();
  const stateRoot = document.documentElement;

  function trace(stage, status, detail = "") {
    const elapsedMs = Date.now() - startedAt;
    const suffix = detail ? ` detail=${detail}` : "";
    const line = `${TRACE_PREFIX} stage=${stage} status=${status} elapsed_ms=${elapsedMs}${suffix}`;
    stateRoot.dataset.oomuScenarioOneE2eState = `${stage}:${status}`;
    document.title = `OOMU E2E — ${stage}:${status}`;
    let badge = document.querySelector("[data-oomu-scenario-one-e2e-trace]");
    if (!(badge instanceof HTMLElement)) {
      badge = document.createElement("div");
      badge.dataset.oomuScenarioOneE2eTrace = "true";
      Object.assign(badge.style, {
        position: "fixed",
        right: "12px",
        bottom: "12px",
        zIndex: "2147483647",
        maxWidth: "min(720px, calc(100vw - 24px))",
        borderRadius: "8px",
        background: "rgba(17, 24, 39, 0.94)",
        color: "#f9fafb",
        font: "12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace",
        padding: "8px 10px",
        pointerEvents: "none",
      });
      document.body.appendChild(badge);
    }
    badge.textContent = detail ? `${stage}:${status} · ${detail}` : `${stage}:${status}`;
    window.dispatchEvent(new CustomEvent("oomu:scenario-one-e2e-trace", {
      detail: { stage, status, elapsedMs },
    }));
    if (status === "failed" || status === "rejected_scope" || status === "timeout") {
      console.error(line);
    } else {
      console.info(line);
    }
  }

  const delay = (milliseconds) => new Promise((resolve) => window.setTimeout(resolve, milliseconds));
  const normalizedText = (element) => (element?.innerText || "").replace(/\s+/g, " ").trim();
  const isVisible = (element) => {
    if (!(element instanceof HTMLElement) || !element.isConnected) return false;
    const style = window.getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const visible = (selector, root = document) =>
    Array.from(root.querySelectorAll(selector)).filter(isVisible);

  async function waitFor(stage, predicate, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const value = predicate();
      if (value) return value;
      await delay(POLL_MS);
    }
    throw new Error(`${stage}_timeout_${timeoutMs}ms`);
  }

  function clickControl(stage, control) {
    if (!(control instanceof HTMLButtonElement) || !isVisible(control) || control.disabled) {
      throw new Error(`${stage}_control_not_actionable`);
    }
    control.focus({ preventScroll: true });
    control.click();
    trace(stage, "clicked");
  }

  function setComposerValue(textarea, value) {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    if (!setter) throw new Error("composer_native_value_setter_missing");
    setter.call(textarea, value);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
  }

  function planHasExactScenarioScope(button) {
    const plan = button.closest("section");
    if (!plan) return false;
    const text = normalizedText(plan);
    return text.includes("ship_test_01")
      && ["supplier_decision.xlsx", "supplier_decision.pptx", "supplier_decision.pdf", "sources.md"]
        .every((output) => text.includes(output))
      && text.includes(CALENDAR_NAME)
      && text.includes(EVENT_TITLE)
      && text.includes(MAIL_RECIPIENT);
  }

  function exactCalendarRecoveryControl(executionId) {
    if (!executionId) return null;
    const cards = visible(
      '[data-oomu-calendar-recovery-code="calendar_not_found"]'
        + '[data-oomu-calendar-recovery-action="resolve_calendar_target"]'
        + '[data-calendar-recovery-requested="OOMU Test"]',
    ).filter((candidate) => candidate.dataset.oomuRecoveryExecutionId === executionId);
    if (cards.length !== 1) return null;
    const controls = visible(
      'button[data-calendar-recovery-action="create_requested"]'
        + '[data-oomu-calendar-recovery="create-requested"]'
        + '[data-oomu-calendar-name="OOMU Test"]',
      cards[0],
    ).filter((button) => !button.disabled);
    return controls.length === 1 ? controls[0] : null;
  }

  function exactPermissionRecoveryCard(executionId) {
    if (!executionId) return null;
    const candidates = [];
    for (const card of visible(
      'section[data-oomu-calendar-recovery-action="restore_calendar_full_access"]'
        + "[data-oomu-calendar-recovery-code]",
    ).filter((candidate) => candidate.dataset.oomuRecoveryExecutionId === executionId)) {
      const code = card.dataset.oomuCalendarRecoveryCode || "";
      if (!CALENDAR_FULL_ACCESS_RECOVERY_CODES.has(code)) continue;
      const controls = visible(
        'button[data-calendar-permission-action="check_and_continue"]',
        card,
      );
      if (controls.length === 1) {
        candidates.push({ card, code, control: controls[0], kind: "calendar_full_access" });
      }
    }
    for (const card of visible(
      'section[data-oomu-mail-recovery-action="restore_mail_automation_access"]'
        + '[data-oomu-mail-recovery-code="mail_automation_permission_required"]',
    ).filter((candidate) => candidate.dataset.oomuRecoveryExecutionId === executionId)) {
      const controls = visible(
        'button[data-mail-automation-action="check_and_continue"]',
        card,
      );
      if (controls.length === 1) {
        candidates.push({
          card,
          code: "mail_automation_permission_required",
          control: controls[0],
          kind: "mail_automation",
        });
      }
    }
    return candidates.length === 1 ? candidates[0] : null;
  }

  async function waitForPermissionGrantAndResume(executionId, expectedKind) {
    const deadline = Date.now() + PERMISSION_RECOVERY_TIMEOUT_MS;
    const stage = expectedKind === "calendar_full_access"
      ? "calendar_permission_recovery"
      : "mail_automation_recovery";
    let attempt = 0;
    let nextCheckAt = 0;
    let pendingAttemptTraced = 0;
    trace(
      stage,
      "waiting_for_user_grant",
      `execution=${executionId} bounded_ms=${PERMISSION_RECOVERY_TIMEOUT_MS}`,
    );

    while (Date.now() < deadline) {
      const execution = visible("[data-agent-execution-status]").find((candidate) =>
        candidate.dataset.agentExecutionId === executionId
      );
      const status = execution?.dataset.agentExecutionStatus || "";
      if (status === "running" || status === "completed") {
        trace(
          stage,
          "same_execution_resumed",
          `execution=${executionId} status=${status} checks=${attempt}`,
        );
        return;
      }
      if (status === "failed") {
        trace(stage, "failed", `execution=${executionId} status=failed checks=${attempt}`);
        throw new Error(`${stage}_same_execution_failed`);
      }

      const recovery = exactPermissionRecoveryCard(executionId);
      if (recovery && recovery.kind !== expectedKind) {
        trace(
          stage,
          "failed",
          `execution=${executionId} unexpected_recovery=${recovery.kind}`,
        );
        throw new Error(`${stage}_changed_before_resume`);
      }
      if (
        recovery
        && recovery.control instanceof HTMLButtonElement
        && !recovery.control.disabled
        && Date.now() >= nextCheckAt
      ) {
        attempt += 1;
        clickControl(`${stage}_check`, recovery.control);
        trace(
          stage,
          "read_only_check_requested",
          `execution=${executionId} code=${recovery.code} check=${attempt}`,
        );
        nextCheckAt = Date.now() + PERMISSION_RECOVERY_RETRY_INTERVAL_MS;
        pendingAttemptTraced = 0;
      } else if (
        recovery
        && recovery.control instanceof HTMLButtonElement
        && !recovery.control.disabled
        && attempt > 0
        && pendingAttemptTraced !== attempt
      ) {
        pendingAttemptTraced = attempt;
        trace(
          stage,
          "grant_still_pending",
          `execution=${executionId} code=${recovery.code} check=${attempt}`,
        );
      }
      await delay(POLL_MS);
    }

    trace(
      stage,
      "timeout",
      `execution=${executionId} bounded_ms=${PERMISSION_RECOVERY_TIMEOUT_MS} checks=${attempt}`,
    );
    throw new Error(`${stage}_user_grant_timeout`);
  }

  async function revealDialogDetails(dialog) {
    for (const details of Array.from(dialog.querySelectorAll("details"))) {
      if (!details.open) {
        const summary = details.querySelector("summary");
        if (summary instanceof HTMLElement && isVisible(summary)) summary.click();
      }
    }
    await delay(100);
  }

  function shieldScopeKind(dialog) {
    const detailValues = visible("[data-oomu-approval-detail]", dialog).map(normalizedText);
    const pathValues = detailValues.filter((value) => value.startsWith("/Users/"));
    const exactOutputPath = pathValues.some((path) =>
      path === OUTPUT_DIRECTORY || path.startsWith(`${OUTPUT_DIRECTORY}/`)
    );
    const exactScenarioInput = pathValues.some((path) => INPUT_PATHS.includes(path));
    const exactCalendar = detailValues.includes(CALENDAR_NAME) && detailValues.includes(EVENT_TITLE);
    const exactMail = detailValues.includes(MAIL_RECIPIENT) && detailValues.includes(EVENT_TITLE);
    const unexpectedAbsolutePath = pathValues.some((path) =>
      path !== OUTPUT_DIRECTORY
      && !path.startsWith(`${OUTPUT_DIRECTORY}/`)
      && !INPUT_PATHS.includes(path)
    );
    const ambiguousAbsolutePath = detailValues.some((value) =>
      value.includes("/Users/") && !pathValues.includes(value)
    );
    const emailAddresses = detailValues.join("\n").match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi) || [];
    const unexpectedEmail = emailAddresses.some((address) => address.toLowerCase() !== MAIL_RECIPIENT);

    if (unexpectedAbsolutePath || ambiguousAbsolutePath || unexpectedEmail) return null;
    if (emailAddresses.length > 0) return exactMail ? "scenario_mail_draft" : null;
    if (exactCalendar) return "oomu_test_calendar";
    if (exactOutputPath) return "ship_test_01";
    if (exactScenarioInput) return "scenario_input";
    return null;
  }

  async function runApprovalMonitor(timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    let approvalCount = 0;
    let executionId = "";
    let lastExecutionStatus = "";
    let lastProgressAt = 0;
    let sawRunning = false;
    let calendarRecoveryHandled = false;
    trace("shield_monitor", "started");
    while (Date.now() < deadline) {
      const dialog = visible('[role="dialog"][aria-modal="true"]')[0];
      if (!dialog) {
        const progress = visible("[data-agent-execution-status]")[0];
        if (progress) {
          const status = progress.dataset.agentExecutionStatus || "unknown";
          executionId = progress.dataset.agentExecutionId || executionId;
          lastProgressAt = Date.now();
          sawRunning ||= status === "running";
          if (status !== lastExecutionStatus) {
            trace("execution", "rendered_status", `status=${status} execution=${executionId || "unknown"}`);
            lastExecutionStatus = status;
          }
          if (status === "completed") {
            trace("driver", "rendered_terminal", `status=completed execution=${executionId || "unknown"}`);
            return;
          }
          if (status === "halted") {
            let recovery = null;
            try {
              recovery = await waitFor(
                "halted_recovery_card",
                () => {
                  const createCalendar = !calendarRecoveryHandled
                    ? exactCalendarRecoveryControl(executionId)
                    : null;
                  if (createCalendar) return { kind: "missing_calendar", control: createCalendar };
                  return exactPermissionRecoveryCard(executionId);
                },
                RECOVERY_CARD_RENDER_TIMEOUT_MS,
              );
            } catch {
              // Only exact Scenario 1 recovery cards are eligible. Every other
              // halted execution continues through the terminal branch below.
            }
            if (recovery?.kind === "missing_calendar") {
              clickControl("calendar_recovery_create", recovery.control);
              calendarRecoveryHandled = true;
              trace("calendar_recovery", "exact_create_requested", `execution=${executionId}`);
              await waitFor("calendar_recovery_resume", () => {
                const resumed = visible("[data-agent-execution-status]").find((candidate) =>
                  candidate.dataset.agentExecutionId === executionId
                  && ["running", "completed"].includes(candidate.dataset.agentExecutionStatus || "")
                );
                return resumed || null;
              }, 120_000);
              trace("calendar_recovery", "same_execution_resumed", `execution=${executionId}`);
              continue;
            }
            if (recovery?.kind === "calendar_full_access" || recovery?.kind === "mail_automation") {
              await waitForPermissionGrantAndResume(executionId, recovery.kind);
              continue;
            }
          }
          if (status === "failed" || status === "halted") {
            trace("driver", "rendered_terminal", `status=${status} execution=${executionId || "unknown"}`);
            throw new Error(`rendered_execution_${status}`);
          }
        } else if (sawRunning && Date.now() - lastProgressAt > 3_000) {
          const visibleAlert = visible('main [role="alert"]')[0];
          if (visibleAlert) {
            trace("driver", "rendered_terminal", `status=failed_visible_alert execution=${executionId || "unknown"}`);
            throw new Error("rendered_execution_failed_visible_alert");
          }
        }
        await delay(POLL_MS);
        continue;
      }

      const nativeStatusDismiss = visible(
        'button[data-oomu-native-approval-status="dismiss"]',
        dialog,
      ).find((button) => !button.disabled);
      if (nativeStatusDismiss) {
        clickControl("native_shield_status", nativeStatusDismiss);
        await delay(POLL_MS);
        continue;
      }
      const decisionControl = visible('button[data-action-state="idle"]', dialog)
        .find((button) => !button.disabled);
      if (!decisionControl) {
        // Backend-originated approvals render a read-only status overlay while
        // the genuine AppKit prompt owns the decision. The debug profile
        // activates that prompt's exact allowlisted button; this DOM driver
        // must not mistake the informational Hide control for a dead end.
        await delay(POLL_MS);
        continue;
      }
      await revealDialogDetails(dialog);
      const scopeKind = shieldScopeKind(dialog);
      if (!scopeKind) {
        trace("shield_approval", "rejected_scope", `approval_index=${approvalCount + 1}`);
        throw new Error("visible_shield_scope_did_not_match_scenario_one");
      }
      clickControl("shield_approval", decisionControl);
      approvalCount += 1;
      trace("shield_approval", "scope_approved", `scope=${scopeKind} approval_index=${approvalCount}`);
      await waitFor("shield_dialog_close", () => !isVisible(dialog), 30_000);
    }
    trace("shield_monitor", "timeout", `bounded_ms=${timeoutMs} approvals=${approvalCount}`);
    throw new Error("execution_terminal_timeout");
  }

  async function waitForPlanApproval(timeoutMs, localModelId) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const planButton = visible('button[data-oomu-plan-approval="execute"]')
        .find((button) => !button.disabled);
      if (planButton) return planButton;

      const localChoice = visible('button[data-auto-route-choice="local"]')
        .find((button) => !button.disabled);
      if (localChoice) {
        clickControl("auto_route_local_fallback", localChoice);
        trace("auto_route_local_fallback", "selected", `model=${localModelId}`);
        await waitFor(
          "auto_route_attention_close",
          () => !isVisible(localChoice),
          30_000,
        );
        continue;
      }

      const dialog = visible('[role="dialog"][aria-modal="true"]')[0];
      if (dialog) {
        const nativeStatusDismiss = visible(
          'button[data-oomu-native-approval-status="dismiss"]',
          dialog,
        ).find((button) => !button.disabled);
        if (nativeStatusDismiss) {
          clickControl("native_shield_status", nativeStatusDismiss);
          await delay(POLL_MS);
          continue;
        }
        const decisionControl = visible('button[data-action-state="idle"]', dialog)
          .find((button) => !button.disabled);
        if (!decisionControl) {
          await delay(POLL_MS);
          continue;
        }
        await revealDialogDetails(dialog);
        const scopeKind = shieldScopeKind(dialog);
        if (!scopeKind) {
          trace("planning_shield_approval", "rejected_scope");
          throw new Error("visible_planning_shield_scope_did_not_match_scenario_one");
        }
        clickControl("planning_shield_approval", decisionControl);
        trace("planning_shield_approval", "scope_approved", `scope=${scopeKind}`);
        await waitFor("planning_shield_dialog_close", () => !isVisible(dialog), 30_000);
        continue;
      }
      await delay(POLL_MS);
    }
    throw new Error(`plan_approval_timeout_${timeoutMs}ms`);
  }

  async function finishIsolatedSetupIfNeeded() {
    const initialSurface = await waitFor(
      "initial_surface",
      () => visible("[data-setup-journey]")[0]
        || visible('button[aria-label="New chat"]')[0],
      120_000,
    );
    const journey = initialSurface.matches("[data-setup-journey]")
      ? initialSurface
      : null;
    if (!journey) return;
    trace("setup", "isolated_profile_detected");
    for (const expectedStep of ["model", "permissions", "connectors"]) {
      const active = await waitFor(
        `setup_${expectedStep}`,
        () => visible(`[data-setup-journey][data-setup-step="${expectedStep}"]`)[0],
        120_000,
      );
      const next = await waitFor(
        `setup_${expectedStep}_continue`,
        () => visible('button[data-setup-action="continue"]', active)
          .find((button) => !button.disabled),
        120_000,
      );
      clickControl(`setup_${expectedStep}`, next);
    }
    const sample = await waitFor(
      "setup_sample",
      () => visible('[data-setup-journey][data-setup-step="sample"]')[0],
      120_000,
    );
    const skip = await waitFor(
      "setup_sample_skip",
      () => visible('button[data-setup-action="skip-sample"]', sample)
        .find((button) => !button.disabled),
      120_000,
    );
    clickControl("setup_sample", skip);
    await waitFor("setup_complete", () => visible("[data-setup-journey]").length === 0, 120_000);
    trace("setup", "completed_without_external_connections");
  }

  async function run() {
    trace("driver", "started");

    await finishIsolatedSetupIfNeeded();

    const newChat = await waitFor(
      "new_chat",
      () => visible('button[aria-label="New chat"]').find((button) => !button.disabled),
      120_000,
    );
    const previousSessionId = visible("form[data-chat-session-id]")[0]?.dataset.chatSessionId || "";
    clickControl("new_chat", newChat);

    const sessionForm = await waitFor(
      "new_chat_session",
      () => visible("form[data-chat-session-id]").find((candidate) => {
        const candidateId = candidate.dataset.chatSessionId || "";
        return candidateId && (!previousSessionId || candidateId !== previousSessionId);
      }),
      120_000,
    );
    const sessionId = sessionForm.dataset.chatSessionId;
    trace("new_chat", "session_ready", `previous=${previousSessionId || "none"} session=${sessionId}`);

    const searchToggle = await waitFor(
      "search_toggle",
      () => visible('button[aria-label="Search"][aria-pressed]', sessionForm)
        .find((button) => !button.disabled),
      120_000,
    );
    if (searchToggle.getAttribute("aria-pressed") !== "true") {
      clickControl("search_toggle", searchToggle);
      await waitFor(
        "search_toggle_on",
        () => searchToggle.getAttribute("aria-pressed") === "true" && !searchToggle.disabled,
        30_000,
      );
    }
    trace("search_toggle", "enabled");

    const autoRouteToggle = await waitFor(
      "auto_route_toggle",
      () => visible('button[aria-label="Auto-route"][aria-pressed]', sessionForm)
        .find((button) => !button.disabled),
      120_000,
    );
    if (autoRouteToggle.getAttribute("aria-pressed") !== "true") {
      clickControl("auto_route_toggle", autoRouteToggle);
      await waitFor(
        "auto_route_toggle_on",
        () => autoRouteToggle.getAttribute("aria-pressed") === "true" && !autoRouteToggle.disabled,
        30_000,
      );
    }
    trace("auto_route_toggle", "enabled");

    const routeIndicator = await waitFor("auto_route_ready", () => {
      const indicator = visible(
        '[data-route-mode="auto"][data-local-model-id][data-auto-route-status][data-cloud-model-id]',
        sessionForm,
      )[0];
      if (!indicator) return null;
      const status = indicator.dataset.autoRouteStatus || "unknown";
      if (status === "degraded" || status === "shutdown") {
        trace("auto_route_readiness", "failed", `status=${status}`);
        throw new Error(`auto_route_${status}`);
      }
      return status === "ready" ? indicator : null;
    }, 120_000);
    const cloudModelId = routeIndicator.dataset.cloudModelId || "";
    trace(
      "auto_route_readiness",
      "verified",
      `status=ready cloud_model=${cloudModelId || "not_available_in_isolated_profile"}`,
    );
    const localModelId = routeIndicator.dataset.localModelId || "";
    if (localModelId !== REQUIRED_LOCAL_MODEL) {
      trace("local_model", "failed", `expected=${REQUIRED_LOCAL_MODEL} actual=${localModelId || "missing"}`);
      throw new Error("scenario_one_requires_exact_e4b_local_model");
    }
    trace("local_model", "verified", `model=${localModelId}`);

    const composer = await waitFor(
      "composer",
      () => visible("textarea", sessionForm).find((textarea) => textarea instanceof HTMLTextAreaElement),
      120_000,
    );
    setComposerValue(composer, PROMPT);
    await waitFor("composer_value", () => composer.value === PROMPT, 10_000);
    trace("composer", "filled");

    const form = composer.closest("form");
    trace(
      "preconditions",
      "ready",
      `session=${sessionId || "unknown"} search=on auto_route=ready local_model=${localModelId} cloud_model=${cloudModelId}`,
    );
    const send = await waitFor(
      "composer_send",
      () => form && visible('button[type="submit"]', form).find((button) => !button.disabled),
      120_000,
    );
    clickControl("composer_send", send);
    await waitFor(
      "submission_acknowledged",
      () => visible("textarea")
        .find((textarea) => textarea instanceof HTMLTextAreaElement && textarea.value === ""),
      120_000,
    );
    trace("composer_send", "accepted_by_ui");

    const approvePlan = await waitForPlanApproval(900_000, localModelId);
    if (!planHasExactScenarioScope(approvePlan)) {
      trace("plan_approval", "rejected_scope");
      throw new Error("visible_plan_scope_did_not_match_scenario_one");
    }
    clickControl("plan_approval", approvePlan);
    trace("plan_approval", "exact_scope_approved");

    await runApprovalMonitor(2_700_000);
  }

  void run().catch((error) => {
    const message = error instanceof Error ? error.message : "unknown_error";
    trace("driver", "failed", message.replace(/\s+/g, "_").slice(0, 160));
  });
})();
