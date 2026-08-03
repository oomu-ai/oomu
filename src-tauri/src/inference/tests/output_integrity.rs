use super::*;

#[test]
fn zero_mockery_validator_detects_forced_placeholder_signatures() {
    assert_eq!(
        output_mockery_violation("Flight total: $X,XXX"),
        Some(OutputMockeryViolation::PlaceholderCurrency)
    );
    assert_eq!(
        output_mockery_violation("Use fake_price until checkout."),
        Some(OutputMockeryViolation::PlaceholderTerm)
    );
    assert_eq!(
        output_mockery_violation("Evidence: []"),
        Some(OutputMockeryViolation::EmptyEvidence)
    );
    assert_eq!(
        output_mockery_violation("Verified flight total: $1,284."),
        None
    );
}

#[test]
fn zero_mockery_gate_blocks_forced_output_logs_and_retries_before_render() {
    use std::cell::{Cell, RefCell};

    fn text(value: &String) -> &str {
        value
    }

    let ledger = RefCell::new(Vec::new());
    let retry_count = Cell::new(0usize);
    let rendered = RefCell::new(Vec::new());
    let forced_model_output = "Best fare: $X,XXX".to_string();

    let (validated, retried) = validate_zero_mockery_with_retry(
        forced_model_output,
        text,
        |violation, attempt, rejected| {
            ledger
                .borrow_mut()
                .push((violation.code().to_string(), attempt, rejected.clone()));
            Ok(())
        },
        |_violation| {
            retry_count.set(retry_count.get() + 1);
            Ok("Scraper data is unavailable. I cannot verify pricing, aborting turn.".to_string())
        },
        |_violation, rejected| rejected,
    )
    .expect("clean retry");

    assert!(retried);
    assert_eq!(retry_count.get(), 1);
    assert_eq!(ledger.borrow().len(), 1);
    assert!(rendered.borrow().is_empty());
    assert!(output_mockery_violation(&validated).is_none());

    rendered.borrow_mut().push(validated);
    assert_eq!(
        rendered.borrow().as_slice(),
        ["Scraper data is unavailable. I cannot verify pricing, aborting turn."]
    );
}

#[test]
fn zero_mockery_gate_replaces_a_second_invalid_attempt_with_an_honest_deficit() {
    use std::cell::RefCell;

    fn text(value: &String) -> &str {
        value
    }

    let ledger = RefCell::new(Vec::new());
    let (validated, retried) = validate_zero_mockery_with_retry(
        "Best fare: $X,XXX".to_string(),
        text,
        |violation, attempt, _rejected| {
            ledger
                .borrow_mut()
                .push((violation.code().to_string(), attempt));
            Ok(())
        },
        |_violation| Ok("Fallback fare: $Y,YYY".to_string()),
        |violation, _rejected| violation.honest_deficit().to_string(),
    )
    .expect("second placeholder output must be replaced before rendering");

    assert!(retried);
    assert_eq!(
        validated,
        OutputMockeryViolation::PlaceholderCurrency.honest_deficit()
    );
    assert!(output_mockery_violation(&validated).is_none());
    assert_eq!(ledger.borrow().len(), 2);
}
