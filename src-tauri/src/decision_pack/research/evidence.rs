use crate::decision_research_policy::{
    authority_profile, profile_allows_url, AuthorityClass, ResearchQueryAlternative,
    ResearchSubject,
};
use crate::{
    artifacts::decision_pack::{
        web_claim_evidence_digest, SourceAuthority, SourceAuthorityClass, WebClaim,
    },
    dom_streaming::DomContext,
};

pub(super) struct ScoredClaim {
    pub(super) claim: WebClaim,
    pub(super) score: usize,
}

pub(super) fn claim_candidates(
    subject: ResearchSubject,
    alternative: &ResearchQueryAlternative,
    pages: &[DomContext],
    accessed_at: &str,
) -> Vec<ScoredClaim> {
    let Some(profile) = authority_profile(&alternative.authority_profile) else {
        return Vec::new();
    };
    pages
        .iter()
        .filter(|page| profile_allows_url(profile, &page.url))
        .filter_map(|page| claim_from_page(subject, page, profile, accessed_at))
        .collect()
}

pub(super) fn legacy_claim_candidates(
    subject: ResearchSubject,
    pages: &[DomContext],
    accessed_at: &str,
) -> Vec<ScoredClaim> {
    pages
        .iter()
        .filter_map(|page| {
            let profile = crate::decision_research_policy::authority_profile_for_url(&page.url)?;
            claim_from_page(subject, page, profile, accessed_at)
        })
        .collect()
}

pub(super) fn canonical_source_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return String::new();
    };
    url.set_fragment(None);
    let mut pairs = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "ref" | "source" | "campaign")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    pairs.sort();
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    url.to_string()
}

fn claim_from_page(
    subject: ResearchSubject,
    page: &DomContext,
    profile: crate::decision_research_policy::AuthorityProfile,
    accessed_at: &str,
) -> Option<ScoredClaim> {
    let (evidence, temporal) = evidence_segments(subject, page)
        .into_iter()
        .filter_map(|evidence| {
            super::date_evidence::claim_date(subject, page, &evidence.text)
                .map(|temporal| (evidence, temporal))
        })
        .max_by(
            |(left_evidence, left_temporal), (right_evidence, right_temporal)| {
                (left_evidence.score + left_temporal.score)
                    .cmp(&(right_evidence.score + right_temporal.score))
                    .then_with(|| {
                        subject_specificity(subject, &left_evidence.text)
                            .cmp(&subject_specificity(subject, &right_evidence.text))
                    })
                    .then_with(|| left_temporal.date.cmp(&right_temporal.date))
                    .then_with(|| left_evidence.text.len().cmp(&right_evidence.text.len()))
            },
        )?;
    let title = bounded(
        if page.title.trim().is_empty() {
            profile.organization
        } else {
            &page.title
        },
        240,
    );
    let url = canonical_source_url(&page.url);
    let authority = SourceAuthority {
        profile_id: profile.id.to_string(),
        organization: profile.organization.to_string(),
        class: match profile.class {
            AuthorityClass::Government => SourceAuthorityClass::Government,
            AuthorityClass::Intergovernmental => SourceAuthorityClass::Intergovernmental,
            AuthorityClass::RegisteredFirstParty => SourceAuthorityClass::RegisteredFirstParty,
        },
    };
    let claim = bounded(&evidence.text, 1_100);
    let effective_date = temporal.date.format("%Y-%m-%d").to_string();
    let evidence_digest = web_claim_evidence_digest(
        subject.as_str(),
        &claim,
        &title,
        &authority,
        &effective_date,
        temporal.kind,
        &url,
    );
    Some(ScoredClaim {
        claim: WebClaim {
            subject: subject.as_str().to_string(),
            claim,
            source_title: title,
            authority,
            effective_date,
            date_evidence_type: temporal.kind,
            url,
            accessed_at: accessed_at.to_string(),
            evidence_digest,
        },
        score: evidence.score + temporal.score,
    })
}

struct EvidenceSegment {
    text: String,
    score: usize,
}

