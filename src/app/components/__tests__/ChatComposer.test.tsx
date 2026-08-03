import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatComposer } from "../ChatComposer";

const invokeMock = vi.hoisted(() => vi.fn());
const voiceListeners = vi.hoisted(
  () => new Set<(event: { payload: Record<string, unknown> }) => void>(),
);
const nativeDragListeners = vi.hoisted(
  () => new Set<(event: { payload: Record<string, unknown> }) => void>(),
);
const eventListenMock = vi.hoisted(() => vi.fn(
  async (eventName: string, listener: (event: { payload: Record<string, unknown> }) => void) => {
    if (eventName === "oomu://voice-stream") {
      voiceListeners.add(listener);
      return () => {
        voiceListeners.delete(listener);
      };
    }
    if (eventName === "oomu://local-context-drag") {
      nativeDragListeners.add(listener);
      return () => {
        nativeDragListeners.delete(listener);
      };
    }
    return () => undefined;
  },
));

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (
    eventName: string,
    listener: (event: { payload: Record<string, unknown> }) => void,
  ) => eventListenMock(eventName, listener),
}));
function emitVoice(payload: Record<string, unknown>) {
  for (const listener of voiceListeners) {
    listener({ payload });
  }
}

function emitNativeDrag(payload: Record<string, unknown>) {
  for (const listener of nativeDragListeners) {
    listener({ payload });
  }
}

function ComposerHarness({
  activeStreamId = null,
  attachments = [],
  isSending = false,
  onAttachmentDrop = vi.fn(),
  onRemoveAttachment = vi.fn(),
  onSubmitMessage = vi.fn(async () => ({ accepted: true })),
}: {
  activeStreamId?: string | null;
  attachments?: Array<{ name: string; byte_count: number }>;
  isSending?: boolean;
  onAttachmentDrop?: () => void;
  onRemoveAttachment?: (index: number) => void;
  onSubmitMessage?: (message: string) => Promise<{ accepted: boolean }>;
} = {}) {
  const [draft, setDraft] = useState("");
  return (
    <I18nProvider>
      <ChatComposer
        activeStreamId={activeStreamId}
        attachments={attachments}
        automatedWebGroundingEnabled
        draft={draft}
        dynamicRoutingEnabled={false}
        hasRouteModel
        hasSelectedAgent
        isQueueExecuting={false}
        isReadingAttachments={false}
        isSavingDynamicRoutingOverride={false}
        isSavingWebGroundingOverride={false}
        isSendMenuOpen={false}
        isSending={isSending}
        localModelIsHydrating={false}
        onAttachmentDrop={onAttachmentDrop}
        onAttachmentRequest={vi.fn()}
        onCloseSendMenu={vi.fn()}
        onCompactSession={vi.fn(async () => undefined)}
        onDraftChange={setDraft}
        onDynamicRoutingToggle={vi.fn()}
        onExecuteQueuedMessages={vi.fn()}
        onQueueMessage={vi.fn(async () => ({ accepted: true }))}
        onRemoveAttachment={onRemoveAttachment}
        onSteerNow={vi.fn(async () => ({ accepted: true }))}
        onStopGeneration={vi.fn()}
        onSubmitMessage={onSubmitMessage}
        onToggleSendMenu={vi.fn()}
        onWebGroundingToggle={vi.fn()}
        queuedMessageCount={0}
        selectedAgentName="OOMU"
        sessionId="session-1"
      />
    </I18nProvider>
  );
}

