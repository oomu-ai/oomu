use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::{process::CommandChild, process::CommandEvent, ShellExt};

const SPEECH_SIDECAR_NAME: &str = "oomu-speech-bridge";
const VOICE_STREAM_EVENT: &str = "oomu://voice-stream";
const MAX_TRANSCRIPT_CHARACTERS: usize = 16_000;
const MAX_PENDING_CAPTURES: usize = 8;
const PENDING_CAPTURE_LIFETIME_MS: i64 = 5 * 60 * 1_000;
static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VoiceCaptureStatus {
    pub capture_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VoiceCaptureError {
    pub code: &'static str,
    pub message: String,
}

impl VoiceCaptureError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct SpeechBridgeOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    is_final: bool,
    #[serde(default)]
    error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct VoiceStreamEvent {
    capture_id: String,
    text: String,
    is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

struct ActiveCapture {
    capture_id: String,
    child: CommandChild,
    transcript: String,
    final_seen: bool,
}

#[derive(Debug)]
pub(crate) struct PendingVoiceCapture {
    pub(crate) capture_id: String,
    pub(crate) transcript: String,
    pub(crate) final_seen: bool,
    completed_at_ms: i64,
}

#[derive(Default)]
struct VoiceCaptureState {
    active: Option<ActiveCapture>,
    pending: VecDeque<PendingVoiceCapture>,
}

#[derive(Clone, Default)]
pub struct VoiceCaptureManager {
    state: Arc<Mutex<VoiceCaptureState>>,
}

impl VoiceCaptureManager {
    fn start(&self, app: &AppHandle) -> Result<VoiceCaptureStatus, VoiceCaptureError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = app;
            return Err(VoiceCaptureError::new(
                "voice_input_unavailable",
                "Voice input is available only in OOMU for macOS.",
            ));
        }

        #[cfg(target_os = "macos")]
        {
            let mut state = self.state.lock().map_err(|_| {
                VoiceCaptureError::new(
                    "voice_input_state_unavailable",
                    "Voice input could not start. Try again.",
                )
            })?;
            if state.active.is_some() {
                return Err(VoiceCaptureError::new(
                    "voice_input_already_active",
                    "Voice input is already listening.",
                ));
            }

            let capture_id = next_capture_id();
            let (mut events, child) = app
                .shell()
                .sidecar(SPEECH_SIDECAR_NAME)
                .map_err(|_| {
                    VoiceCaptureError::new(
                        "voice_input_unavailable",
                        "Voice input is not available in this build of OOMU.",
                    )
                })?
                .spawn()
                .map_err(|_| {
                    VoiceCaptureError::new(
                        "voice_input_start_failed",
                        "Voice input could not start. Try again.",
                    )
                })?;

            state.active = Some(ActiveCapture {
                capture_id: capture_id.clone(),
                child,
                transcript: String::new(),
                final_seen: false,
            });
            drop(state);

            let manager = self.clone();
            let event_app = app.clone();
            let event_capture_id = capture_id.clone();
            tauri::async_runtime::spawn(async move {
                let mut reported_error = false;
                while let Some(event) = events.recv().await {
                    match event {
                        CommandEvent::Stdout(bytes) => {
                            match parse_bridge_output(&bytes, &event_capture_id) {
                                Ok(Some(payload)) => {
                                    reported_error |= payload.error_code.is_some();
                                    manager.observe_output(&payload);
                                    let _ = event_app.emit(VOICE_STREAM_EVENT, payload);
                                }
                                Ok(None) => {}
                                Err(error_code) => {
                                    reported_error = true;
                                    let _ = event_app.emit(
                                        VOICE_STREAM_EVENT,
                                        error_event(&event_capture_id, error_code),
                                    );
                                }
                            }
                        }
                        CommandEvent::Stderr(bytes) => {
                            eprintln!("OOMU_VOICE_BRIDGE_WARNING bytes={}", bytes.len().min(4_096));
                        }
                        CommandEvent::Error(_) => {
                            reported_error = true;
                            let _ = event_app.emit(
                                VOICE_STREAM_EVENT,
                                error_event(&event_capture_id, "voice_input_failed"),
                            );
                        }
                        CommandEvent::Terminated(payload) => {
                            let was_active = manager.clear_if_active(&event_capture_id);
                            if was_active && payload.code != Some(0) && !reported_error {
                                let _ = event_app.emit(
                                    VOICE_STREAM_EVENT,
                                    error_event(&event_capture_id, "voice_input_failed"),
                                );
                            }
                            break;
                        }
                        _ => {}
                    }
                }
                manager.clear_if_active(&event_capture_id);
            });

            Ok(VoiceCaptureStatus {
                capture_id,
                active: true,
            })
        }
    }

    fn stop(&self) -> Result<VoiceCaptureStatus, VoiceCaptureError> {
        let capture = self
            .state
            .lock()
            .map_err(|_| {
                VoiceCaptureError::new(
                    "voice_input_state_unavailable",
                    "Voice input could not stop cleanly.",
                )
            })?
            .active
            .take();

        let Some(capture) = capture else {
            return Ok(VoiceCaptureStatus {
                capture_id: String::new(),
                active: false,
            });
        };

        let capture_id = capture.capture_id.clone();
        self.remember_completed(capture.capture_id, capture.transcript, capture.final_seen);
        terminate_child(capture.child);
        Ok(VoiceCaptureStatus {
            capture_id,
            active: false,
        })
    }

    fn clear_if_active(&self, capture_id: &str) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state
            .active
            .as_ref()
            .is_some_and(|capture| capture.capture_id == capture_id)
        {
            if let Some(capture) = state.active.take() {
                remember_completed_locked(
                    &mut state,
                    capture.capture_id,
                    capture.transcript,
                    capture.final_seen,
                );
            }
            true
        } else {
            false
        }
    }

    fn observe_output(&self, payload: &VoiceStreamEvent) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let Some(active) = state.active.as_mut() else {
            return;
        };
        if active.capture_id != payload.capture_id || payload.error_code.is_some() {
            return;
        }
        if !payload.text.is_empty() {
            active.transcript = payload.text.clone();
        }
        active.final_seen |= payload.is_final;
    }

    fn remember_completed(&self, capture_id: String, transcript: String, final_seen: bool) {
        if let Ok(mut state) = self.state.lock() {
            remember_completed_locked(&mut state, capture_id, transcript, final_seen);
        }
    }

    pub(crate) fn take_matching_capture(
        &self,
        accepted_message: &str,
    ) -> Option<PendingVoiceCapture> {
        let accepted = normalized_voice_text(accepted_message);
        let now = unix_time_ms();
        let mut state = self.state.lock().ok()?;
        state.pending.retain(|capture| {
            now.saturating_sub(capture.completed_at_ms) <= PENDING_CAPTURE_LIFETIME_MS
        });
        let index = state.pending.iter().rposition(|capture| {
            let transcript = normalized_voice_text(&capture.transcript);
            !transcript.is_empty() && accepted.contains(&transcript)
        })?;
        state.pending.remove(index)
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.stop() {
            eprintln!("OOMU_VOICE_CAPTURE_SHUTDOWN_FAILED code={}", error.code);
        }
    }
}

