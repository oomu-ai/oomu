use crate::{
    artifacts::decision_pack::{
        web_claim_evidence_digest, DateEvidenceType, ResearchGap, ResearchGapReason,
        SourceAuthority, SourceAuthorityClass, WebClaim,
    },
    decision_research_policy::{authority_profile_for_url, AuthorityClass},
};
use regex::{Captures, Regex};

pub(super) fn parse(sources: &str) -> Result<(Vec<WebClaim>, Vec<ResearchGap>), String> {
    let sections = sources
        .split_once("## Current web sources\n\n")
        .and_then(|(_, remainder)| remainder.split_once("\n\n## Research gaps\n\n"))
        .ok_or_else(|| {
            "The existing source ledger is missing its research evidence.".to_string()
        })?;
    if sections.0 == "No web claims were included in the canonical analysis.\n" {
        return Err("The existing source ledger has no verified web evidence.".to_string());
    }
    Ok((
        parse_web_claims(sections.0)?,
        parse_research_gaps(sections.1)?,
    ))
}

fn parse_web_claims(source: &str) -> Result<Vec<WebClaim>, String> {
    let pattern = Regex::new(
        r"(?s)^\d+\. \*\*(.+?)\*\* — (.+?)  \n   Authority: (.+?) \((government|intergovernmental|registered first-party)\)  \n   Source: \[(.+?)\]\((.+?)\)  \n   Effective date: `([^`]+)` \((publication date|release date|observation date|updated date)\)  \n   Accessed: `([^`]+)`  \n   Evidence digest: `([0-9a-f]{64})`  \n   Canonical source: `decision-pack-web-claim-\d+`  \n   Canonical evidence: `canonical-analysis-web-claim-\d+`$",
    )
    .map_err(|error| error.to_string())?;
    source
        .trim_end()
        .split("\n\n")
        .map(|block| parse_web_claim(block, &pattern))
        .collect()
}

fn parse_web_claim(block: &str, pattern: &Regex) -> Result<WebClaim, String> {
    let captures = pattern.captures(block).ok_or_else(|| {
        "The existing web evidence is not in OOMU’s canonical format.".to_string()
    })?;
    let decoded = |index| decode_markdown_line(&captures[index]);
    let subject = decoded(1);
    let claim = decoded(2);
    let organization = decoded(3);
    let source_title = decoded(5);
    let url = captures[6].to_string();
    let profile = authority_profile_for_url(&url).ok_or_else(|| {
        "The existing web evidence no longer uses an approved official source.".to_string()
    })?;
    let class = recorded_authority_class(&captures[4]);
    if organization != profile.organization || class != profile_authority_class(profile.class) {
        return Err(
            "The existing web evidence authority no longer matches its official source."
                .to_string(),
        );
    }
    let date_evidence_type = recorded_date_type(&captures[8]);
    let authority = SourceAuthority {
        profile_id: profile.id.to_string(),
        organization,
        class,
    };
    let effective_date = captures[7].to_string();
    let evidence_digest = captures[10].to_string();
    if evidence_digest
        != web_claim_evidence_digest(
            &subject,
            &claim,
            &source_title,
            &authority,
            &effective_date,
            date_evidence_type,
            &url,
        )
    {
        return Err("The existing web evidence digest does not match its claim.".to_string());
    }
    Ok(WebClaim {
        subject,
        claim,
        source_title,
        authority,
        effective_date,
        date_evidence_type,
        url,
        accessed_at: captures[9].to_string(),
        evidence_digest,
    })
}

fn recorded_authority_class(value: &str) -> SourceAuthorityClass {
    match value {
        "government" => SourceAuthorityClass::Government,
        "intergovernmental" => SourceAuthorityClass::Intergovernmental,
        "registered first-party" => SourceAuthorityClass::RegisteredFirstParty,
        _ => unreachable!("regex restricts authority class"),
    }
}

fn profile_authority_class(value: AuthorityClass) -> SourceAuthorityClass {
    match value {
        AuthorityClass::Government => SourceAuthorityClass::Government,
        AuthorityClass::Intergovernmental => SourceAuthorityClass::Intergovernmental,
        AuthorityClass::RegisteredFirstParty => SourceAuthorityClass::RegisteredFirstParty,
    }
}

fn recorded_date_type(value: &str) -> DateEvidenceType {
    match value {
        "publication date" => DateEvidenceType::PublicationDate,
        "release date" => DateEvidenceType::ReleaseDate,
        "observation date" => DateEvidenceType::ObservationDate,
        "updated date" => DateEvidenceType::UpdatedDate,
        _ => unreachable!("regex restricts date evidence type"),
    }
}

fn parse_research_gaps(source: &str) -> Result<Vec<ResearchGap>, String> {
    let gap_text = source.trim_end();
    if gap_text == "Every required research subject was qualified." {
        return Ok(Vec::new());
    }
    let pattern = Regex::new(
        r"^- \*\*(.+?):\*\* (evidence unavailable|network unavailable) after (\d+) bounded attempt\(s\) and (\d+) fetched page\(s\)\. No claim from this subject informed the recommendation\.$",
    )
    .map_err(|error| error.to_string())?;
    gap_text
        .lines()
        .map(|line| parse_research_gap(line, &pattern))
        .collect()
}

fn parse_research_gap(line: &str, pattern: &Regex) -> Result<ResearchGap, String> {
    let captures = pattern.captures(line).ok_or_else(|| {
        "The existing research gap is not in OOMU’s canonical format.".to_string()
    })?;
    Ok(ResearchGap {
        subject: decode_markdown_line(&captures[1]),
        reason: recorded_gap_reason(&captures),
        attempt_count: captures[3]
            .parse()
            .map_err(|_| "The existing research attempt count is invalid.".to_string())?,
        page_count: captures[4]
            .parse()
            .map_err(|_| "The existing research page count is invalid.".to_string())?,
    })
}

fn recorded_gap_reason(captures: &Captures<'_>) -> ResearchGapReason {
    match &captures[2] {
        "evidence unavailable" => ResearchGapReason::EvidenceUnavailable,
        "network unavailable" => ResearchGapReason::NetworkUnavailable,
        _ => unreachable!("regex restricts research gap reason"),
    }
}

fn decode_markdown_line(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some('\\' | '*' | '_')) {
            decoded.push(characters.next().expect("peeked Markdown escape"));
        } else {
            decoded.push(character);
        }
    }
    decoded
}
