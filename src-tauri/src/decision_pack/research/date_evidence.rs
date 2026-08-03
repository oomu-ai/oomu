use crate::decision_research_policy::ResearchSubject;
use crate::{
    artifacts::decision_pack::DateEvidenceType,
    dom_streaming::{DomContext, DomTemporalEvidence},
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use regex::Regex;

pub(super) struct SelectedDate {
    pub(super) date: NaiveDate,
    pub(super) kind: DateEvidenceType,
    pub(super) score: usize,
}

pub(super) fn claim_date(
    _subject: ResearchSubject,
    page: &DomContext,
    evidence: &str,
) -> Option<SelectedDate> {
    // Freshness belongs to the selected statistic, not to its containing page.
    // A newly edited landing page can legitimately retain years of historical
    // observations, so its datePublished/dateModified metadata must never turn
    // an old row into a current claim.
    let claim_dates = claim_bound_dates(evidence);
    if claim_dates.is_empty() {
        return None;
    }

    let mut matches = claim_dates
        .iter()
        .map(|candidate| SelectedDate {
            date: candidate.date,
            kind: candidate.kind,
            score: 60,
        })
        .collect::<Vec<_>>();
    let corroborated_dates = claim_dates
        .iter()
        .map(|candidate| candidate.date)
        .collect::<Vec<_>>();

    // Page metadata can preserve the source's own date classification only
    // when it names the exact date already present in the selected evidence.
    // It can corroborate claim-bound evidence; it cannot supply freshness.
    matches.extend(
        page.temporal_evidence
            .iter()
            .filter_map(|temporal| temporal_match(temporal, &corroborated_dates)),
    );
    matches.into_iter().max_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.date.cmp(&right.date))
    })
}

fn temporal_match(
    evidence: &DomTemporalEvidence,
    claim_dates: &[NaiveDate],
) -> Option<SelectedDate> {
    let kind = match evidence.evidence_type.as_str() {
        "publicationDate" => DateEvidenceType::PublicationDate,
        "releaseDate" => DateEvidenceType::ReleaseDate,
        "updatedDate" => DateEvidenceType::UpdatedDate,
        _ => return None,
    };
    extracted_dates(&evidence.value)
        .into_iter()
        .filter(|date| fresh(*date) && claim_dates.contains(date))
        .max()
        .map(|date| SelectedDate {
            date,
            kind,
            score: 65,
        })
}

struct ClaimBoundDate {
    date: NaiveDate,
    kind: DateEvidenceType,
}

fn claim_bound_dates(text: &str) -> Vec<ClaimBoundDate> {
    date_occurrences(text)
        .into_iter()
        .filter(|occurrence| fresh(occurrence.date))
        .filter_map(|occurrence| {
            claim_period_kind(text, &occurrence).map(|kind| ClaimBoundDate {
                date: occurrence.date,
                kind,
            })
        })
        .collect()
}

fn claim_period_kind(text: &str, occurrence: &DateOccurrence) -> Option<DateEvidenceType> {
    let before = text
        .get(..occurrence.start)?
        .chars()
        .rev()
        .take(72)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .to_ascii_lowercase();
    let after = text
        .get(occurrence.end..)?
        .chars()
        .take(72)
        .collect::<String>()
        .to_ascii_lowercase();
    let immediate_before = before
        .chars()
        .rev()
        .take(36)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let immediate_after = after.chars().take(36).collect::<String>();
    let immediate = format!("{immediate_before} {immediate_after}");
    if ["updated", "modified", "published", "publication"]
        .iter()
        .any(|marker| immediate.contains(marker))
    {
        return None;
    }

    let nearby = format!("{before} {after}");
    if ["release date", "released on", "release for"]
        .iter()
        .any(|marker| nearby.contains(marker))
    {
        return Some(DateEvidenceType::ReleaseDate);
    }
    if [
        "observation date",
        "week ending",
        "as of",
        "data for",
        "index for",
        "price for",
        "reporting period",
    ]
    .iter()
    .any(|marker| nearby.contains(marker))
        || period_preposition(&immediate_before)
        || table_row_date(text, occurrence)
        || dated_metric_heading(text, occurrence)
    {
        return Some(DateEvidenceType::ObservationDate);
    }
    None
}

fn period_preposition(before: &str) -> bool {
    let before = before.trim_end();
    [" in", " on", " for", " through", " ending"]
        .iter()
        .any(|marker| before.ends_with(marker))
}

fn table_row_date(text: &str, occurrence: &DateOccurrence) -> bool {
    text.get(occurrence.end..)
        .is_some_and(|suffix| suffix.trim_start().starts_with('|'))
        || text
            .get(..occurrence.start)
            .is_some_and(|prefix| prefix.trim_end().ends_with('|'))
}