fn remember_completed_locked(
    state: &mut VoiceCaptureState,
    capture_id: String,
    transcript: String,
    final_seen: bool,
) {
    let transcript = transcript.trim().to_string();
    if transcript.is_empty() {
        return;
    }
    state
        .pending
        .retain(|capture| capture.capture_id != capture_id);
    state.pending.push_back(PendingVoiceCapture {
        capture_id,
        transcript,
        final_seen,
        completed_at_ms: unix_time_ms(),
    });
    while state.pending.len() > MAX_PENDING_CAPTURES {
        state.pending.pop_front();
    }
}

fn normalized_voice_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unix_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[tauri::command]
pub fn start_voice_capture(
    app: AppHandle,
    manager: tauri::State<'_, VoiceCaptureManager>,
) -> Result<VoiceCaptureStatus, VoiceCaptureError> {
    manager.start(&app)
}

#[tauri::command]
pub fn stop_voice_capture(
    manager: tauri::State<'_, VoiceCaptureManager>,
) -> Result<VoiceCaptureStatus, VoiceCaptureError> {
    manager.stop()
}

fn next_capture_id() -> String {
    format!("voice-{}", NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed))
}

fn parse_bridge_output(
    bytes: &[u8],
    capture_id: &str,
) -> Result<Option<VoiceStreamEvent>, &'static str> {
    let raw = std::str::from_utf8(bytes).map_err(|_| "voice_input_invalid_output")?;
    let output: SpeechBridgeOutput =
        serde_json::from_str(raw.trim()).map_err(|_| "voice_input_invalid_output")?;
    let text = output
        .text
        .replace('\0', "")
        .chars()
        .take(MAX_TRANSCRIPT_CHARACTERS + 1)
        .collect::<String>();
    if text.chars().count() > MAX_TRANSCRIPT_CHARACTERS {
        return Err("voice_input_invalid_output");
    }
    let text = text.trim().to_string();
    let error_code = output
        .error_code
        .as_deref()
        .map(str::trim)
        .filter(|code| valid_error_code(code))
        .map(str::to_string);

    if text.is_empty() && error_code.is_none() {
        return Ok(None);
    }
    Ok(Some(VoiceStreamEvent {
        capture_id: capture_id.to_string(),
        text,
        is_final: output.is_final,
        error_code,
    }))
}

