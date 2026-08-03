pub(super) fn rate_flag(
    historical_rate: f64,
    active_quote: f64,
    status: &str,
) -> (&'static str, &'static str) {
    let status = status.to_ascii_lowercase();
    if ["exception", "conflict", "mismatch", "review", "risk"]
        .iter()
        .any(|term| status.contains(term))
    {
        ("Review", "flag_review")
    } else if historical_rate == active_quote {
        ("Aligned", "flag_ok")
    } else {
        ("Changed", "flag_review")
    }
}
