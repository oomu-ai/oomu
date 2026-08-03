use serde::Serialize;
use tauri::{DragDropEvent, Emitter, Manager, PhysicalPosition, Runtime, Window};

const OOMU_LOCAL_CONTEXT_DRAG_EVENT: &str = "oomu://local-context-drag";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalContextDragPosition {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalContextDragPayload {
    #[serde(rename = "type")]
    event_type: &'static str,
    drop_id: Option<String>,
    position: Option<LocalContextDragPosition>,
}

pub(crate) fn webview_drag_position(position: &PhysicalPosition<f64>) -> LocalContextDragPosition {
    // WRY reports drag locations relative to the webview. On macOS those
    // values are already Cocoa points, even though Tauri carries them in a
    // PhysicalPosition. Dividing them by the Retina scale factor moves the
    // apparent drop away from the pointer and makes the bottom composer miss.
    LocalContextDragPosition {
        x: position.x,
        y: position.y,
    }
}

pub(crate) fn emit_local_context_drag<R: Runtime>(window: &Window<R>, drag_event: &DragDropEvent) {
    let payload = match drag_event {
        DragDropEvent::Enter { position, .. } => LocalContextDragPayload {
            event_type: "enter",
            drop_id: None,
            position: Some(webview_drag_position(position)),
        },
        DragDropEvent::Over { position } => LocalContextDragPayload {
            event_type: "over",
            drop_id: None,
            position: Some(webview_drag_position(position)),
        },
        DragDropEvent::Drop { paths, position } => {
            let drop_id = match crate::local_context::register_dropped_local_context(
                window
                    .state::<crate::local_context::LocalContextGrantStore>()
                    .inner(),
                paths,
            ) {
                Ok(drop_id) => drop_id,
                Err(error) => {
                    eprintln!(
                        "OOMU_LOCAL_CONTEXT_DROP_REGISTRATION_FAILED error={}",
                        crate::redaction::redacted_log_text(&error),
                    );
                    None
                }
            };
            LocalContextDragPayload {
                event_type: "drop",
                drop_id,
                position: Some(webview_drag_position(position)),
            }
        }
        DragDropEvent::Leave => LocalContextDragPayload {
            event_type: "leave",
            drop_id: None,
            position: None,
        },
        _ => return,
    };
    if payload.event_type == "drop" {
        eprintln!(
            "OOMU_LOCAL_CONTEXT_DROP_RECEIVED status={} position_x={} position_y={}",
            if payload.drop_id.is_some() {
                "registered"
            } else {
                "rejected"
            },
            payload
                .position
                .as_ref()
                .map(|value| value.x)
                .unwrap_or_default(),
            payload
                .position
                .as_ref()
                .map(|value| value.y)
                .unwrap_or_default(),
        );
    }
    if let Err(error) = window.emit(OOMU_LOCAL_CONTEXT_DRAG_EVENT, payload) {
        eprintln!("OOMU_LOCAL_CONTEXT_DRAG_EVENT_FAILED error={error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_webview_relative_drag_coordinates() {
        let position = webview_drag_position(&PhysicalPosition::new(812.0, 684.0));
        assert_eq!(position.x, 812.0);
        assert_eq!(position.y, 684.0);
    }
}