fn valid_error_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn error_event(capture_id: &str, error_code: &str) -> VoiceStreamEvent {
    VoiceStreamEvent {
        capture_id: capture_id.to_string(),
        text: String::new(),
        is_final: true,
        error_code: Some(error_code.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn terminate_child(child: CommandChild) {
    let pid = child.pid() as libc::pid_t;
    // SIGTERM lets AVAudioEngine remove its tap and end the speech request
    // before the helper exits. Fall back to the process handle's hard stop only
    // if the process can no longer be signalled normally.
    let signalled = unsafe { libc::kill(pid, libc::SIGTERM) } == 0;
    if !signalled {
        let _ = child.kill();
    }
}

#[cfg(not(target_os = "macos"))]
fn terminate_child(child: CommandChild) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_partial_and_final_transcripts() {
        let partial =
            parse_bridge_output(br#"{"text":"Book a meeting","is_final":false}"#, "voice-1")
                .unwrap()
                .unwrap();
        assert_eq!(partial.capture_id, "voice-1");
        assert_eq!(partial.text, "Book a meeting");
        assert!(!partial.is_final);

        let final_result = parse_bridge_output(
            br#"{"text":"Book a meeting tomorrow.","is_final":true}"#,
            "voice-1",
        )
        .unwrap()
        .unwrap();
        assert!(final_result.is_final);
        assert_eq!(final_result.text, "Book a meeting tomorrow.");
    }

    #[test]
    fn accepts_typed_helper_errors_without_exposing_details() {
        let event = parse_bridge_output(
            br#"{"text":"","is_final":true,"error_code":"microphone_permission_denied"}"#,
            "voice-2",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            event.error_code.as_deref(),
            Some("microphone_permission_denied")
        );
        assert!(event.text.is_empty());
    }

    #[test]
    fn rejects_unstructured_or_oversized_helper_output() {
        assert_eq!(
            parse_bridge_output(b"not-json", "voice-3"),
            Err("voice_input_invalid_output")
        );
        let oversized = format!(
            "{{\"text\":\"{}\",\"is_final\":false}}",
            "a".repeat(MAX_TRANSCRIPT_CHARACTERS + 1)
        );
        assert_eq!(
            parse_bridge_output(oversized.as_bytes(), "voice-3"),
            Err("voice_input_invalid_output")
        );
    }

    #[test]
    fn completed_voice_input_binds_only_to_a_turn_containing_its_transcript() {
        let manager = VoiceCaptureManager::default();
        manager.remember_completed(
            "voice-4".to_string(),
            "Tell me what is on my calendar today.".to_string(),
            true,
        );
        assert!(manager
            .take_matching_capture("Please inspect a project instead.")
            .is_none());
        let capture = manager
            .take_matching_capture("Tell me what is on my calendar today")
            .expect("the accepted turn must consume its matching voice capture");
        assert_eq!(capture.capture_id, "voice-4");
        assert!(capture.final_seen);
        assert!(manager
            .take_matching_capture("Tell me what is on my calendar today")
            .is_none());
    }
}