describe("ChatComposer voice input", () => {
  beforeEach(() => {
    voiceListeners.clear();
    nativeDragListeners.clear();
    eventListenMock.mockClear();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") {
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
      if (command === "get_session_context_status") {
        return { estimatedPercentageUsed: 0, tokensTotal: 8_192 };
      }
      if (command === "start_voice_capture") {
        return { capture_id: "voice-1", active: true };
      }
      if (command === "stop_voice_capture") {
        return { capture_id: "voice-1", active: false };
      }
      return undefined;
    });
  });

  afterEach(() => cleanup());

  it("streams partial speech into the draft and commits the final text", async () => {
    render(<ComposerHarness />);

    fireEvent.click(screen.getByRole("button", { name: "Use voice input" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_voice_capture"));

    act(() => emitVoice({ capture_id: "voice-1", text: "Schedule lunch", is_final: false }));
    expect(screen.getByPlaceholderText("Message OOMU…")).toHaveValue("Schedule lunch");

    act(() =>
      emitVoice({
        capture_id: "voice-1",
        text: "Schedule lunch tomorrow.",
        is_final: true,
      }),
    );
    expect(screen.getByPlaceholderText("Message OOMU…")).toHaveValue(
      "Schedule lunch tomorrow.",
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("stop_voice_capture"));
  });

  it("stops listening as soon as the user types", async () => {
    render(<ComposerHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Use voice input" }));
    await screen.findByRole("button", { name: "Use voice input", pressed: true });

    fireEvent.change(screen.getByPlaceholderText("Message OOMU…"), {
      target: { value: "I will type this" },
    });

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("stop_voice_capture"));
    expect(screen.getByPlaceholderText("Message OOMU…")).toHaveValue("I will type this");
  });

  it("shows a short permission message instead of native error details", async () => {
    render(<ComposerHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Use voice input" }));
    await screen.findByRole("button", { name: "Use voice input", pressed: true });

    act(() =>
      emitVoice({
        capture_id: "voice-1",
        text: "",
        is_final: true,
        error_code: "microphone_permission_denied",
      }),
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Voice input is off. Allow OOMU to use the microphone and Speech Recognition in System Settings.",
    );
  });

  it("keeps web search as an icon-only control with an accessible tooltip", () => {
    render(<ComposerHarness />);
    const searchButton = screen.getByRole("button", { name: "Search" });
    expect(searchButton).toHaveAttribute(
      "title",
      "Let the assistant look things up on the web for current information.",
    );
    expect(searchButton).not.toHaveTextContent("Search");
    expect(screen.queryByText("Searching the web for current information before answering.")).not.toBeInTheDocument();
  });

  it("accepts the exact native Finder receipt only over the composer", async () => {
    const onAttachmentDrop = vi.fn();
    const view = render(<ComposerHarness onAttachmentDrop={onAttachmentDrop} />);
    const dropTarget = view.container.querySelector<HTMLElement>("[data-chat-drop-target]");
    expect(dropTarget).toBeInstanceOf(HTMLFormElement);
    vi.spyOn(dropTarget!, "getBoundingClientRect").mockReturnValue({
      bottom: 300,
      height: 200,
      left: 100,
      right: 700,
      top: 100,
      width: 600,
      x: 100,
      y: 100,
      toJSON: () => ({}),
    });
    await waitFor(() => expect(nativeDragListeners.size).toBe(1));

    act(() => emitNativeDrag({ type: "over", position: { x: 250, y: 180 } }));
    expect(screen.getByRole("status")).toHaveTextContent("Drop to attach");

    act(() => emitNativeDrag({
      type: "drop", dropId: "a".repeat(64),
      position: { x: 250, y: 180 },
    }));
    expect(onAttachmentDrop).toHaveBeenCalledWith("a".repeat(64));
    expect(screen.queryByText("Drop to attach")).not.toBeInTheDocument();

    act(() => emitNativeDrag({
      type: "drop", dropId: "b".repeat(64),
      position: { x: 20, y: 20 },
    }));
    expect(onAttachmentDrop).toHaveBeenCalledTimes(1);
  });

  it("contains a stale native unlisten rejection after the composer unmounts", async () => {
    let resolveListener: ((unlisten: () => void) => void) | null = null;
    eventListenMock.mockImplementation((eventName: string) => {
      if (eventName === "oomu://local-context-drag") {
        return new Promise((resolve) => { resolveListener = resolve; });
      }
      return Promise.resolve(() => undefined);
    });
    const view = render(<ComposerHarness />);

    view.unmount();
    act(() => resolveListener?.(
      (() => Promise.reject(new Error("listener_already_removed"))) as unknown as () => void,
    ));

    await act(async () => Promise.resolve());
    expect(eventListenMock).toHaveBeenCalledWith(
      "oomu://local-context-drag", expect.any(Function),
    );
  });
});

describe("ChatComposer accessibility identifiers", () => {
  afterEach(() => cleanup());

  it("keeps stable native hooks while preserving localized labels", () => {
    const view = render(<ComposerHarness />);
    expect(view.container.querySelector("#oomu-chat-composer")).toBe(screen.getByPlaceholderText("Message OOMU…"));
    expect(view.container.querySelector("#oomu-chat-search")).toBe(screen.getByRole("button", { name: "Search" }));
    expect(view.container.querySelector("#oomu-chat-send")).toBe(screen.getByRole("button", { name: "Send" }));

    view.rerender(<ComposerHarness activeStreamId="stream-1" isSending />);
    expect(view.container.querySelector("#oomu-chat-stop")).toBe(screen.getByRole("button", { name: "Stop" }));
    expect(view.container.querySelector("#oomu-chat-send")).toBeNull();
  });
});

describe("ChatComposer submission acceptance", () => {
  beforeEach(() => {
    voiceListeners.clear();
    nativeDragListeners.clear();
    eventListenMock.mockClear();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => command === "get_session_context_status"
      ? { estimatedPercentageUsed: 0, tokensTotal: 8_192 }
      : undefined);
  });

  afterEach(() => cleanup());

  it("keeps the exact draft until native acceptance resolves", async () => {
    let resolveAcceptance: ((value: { accepted: boolean }) => void) | null = null;
    const onSubmitMessage = vi.fn(() => new Promise<{ accepted: boolean }>((resolve) => {
      resolveAcceptance = resolve;
    }));
    render(<ComposerHarness onSubmitMessage={onSubmitMessage} />);
    const composer = screen.getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "  exact prompt  " } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(composer).toHaveValue("  exact prompt  ");
    expect(onSubmitMessage).toHaveBeenCalledTimes(1);
    act(() => resolveAcceptance?.({ accepted: true }));
    await waitFor(() => expect(composer).toHaveValue(""));
  });

  it("enables an attachment-only submission and preserves its canonical empty draft", async () => {
    const onSubmitMessage = vi.fn(async () => ({ accepted: true }));
    render(
      <ComposerHarness
        attachments={[{ name: "decision-brief.txt", byte_count: 42 }]}
        onSubmitMessage={onSubmitMessage}
      />,
    );

    const sendButton = screen.getByRole("button", { name: "Send" });
    expect(sendButton).toBeEnabled();
    fireEvent.click(sendButton);

    await waitFor(() => expect(onSubmitMessage).toHaveBeenCalledWith(""));
  });

  it("shows every attached file and removes only the file the user chose", () => {
    const onRemoveAttachment = vi.fn();
    render(<ComposerHarness
      attachments={[
        { name: "plan.json", byte_count: 42 },
        { name: "notes.md", byte_count: 88 },
        { name: "photo.png", byte_count: 120 },
      ]}
      onRemoveAttachment={onRemoveAttachment}
    />);

    expect(screen.getByText("plan.json")).toBeVisible();
    expect(screen.getByText("notes.md")).toBeVisible();
    expect(screen.getByText("photo.png")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Remove notes.md" }));
    expect(onRemoveAttachment).toHaveBeenCalledWith(1);
  });

  it("retains the draft when submission is not accepted or throws", async () => {
    const onSubmitMessage = vi.fn(async () => ({ accepted: false }));
    render(<ComposerHarness onSubmitMessage={onSubmitMessage} />);
    const composer = screen.getByPlaceholderText("Message OOMU…");
    fireEvent.change(composer, { target: { value: "keep this" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(onSubmitMessage).toHaveBeenCalledTimes(1));
    expect(composer).toHaveValue("keep this");
  });

  it("coalesces rapid double submission into one acceptance attempt", async () => {
    let resolveAcceptance: ((value: { accepted: boolean }) => void) | null = null;
    const onSubmitMessage = vi.fn(() => new Promise<{ accepted: boolean }>((resolve) => {
      resolveAcceptance = resolve;
    }));
    render(<ComposerHarness onSubmitMessage={onSubmitMessage} />);
    fireEvent.change(screen.getByPlaceholderText("Message OOMU…"), {
      target: { value: "send once" },
    });
    const send = screen.getByRole("button", { name: "Send" });
    fireEvent.click(send);
    fireEvent.click(send);

    expect(onSubmitMessage).toHaveBeenCalledTimes(1);
    act(() => resolveAcceptance?.({ accepted: true }));
    await waitFor(() => expect(screen.getByPlaceholderText("Message OOMU…")).toHaveValue(""));
  });
});
