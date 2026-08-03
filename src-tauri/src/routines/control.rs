use chrono::{Duration, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const CONTROL_KEY: &str = "_oomuRoutine";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RoutineControlEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume_at_ms: Option<i64>,
}

fn control_envelope(run_request: &Value) -> Result<Option<RoutineControlEnvelope>, String> {
    let Some(raw) = run_request.get(CONTROL_KEY) else {
        return Ok(None);
    };
    let control = serde_json::from_value::<RoutineControlEnvelope>(raw.clone())
        .map_err(|_| "Stored Routine controls are invalid.".to_string())?;
    if control.end_at_ms.is_none() && control.resume_at_ms.is_none() {
        return Err("Stored Routine controls are invalid.".to_string());
    }
    if control.end_at_ms.is_some_and(|value| value <= 0) {
        return Err("Stored Routine end boundary is invalid.".to_string());
    }
    if control.resume_at_ms.is_some_and(|value| value <= 0) {
        return Err("Stored Routine recurrence anchor is invalid.".to_string());
    }
    Ok(Some(control))
}

fn write_control_envelope(
    run_request: &Value,
    control: Option<RoutineControlEnvelope>,
) -> Result<Value, String> {
    let mut value = run_request.clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Routine task template must be a JSON object.".to_string())?;
    match control {
        Some(control) => {
            object.insert(
                CONTROL_KEY.to_string(),
                serde_json::to_value(control).map_err(|error| error.to_string())?,
            );
        }
        None => {
            object.remove(CONTROL_KEY);
        }
    }
    Ok(value)
}

pub(crate) fn next_midnight_ms(timezone: &str, after_ms: i64) -> Result<i64, String> {
    let zone: Tz = timezone
        .parse()
        .map_err(|_| "Routine timezone is invalid.".to_string())?;
    let local = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| "Routine start time is invalid.".to_string())?
        .with_timezone(&zone);
    let midnight = (local.date_naive() + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "Routine midnight boundary is invalid.".to_string())?;
    let boundary = match zone.from_local_datetime(&midnight) {
        chrono::LocalResult::Single(value) => value,
        chrono::LocalResult::Ambiguous(first, _) => first,
        chrono::LocalResult::None => {
            return Err(
                "Midnight does not exist in the selected timezone on that date.".to_string(),
            )
        }
    };
    Ok(boundary.timestamp_millis())
}

pub(crate) fn with_end_at_ms(run_request: &Value, end_at_ms: i64) -> Result<Value, String> {
    if end_at_ms <= 0 {
        return Err("Routine end time is invalid.".to_string());
    }
    if run_request.get(CONTROL_KEY).is_some() {
        return Err("Routine task template uses a reserved field.".to_string());
    }
    write_control_envelope(
        run_request,
        Some(RoutineControlEnvelope {
            end_at_ms: Some(end_at_ms),
            resume_at_ms: None,
        }),
    )
}

pub(crate) fn end_at_ms(run_request: &Value) -> Result<Option<i64>, String> {
    Ok(control_envelope(run_request)?.and_then(|control| control.end_at_ms))
}

pub(crate) fn with_run_now_resume_at_ms(
    run_request: &Value,
    resume_at_ms: i64,
) -> Result<Value, String> {
    if resume_at_ms <= 0 {
        return Err("Routine recurrence anchor is invalid.".to_string());
    }
    let mut control = control_envelope(run_request)?.unwrap_or(RoutineControlEnvelope {
        end_at_ms: None,
        resume_at_ms: None,
    });
    control.resume_at_ms.get_or_insert(resume_at_ms);
    write_control_envelope(run_request, Some(control))
}

pub(crate) fn run_now_resume_at_ms(run_request: &Value) -> Result<Option<i64>, String> {
    Ok(control_envelope(run_request)?.and_then(|control| control.resume_at_ms))
}

pub(crate) fn without_run_now_resume(run_request: &Value) -> Result<Value, String> {
    let Some(mut control) = control_envelope(run_request)? else {
        return Ok(run_request.clone());
    };
    control.resume_at_ms = None;
    let remaining = control.end_at_ms.map(|_| control);
    write_control_envelope(run_request, remaining)
}

pub(crate) fn without_controls(run_request: &Value) -> Result<Value, String> {
    let mut value = run_request.clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| "workflow_schedules.run_request_json must be a JSON object.".to_string())?;
    object.remove(CONTROL_KEY);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn midnight_uses_the_reviewed_timezone() {
        let before = Utc
            .with_ymd_and_hms(2026, 8, 2, 20, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let boundary = next_midnight_ms("America/New_York", before).unwrap();
        assert_eq!(
            boundary,
            Utc.with_ymd_and_hms(2026, 8, 3, 4, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        );
    }

    #[test]
    fn control_envelope_is_validated_and_removed_before_workflow_execution() {
        let controlled = with_end_at_ms(&json!({"inputs": {}}), 42).unwrap();
        assert_eq!(end_at_ms(&controlled).unwrap(), Some(42));
        assert_eq!(
            without_controls(&controlled).unwrap(),
            json!({"inputs": {}})
        );
        assert!(with_end_at_ms(&controlled, 43).is_err());
    }

    #[test]
    fn run_now_anchor_preserves_end_boundary_and_is_consumable_once() {
        let bounded = with_end_at_ms(&json!({"inputs": {}}), 100).unwrap();
        let queued = with_run_now_resume_at_ms(&bounded, 80).unwrap();
        assert_eq!(end_at_ms(&queued).unwrap(), Some(100));
        assert_eq!(run_now_resume_at_ms(&queued).unwrap(), Some(80));

        let duplicate = with_run_now_resume_at_ms(&queued, 90).unwrap();
        assert_eq!(run_now_resume_at_ms(&duplicate).unwrap(), Some(80));

        let consumed = without_run_now_resume(&duplicate).unwrap();
        assert_eq!(end_at_ms(&consumed).unwrap(), Some(100));
        assert_eq!(run_now_resume_at_ms(&consumed).unwrap(), None);
        assert_eq!(without_controls(&consumed).unwrap(), json!({"inputs": {}}));
    }

    #[test]
    fn run_now_only_control_disappears_after_consumption() {
        let queued = with_run_now_resume_at_ms(&json!({"inputs": {}}), 80).unwrap();
        assert_eq!(run_now_resume_at_ms(&queued).unwrap(), Some(80));
        assert_eq!(
            without_run_now_resume(&queued).unwrap(),
            json!({"inputs": {}})
        );
    }
}
