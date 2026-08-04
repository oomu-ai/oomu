use super::*;

#[test]
fn interval_schedule_adds_expected_duration() {
    assert_eq!(next_run_after("every 2 minutes", 1_000).unwrap(), 121_000);
    assert_eq!(next_run_after("every 1 hour", 1_000).unwrap(), 3_601_000);
}

#[test]
fn calendar_intervals_cover_days_through_years_without_fixed_duration_drift() {
    let start = Utc
        .with_ymd_and_hms(2026, 1, 31, 10, 15, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let expected = [
        ("every 1 day", (2026, 2, 1, 10, 15)),
        ("every 2 weeks", (2026, 2, 14, 10, 15)),
        ("every 1 month", (2026, 2, 28, 10, 15)),
        ("every 1 quarter", (2026, 4, 30, 10, 15)),
        ("every 1 year", (2027, 1, 31, 10, 15)),
    ];

    for (expression, (year, month, day, hour, minute)) in expected {
        let next = next_run_after_in_timezone(expression, "UTC", start).unwrap();
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
            "{expression}"
        );
    }
}

#[test]
fn calendar_day_interval_preserves_reviewed_wall_time_across_dst() {
    let zone: Tz = "America/New_York".parse().unwrap();
    let start = zone
        .with_ymd_and_hms(2026, 3, 7, 9, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let next = Utc
        .timestamp_millis_opt(
            next_run_after_in_timezone("every day", "America/New_York", start).unwrap(),
        )
        .single()
        .unwrap()
        .with_timezone(&zone);

    assert_eq!((next.year(), next.month(), next.day()), (2026, 3, 8));
    assert_eq!((next.hour(), next.minute()), (9, 0));
}

#[test]
fn timezone_cron_runs_once_during_a_fall_back_overlap() {
    let zone: Tz = "America/New_York".parse().unwrap();
    let before_overlap = zone
        .with_ymd_and_hms(2026, 11, 1, 0, 30, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let first =
        next_run_after_in_timezone("30 1 * * *", "America/New_York", before_overlap).unwrap();
    let first_local = Utc
        .timestamp_millis_opt(first)
        .single()
        .unwrap()
        .with_timezone(&zone);
    assert_eq!((first_local.hour(), first_local.minute()), (1, 30));

    let after_first = next_run_after_in_timezone("30 1 * * *", "America/New_York", first).unwrap();
    let after_first_local = Utc
        .timestamp_millis_opt(after_first)
        .single()
        .unwrap()
        .with_timezone(&zone);
    assert_eq!(
        (
            after_first_local.year(),
            after_first_local.month(),
            after_first_local.day(),
            after_first_local.hour(),
            after_first_local.minute(),
        ),
        (2026, 11, 2, 1, 30),
    );
}

#[test]
fn timezone_daily_runs_once_during_a_fall_back_overlap() {
    let first = Utc
        .with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let next = next_run_after_in_timezone("daily at 01:30", "America/New_York", first).unwrap();
    assert_eq!(
        next,
        Utc.with_ymd_and_hms(2026, 11, 2, 6, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    );
}

#[test]
fn interval_schedule_rejects_sub_minute_and_unknown_timeframes_truthfully() {
    let seconds = next_run_after_in_timezone("every second", "UTC", 1_000).unwrap_err();
    assert!(seconds.contains("Use minutes, hours, days, weeks, months, quarters, or years"));
    let decades = next_run_after_in_timezone("every decade", "UTC", 1_000).unwrap_err();
    assert!(decades.contains("Unsupported interval schedule unit"));
}

#[test]
fn daily_schedule_uses_local_clock_time() {
    let morning = Local
        .with_ymd_and_hms(2026, 1, 5, 8, 0, 0)
        .earliest()
        .unwrap()
        .timestamp_millis();
    let next = local_datetime(next_run_after("daily at 09:00", morning).unwrap()).unwrap();
    assert_eq!(next.hour(), 9);
    assert_eq!(next.minute(), 0);
    assert_eq!(next.day(), 5);

    let late = Local
        .with_ymd_and_hms(2026, 1, 5, 10, 0, 0)
        .earliest()
        .unwrap()
        .timestamp_millis();
    let next = local_datetime(next_run_after("daily at 09:00", late).unwrap()).unwrap();
    assert_eq!(next.hour(), 9);
    assert_eq!(next.minute(), 0);
    assert_eq!(next.day(), 6);
}

#[test]
fn simple_cron_schedule_matches_next_allowed_minute() {
    let start = Local
        .with_ymd_and_hms(2026, 1, 5, 9, 1, 30)
        .earliest()
        .unwrap()
        .timestamp_millis();
    let next = local_datetime(next_run_after("*/2 * * * *", start).unwrap()).unwrap();
    assert_eq!(next.minute() % 2, 0);
    assert_eq!(next.second(), 0);
}

#[test]
fn manual_schedule_is_not_recurring() {
    assert!(next_run_after("Manual run", 1_000).is_err());
}

#[test]
fn one_process_holds_the_scheduler_lease() {
    let root = std::env::temp_dir().join(format!("oomu-lease-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    assert!(acquire_scheduler_lease(&engine).unwrap());
    assert!(acquire_scheduler_lease(&engine).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_copy_uses_the_saved_active_locale() {
    let root = std::env::temp_dir().join(format!("oomu-scheduler-locale-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
        .upsert_app_preference("ui.active_locale", "es-ES")
        .unwrap();

    let copy = SchedulerCopy::load(&engine);
    let notification = background_notice_copy(
        &copy,
        "Correo diario",
        ExecutionStatus::Completed,
        Some(WorkflowCompletionKind::EmptyCollection),
        None,
        &[],
    )
    .unwrap();
    assert_eq!(notification.0, "Flujo de trabajo completado");
    assert_eq!(
            notification.1,
            "Correo diario se completó. No se encontró nada, así que no se ejecutaron los pasos siguientes."
        );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_copy_falls_back_to_english_without_rewriting_user_values() {
    let copy = SchedulerCopy::from_translations(&json!({}));
    assert_eq!(copy.completed_title, "Workflow Completed");
    assert_eq!(
            render_scheduler_copy(
                &copy.delivery_approval,
                &[("name", "Review {code}"), ("code", "safe-code")],
            ),
            "Review {code} needs approval. Reply '/approve safe-code approve' or '/approve safe-code deny' within 15 minutes."
        );
}

#[test]
fn every_supported_locale_has_complete_scheduler_copy() {
    const LOCALES: [&str; 12] = [
        "de-DE", "en-US", "es-ES", "fr-FR", "id-ID", "ja-JP", "pt-BR", "ru-RU", "uk-UA", "vi-VN",
        "zh-CN", "zh-TW",
    ];
    const REQUIRED: [(&str, &[&str]); 17] = [
        ("/workflow_scheduler/notification/completed_title", &[]),
        (
            "/workflow_scheduler/notification/completed_body",
            &["{name}"],
        ),
        (
            "/workflow_scheduler/notification/completed_empty_body",
            &["{name}"],
        ),
        ("/workflow_scheduler/notification/approval_title", &[]),
        (
            "/workflow_scheduler/notification/approval_body",
            &["{name}"],
        ),
        ("/workflow_scheduler/notification/failed_title", &[]),
        (
            "/workflow_scheduler/notification/failed_body",
            &["{name}", "{details}"],
        ),
        (
            "/workflow_scheduler/notification/run_failed_body",
            &["{name}", "{error}"],
        ),
        ("/workflow_scheduler/delivery/completed", &["{name}"]),
        ("/workflow_scheduler/delivery/completed_empty", &["{name}"]),
        (
            "/workflow_scheduler/delivery/completed_verified",
            &["{name}", "{filenames}"],
        ),
        (
            "/workflow_scheduler/delivery/approval",
            &["{name}", "{code}"],
        ),
        ("/workflow_scheduler/delivery/blocked", &["{name}"]),
        (
            "/workflow_scheduler/delivery/failed",
            &["{name}", "{error}"],
        ),
        (
            "/workflow_scheduler/delivery/failed_verified",
            &["{fallback}", "{filenames}"],
        ),
        ("/workflow_scheduler/delivery/repair", &[]),
        ("/workflow_scheduler/retry/waiting", &[]),
    ];
    let locale_dir = std::path::Path::new(crate::OOMU_MANIFEST_DIR).join("../src/locales");

    for locale in LOCALES {
        let raw = std::fs::read_to_string(locale_dir.join(format!("{locale}.json"))).unwrap();
        let translations: Value = serde_json::from_str(&raw).unwrap();
        for (pointer, placeholders) in REQUIRED {
            let value = translations
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| panic!("{locale} is missing {pointer}"));
            for placeholder in placeholders {
                assert!(
                    value.contains(placeholder),
                    "{locale} {pointer} must retain {placeholder}"
                );
            }
        }
    }
}