fn evidence_segments(subject: ResearchSubject, page: &DomContext) -> Vec<EvidenceSegment> {
    let mut candidates = Vec::new();
    for table in &page.tables {
        if let Some((dated_header, _)) = table
            .rows
            .iter()
            .map(|row| normalized_line(&row.join(" | ")))
            .filter_map(|text| {
                super::date_evidence::claim_date(subject, page, &text)
                    .map(|temporal| (text, temporal.date))
            })
            .max_by_key(|(_, date)| *date)
        {
            for row in &table.rows {
                let row = normalized_line(&row.join(" | "));
                if row == dated_header || row.is_empty() {
                    continue;
                }
                let text = normalized_line(&format!("{} | {dated_header} | {row}", table.label));
                if acceptable_evidence_text(subject, &text) {
                    candidates.push(EvidenceSegment {
                        score: 70 + national_scope_score(&row),
                        text,
                    });
                }
            }
        }
        for row in &table.rows {
            let text = normalized_line(&row.join(" | "));
            if acceptable_evidence_text(subject, &text) {
                candidates.push(EvidenceSegment { text, score: 50 });
            }
        }
    }
    for line in page.visible_text.lines() {
        let text = normalized_line(line);
        if acceptable_evidence_text(subject, &text) {
            candidates.push(EvidenceSegment { text, score: 30 });
        }
    }
    candidates
}

fn national_scope_score(row: &str) -> usize {
    let first_cell = row
        .split('|')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    usize::from(matches!(
        first_cell.as_str(),
        "u.s." | "u.s" | "us" | "united states" | "national"
    )) * 12
}

fn acceptable_evidence_text(subject: ResearchSubject, text: &str) -> bool {
    (40..=1_200).contains(&text.chars().count())
        && contains_subject(subject, text)
        && !looks_like_navigation(text)
}

fn contains_subject(subject: ResearchSubject, text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    subject_terms(subject)
        .iter()
        .any(|term| lowered.contains(term))
}

fn subject_specificity(subject: ResearchSubject, text: &str) -> usize {
    let lowered = text.to_ascii_lowercase();
    subject_terms(subject)
        .iter()
        .filter(|term| lowered.contains(**term))
        .count()
}

fn subject_terms(subject: ResearchSubject) -> &'static [&'static str] {
    match subject {
        ResearchSubject::Fuel => &["fuel", "diesel", "gasoline", "petroleum"],
        ResearchSubject::Freight => &[
            "freight",
            "transportation services index",
            "truck transportation",
            "rail transportation",
            "air cargo",
        ],
    }
}

fn looks_like_navigation(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    [
        "skip to",
        "navigation",
        "menu",
        "subscribe",
        "privacy policy",
    ]
    .iter()
    .any(|term| lowered.contains(term))
}