fn dated_metric_heading(text: &str, occurrence: &DateOccurrence) -> bool {
    let starts_with_date = text
        .get(..occurrence.start)
        .is_some_and(|prefix| prefix.trim().is_empty());
    if !starts_with_date {
        return false;
    }
    text.get(occurrence.end..).is_some_and(|suffix| {
        let lowered = suffix.to_ascii_lowercase();
        [
            "freight",
            "transportation services index",
            "fuel",
            "diesel",
            "gasoline",
            "petroleum",
        ]
        .iter()
        .any(|term| lowered.contains(term))
    })
}

fn fresh(date: NaiveDate) -> bool {
    let today = Utc::now().date_naive();
    date <= today + Duration::days(14) && today.signed_duration_since(date).num_days() <= 120
}

struct DateOccurrence {
    date: NaiveDate,
    start: usize,
    end: usize,
}

fn extracted_dates(text: &str) -> Vec<NaiveDate> {
    date_occurrences(text)
        .into_iter()
        .map(|occurrence| occurrence.date)
        .collect()
}

fn date_occurrences(text: &str) -> Vec<DateOccurrence> {
    let mut occurrences = Vec::new();
    let iso = Regex::new(r"\b(20\d{2})-(0[1-9]|1[0-2])-([0-2]\d|3[01])\b").expect("ISO date regex");
    for captures in iso.captures_iter(text) {
        push_occurrence(&mut occurrences, &captures, 1, 2, 3);
    }
    let numeric = Regex::new(r"\b(0?[1-9]|1[0-2])/([0-2]?\d|3[01])/(20\d{2}|\d{2})\b")
        .expect("numeric date regex");
    for captures in numeric.captures_iter(text) {
        let month = captures
            .get(1)
            .and_then(|value| value.as_str().parse().ok());
        let day = captures
            .get(2)
            .and_then(|value| value.as_str().parse().ok());
        let year = captures
            .get(3)
            .and_then(|value| value.as_str().parse::<i32>().ok());
        if let (Some(month), Some(day), Some(mut year)) = (month, day, year) {
            if year < 100 {
                year += 2000;
            }
            if let (Some(date), Some(full_match)) =
                (NaiveDate::from_ymd_opt(year, month, day), captures.get(0))
            {
                occurrences.push(DateOccurrence {
                    date,
                    start: full_match.start(),
                    end: full_match.end(),
                });
            }
        }
    }
    for (month, number) in [
        ("January", 1),
        ("February", 2),
        ("March", 3),
        ("April", 4),
        ("May", 5),
        ("June", 6),
        ("July", 7),
        ("August", 8),
        ("September", 9),
        ("October", 10),
        ("November", 11),
        ("December", 12),
    ] {
        let pattern = Regex::new(&format!(
            r"(?i)\b{month}\s+([0-2]?\d|3[01]),?\s+(20\d{{2}})\b"
        ))
        .expect("month date regex");
        for captures in pattern.captures_iter(text) {
            let day = captures
                .get(1)
                .and_then(|value| value.as_str().parse().ok());
            let year = captures
                .get(2)
                .and_then(|value| value.as_str().parse().ok());
            if let (Some(day), Some(year)) = (day, year) {
                if let (Some(date), Some(full_match)) =
                    (NaiveDate::from_ymd_opt(year, number, day), captures.get(0))
                {
                    occurrences.push(DateOccurrence {
                        date,
                        start: full_match.start(),
                        end: full_match.end(),
                    });
                }
            }
        }
        let month_year =
            Regex::new(&format!(r"(?i)\b{month}\s+(20\d{{2}})\b")).expect("month-year regex");
        for captures in month_year.captures_iter(text) {
            if let Some(year) = captures
                .get(1)
                .and_then(|value| value.as_str().parse().ok())
            {
                if let (Some(date), Some(full_match)) =
                    (NaiveDate::from_ymd_opt(year, number, 1), captures.get(0))
                {
                    occurrences.push(DateOccurrence {
                        date,
                        start: full_match.start(),
                        end: full_match.end(),
                    });
                }
            }
        }
    }
    occurrences
        .retain(|occurrence| (2020..=Utc::now().year() + 1).contains(&occurrence.date.year()));
    occurrences
}

fn push_occurrence(
    occurrences: &mut Vec<DateOccurrence>,
    captures: &regex::Captures<'_>,
    year_index: usize,
    month_index: usize,
    day_index: usize,
) {
    let year = captures
        .get(year_index)
        .and_then(|value| value.as_str().parse().ok());
    let month = captures
        .get(month_index)
        .and_then(|value| value.as_str().parse().ok());
    let day = captures
        .get(day_index)
        .and_then(|value| value.as_str().parse().ok());
    if let (Some(year), Some(month), Some(day), Some(full_match)) =
        (year, month, day, captures.get(0))
    {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            occurrences.push(DateOccurrence {
                date,
                start: full_match.start(),
                end: full_match.end(),
            });
        }
    }
}
