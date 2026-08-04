import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.restoreAllMocks();
  vi.resetModules();
  vi.doUnmock("@tauri-apps/api/core");
  Reflect.deleteProperty(window, "__TAURI_IPC__");
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

describe("invoke provider and cancellation diagnostics", () => {
  it("preserves expected cancellation codes without console.error", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "local_inference_cancelled",
        message: "Remote generation was cancelled.",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("chat_turn")).rejects.toMatchObject({
      code: "local_inference_cancelled",
      message: "Remote generation was cancelled. (local_inference_cancelled)",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(debugSpy).toHaveBeenCalledWith(
      'Tauri invoke cancelled for command "chat_turn": Remote generation was cancelled. (local_inference_cancelled)',
    );
  });

  it("redacts sensitive provider URL query values before logging", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "provider_network_error",
        message:
          "HTTP status client error for url (https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent?key=secret-value&alt=sse)",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("chat_turn")).rejects.toMatchObject({
      code: "provider_network_error",
      detail: {
        message: expect.stringContaining("key=[redacted]&alt=sse"),
      },
      message: expect.not.stringContaining("secret-value"),
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("key=[redacted]&alt=sse"),
    );
    expect(warnSpy).not.toHaveBeenCalledWith(
      expect.stringContaining("secret-value"),
    );
  });

  it("logs provider rate limits as warnings instead of console errors", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "provider_rate_limited",
        message:
          "The remote provider rate limit was reached (HTTP 429 Too Many Requests).",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("chat_turn")).rejects.toMatchObject({
      code: "provider_rate_limited",
      message: expect.stringContaining("provider_rate_limited"),
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Tauri invoke warning for command "chat_turn"'),
    );
  });

  it("keeps recoverable model installer failures out of the dev error overlay", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "model_install_insufficient_storage",
        retryable: false,
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("start_recommended_model_install")).rejects.toMatchObject({
      code: "model_install_insufficient_storage",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining(
        'Tauri invoke warning for command "start_recommended_model_install"',
      ),
    );
  });

});

describe("invoke application update diagnostics", () => {
  it("keeps typed application-update failures out of the dev error overlay", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue("application_update_offer_expired"),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("install_pending_application_update")).rejects.toMatchObject({
      code: "application_update_offer_expired",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Tauri invoke warning for command "install_pending_application_update"'),
    );
  });
});

describe("invoke recoverable execution diagnostics", () => {
  it("reports a receipt-qualified search failure as an operational warning", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue(
        "Sovereign public search did not produce receipt-backed public evidence. (search_unavailable)",
      ),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("mcp_execute_tool")).rejects.toMatchObject({
      code: "search_unavailable",
      message: expect.stringContaining("receipt-backed public evidence"),
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      'Tauri invoke warning for command "mcp_execute_tool": Web search could not complete.',
    );
  });

  it("logs interrupted provider streams as warnings instead of fatal console errors", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "provider_stream_interrupted_after_tokens",
        message:
          "The remote provider connection closed before the response finished.",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("chat_turn")).rejects.toMatchObject({
      code: "provider_stream_interrupted_after_tokens",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Tauri invoke warning for command "chat_turn"'),
    );
  });

  it("logs an already-running response as a warning instead of opening the dev error overlay", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "chat_turn_already_running",
        message: "This message is already being answered.",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("chat_turn")).rejects.toMatchObject({
      code: "chat_turn_already_running",
      message: expect.stringContaining("chat_turn_already_running"),
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Tauri invoke warning for command "chat_turn"'),
    );
  });

  it("keeps planner diagnostics in a warning without opening the dev error overlay", async () => {
    const diagnostic =
      "Gateway response did not contain parseable ActionPlan JSON. Local planner prompt safety valve compressed 25122 characters into the 6000 character envelope before inference.";
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "planner_output_unusable",
        message: diagnostic,
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("process_agent_objective")).rejects.toMatchObject({
      code: "planner_output_unusable",
      message: expect.stringContaining("planner_output_unusable"),
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      'Tauri invoke warning for command "process_agent_objective": OOMU could not prepare a safe action plan.',
    );
    expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining("25122"));
    expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining("ActionPlan JSON"));
  });
});

describe("invoke planner control-flow diagnostics", () => {
  it.each([
    "planner_objective_too_large",
    "planner_prompt_compilation_failed",
  ])("keeps %s out of the dev error overlay", async (code) => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code,
        message: "Internal planner envelope diagnostic 6000/25122.",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("process_agent_objective")).rejects.toMatchObject({ code });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      'Tauri invoke warning for command "process_agent_objective": OOMU could not prepare a safe action plan.',
    );
    expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining("6000"));
    expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining(code));
  });

  it("treats a non-executable planner objective as quiet control flow", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "agent_objective_not_executable",
        message: "This objective does not require an ActionPlan.",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("process_agent_objective")).rejects.toMatchObject({
      code: "agent_objective_not_executable",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).not.toHaveBeenCalled();
    expect(debugSpy).toHaveBeenCalledWith(
      'Tauri invoke control flow for command "process_agent_objective": the request does not need an action plan.',
    );
    expect(debugSpy).not.toHaveBeenCalledWith(expect.stringContaining("ActionPlan"));
    expect(debugSpy).not.toHaveBeenCalledWith(
      expect.stringContaining("agent_objective_not_executable"),
    );
  });
});

describe("invoke setup recovery and native runtime boundary", () => {
  it("keeps the recoverable setup storage boundary out of the dev error overlay", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({
        code: "setup_storage_recovery_required",
        message: "volatile private database path and low-level details",
      }),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("run_setup_sample_task")).rejects.toMatchObject({
      code: "setup_storage_recovery_required",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      'Tauri invoke warning for command "run_setup_sample_task": OOMU must finish secure storage recovery before setup can continue.',
    );
    expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining("private database"));
  });

  it("fails closed outside native runtime without calling a loopback bridge", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const { invoke } = await import("../invoke");
    await expect(invoke("read_local_context", {
      request: {
        grantId: "a".repeat(64),
        sessionId: "session",
        turnId: "turn",
      },
    }))
      .rejects.toMatchObject({
      code: "native_runtime_required",
      message: expect.stringContaining("requires the native desktop runtime"),
    });
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

describe("invoke external browser diagnostics", () => {
  it("keeps a handled browser-open failure out of the dev error overlay", async () => {
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue("external_url_open_failed"),
    }));
    Object.defineProperty(window, "__TAURI_IPC__", {
      configurable: true,
      value: vi.fn(),
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const { invoke } = await import("../invoke");

    await expect(invoke("open_external_http_url")).rejects.toMatchObject({
      code: "external_url_open_failed",
    });
    expect(errorSpy).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Tauri invoke warning for command "open_external_http_url"'),
    );
  });
});