fn normalized_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bounded(value: &str, maximum: usize) -> String {
    value.trim().chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::decision_pack::DateEvidenceType;
    use crate::dom_streaming::{DomTable, DomTemporalEvidence};
    use chrono::{Datelike, Duration, NaiveDate, Utc};

    fn context(url: &str, title: &str, visible_text: &str) -> DomContext {
        DomContext {
            url: url.to_string(),
            title: title.to_string(),
            visible_text: visible_text.to_string(),
            inputs: Vec::new(),
            buttons: Vec::new(),
            links: Vec::new(),
            tables: Vec::new(),
            temporal_evidence: Vec::new(),
            extraction_method: "static_html".to_string(),
        }
    }

    #[test]
    fn qualifies_current_bts_freight_release_without_a_hard_coded_url() {
        let today = Utc::now().date_naive();
        let observation = today - Duration::days(30);
        let observation_month =
            NaiveDate::from_ymd_opt(observation.year(), observation.month(), 1).unwrap();
        let mut page = context(
            "https://www.bts.gov/newsroom/a-current-freight-release",
            &format!(
                "{} Freight Transportation Services Index",
                observation.format("%B %Y")
            ),
            &format!("The Freight Transportation Services Index changed 1.3 percent in {} and materially informs current freight conditions.", observation.format("%B %Y")),
        );
        page.temporal_evidence.push(DomTemporalEvidence {
            value: today.format("%Y-%m-%d").to_string(),
            evidence_type: "publicationDate".to_string(),
            label: "datePublished".to_string(),
        });
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official freight conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);
        let claims = claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0].claim.effective_date,
            observation_month.format("%Y-%m-%d").to_string()
        );
        assert_eq!(
            claims[0].claim.date_evidence_type,
            DateEvidenceType::ObservationDate
        );
    }

    #[test]
    fn binds_eia_release_or_observation_date_to_diesel_evidence() {
        let today = Utc::now().date_naive();
        let observation = today - Duration::days(1);
        let mut page = context(
            "https://www.eia.gov/petroleum/gasdiesel/?utm_source=test",
            "Gasoline and Diesel Fuel Update",
            &format!(
                "Diesel Fuel Release Date: {}\nU.S. on-highway diesel fuel price for {} was reported in dollars per gallon.",
                today.format("%B %-d, %Y"),
                observation.format("%m/%d/%y")
            ),
        );
        page.tables.push(DomTable {
            label: String::new(),
            rows: vec![vec![
                observation.format("%m/%d/%y").to_string(),
                "U.S. on-highway diesel fuel price 3.767 dollars per gallon".to_string(),
            ]],
        });
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official fuel conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);
        let claims = claim_candidates(
            ResearchSubject::Fuel,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0].claim.effective_date,
            observation.format("%Y-%m-%d").to_string()
        );
        assert_eq!(
            claims[0].claim.url,
            "https://www.eia.gov/petroleum/gasdiesel/"
        );
    }

    #[test]
    fn binds_labeled_table_header_date_to_its_national_metric_row() {
        let today = Utc::now().date_naive();
        let prior_week = today - Duration::days(7);
        let mut page = context(
            "https://www.eia.gov/petroleum/gasdiesel/",
            "Gasoline and Diesel Fuel Update",
            "Diesel fuel release and methodology information without a claim-bound date.",
        );
        page.tables.push(DomTable {
            label: "U.S. On-Highway Diesel Fuel Prices (dollars per gallon)".to_string(),
            rows: vec![
                vec![
                    prior_week.format("%m/%d/%y").to_string(),
                    today.format("%m/%d/%y").to_string(),
                    "week ago".to_string(),
                ],
                vec![
                    "U.S.".to_string(),
                    "3.777".to_string(),
                    "3.855".to_string(),
                    "0.078".to_string(),
                ],
                vec![
                    "West Coast less California".to_string(),
                    "4.422".to_string(),
                    "4.393".to_string(),
                    "-0.029".to_string(),
                ],
            ],
        });
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official fuel conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);

        let claims = claim_candidates(
            ResearchSubject::Fuel,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        );

        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0].claim.effective_date,
            today.format("%Y-%m-%d").to_string()
        );
        assert!(claims[0]
            .claim
            .claim
            .contains("U.S. On-Highway Diesel Fuel Prices"));
        assert!(claims[0].claim.claim.contains("U.S. | 3.777 | 3.855"));
        assert!(!claims[0].claim.claim.contains("West Coast less California"));
    }

    #[tokio::test]
    #[ignore = "requires live access to the registered EIA public page"]
    async fn live_eia_direct_context_produces_a_current_decision_pack_claim() {
        let objective =
            "Independently research current primary or official web sources for fuel conditions.";
        let policy = crate::decision_research_policy::compile_research_policy(objective).unwrap();
        let digest = crate::decision_research_policy::policy_digest(&policy).unwrap();
        let alternative = policy.subjects[0].query_alternatives[0].clone();
        let authorization =
            crate::sovereign_search::SovereignSearchAuthorization::approved_decision_pack(
                "plan-live-eia-claim",
                objective,
                &alternative.query,
                policy,
                digest,
            );
        let response = crate::sovereign_search::execute_sovereign_duckduckgo_search(
            crate::sovereign_search::SovereignSearchExecutionRequest::approved_action_plan(
                &alternative.query,
                Some(5),
                None,
                authorization,
            ),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(!response.degraded, "{:?}", response.error);
        let context: super::super::SearchContext =
            serde_json::from_str(&response.context_json).unwrap();
        let claims = claim_candidates(
            ResearchSubject::Fuel,
            &alternative,
            &context.pages,
            &Utc::now().to_rfc3339(),
        );
        eprintln!(
            "LIVE_EIA_CLAIM_DIAGNOSTIC pages={} tables={} labeled_tables={} temporal={} claims={}",
            context.pages.len(),
            context
                .pages
                .iter()
                .map(|page| page.tables.len())
                .sum::<usize>(),
            context
                .pages
                .iter()
                .flat_map(|page| page.tables.iter())
                .filter(|table| !table.label.is_empty())
                .count(),
            context
                .pages
                .iter()
                .map(|page| page.temporal_evidence.len())
                .sum::<usize>(),
            claims.len(),
        );
        assert!(
            !claims.is_empty(),
            "live EIA DOM context did not yield a current claim"
        );
    }

    #[test]
    fn rejects_recently_modified_page_when_selected_freight_statistic_is_stale() {
        let today = Utc::now().date_naive();
        let stale_observation = today - Duration::days(400);
        let mut page = context(
            "https://www.bts.gov/archive/freight-index",
            "Freight Transportation Services Index archive",
            &format!(
                "The Freight Transportation Services Index was 118.2 on {} and describes that historical reporting period.",
                stale_observation.format("%B %-d, %Y")
            ),
        );
        page.temporal_evidence.extend([
            DomTemporalEvidence {
                value: today.format("%Y-%m-%d").to_string(),
                evidence_type: "updatedDate".to_string(),
                label: "dateModified".to_string(),
            },
            DomTemporalEvidence {
                value: today.format("%Y-%m-%d").to_string(),
                evidence_type: "publicationDate".to_string(),
                label: "datePublished".to_string(),
            },
        ]);
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official freight conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);

        assert!(claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        )
        .is_empty());
    }

    #[test]
    fn rejects_generic_fresh_update_date_embedded_beside_a_stale_statistic() {
        let today = Utc::now().date_naive();
        let stale_observation = today - Duration::days(400);
        let page = context(
            "https://www.bts.gov/archive/freight-index",
            "Freight Transportation Services Index archive",
            &format!(
                "Updated {} — the Freight Transportation Services Index was 118.2 in {}.",
                today.format("%B %-d, %Y"),
                stale_observation.format("%B %Y")
            ),
        );
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official freight conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);

        assert!(claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        )
        .is_empty());
    }

    #[test]
    fn page_publication_metadata_can_only_classify_the_same_claim_bound_date() {
        let today = Utc::now().date_naive();
        let mut page = context(
            "https://www.bts.gov/newsroom/freight-release",
            "Freight Transportation Services Index release",
            &format!(
                "The Freight Transportation Services Index release date was {}, when the index was reported at 121.4.",
                today.format("%B %-d, %Y")
            ),
        );
        page.temporal_evidence.push(DomTemporalEvidence {
            value: today.format("%Y-%m-%d").to_string(),
            evidence_type: "publicationDate".to_string(),
            label: "datePublished".to_string(),
        });
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official freight conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);
        let claims = claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339(),
        );

        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0].claim.date_evidence_type,
            DateEvidenceType::PublicationDate
        );
        assert_eq!(
            claims[0].claim.effective_date,
            today.format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn rejects_lookalike_authority_and_unrelated_dates() {
        let today = Utc::now().date_naive();
        let page = context(
            "https://www.bts.gov.example.com/freight",
            "Freight report",
            &format!("The freight transportation services index changed during the latest reporting period. Published {}.", today.format("%B %-d, %Y")),
        );
        let alternative = crate::decision_research_policy::compile_research_policy(
            "research official freight conditions",
        )
        .unwrap()
        .subjects
        .remove(0)
        .query_alternatives
        .remove(0);
        assert!(claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[page],
            &Utc::now().to_rfc3339()
        )
        .is_empty());

        let unrelated_date = context(
            "https://www.bts.gov/newsroom/freight",
            "Freight report",
            &format!("The freight transportation services index changed during the latest reporting period.\nContext line one.\nContext line two.\nContext line three.\nWebsite footer updated {}.", today.format("%B %-d, %Y")),
        );
        assert!(claim_candidates(
            ResearchSubject::Freight,
            &alternative,
            &[unrelated_date],
            &Utc::now().to_rfc3339()
        )
        .is_empty());
    }
}
