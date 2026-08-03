//! Acyclic, serializable authority shared by planning, search, and decision packs.

use serde::{Deserialize, Serialize};

pub(crate) const RESEARCH_POLICY_VERSION: u8 = 1;
const MAX_RESEARCH_SUBJECTS: usize = 2;
const MAX_QUERY_ALTERNATIVES_PER_SUBJECT: usize = 3;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResearchRequirement {
    AnyOf,
    AllOf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResearchSubject {
    Fuel,
    Freight,
}

impl ResearchSubject {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fuel => "fuel",
            Self::Freight => "freight",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResearchQueryAlternative {
    pub(crate) query: String,
    pub(crate) authority_profile: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResearchSubjectPolicy {
    pub(crate) subject: ResearchSubject,
    pub(crate) query_alternatives: Vec<ResearchQueryAlternative>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResearchPolicy {
    pub(crate) version: u8,
    pub(crate) requirement: ResearchRequirement,
    pub(crate) minimum_satisfied_subjects: usize,
    pub(crate) subjects: Vec<ResearchSubjectPolicy>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityClass {
    Government,
    Intergovernmental,
    RegisteredFirstParty,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AuthorityProfile {
    pub(crate) id: &'static str,
    pub(crate) organization: &'static str,
    pub(crate) class: AuthorityClass,
    pub(crate) hosts: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SignedResearchAuthority {
    pub(crate) profile: AuthorityProfile,
    pub(crate) direct_context_urls: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
struct RegisteredDirectContext {
    query: &'static str,
    authority_profile: &'static str,
    urls: &'static [&'static str],
}

const US_EIA: AuthorityProfile = AuthorityProfile {
    id: "usEnergyInformationAdministration",
    organization: "U.S. Energy Information Administration",
    class: AuthorityClass::Government,
    hosts: &["eia.gov"],
};
const US_BTS: AuthorityProfile = AuthorityProfile {
    id: "usBureauTransportationStatistics",
    organization: "U.S. Bureau of Transportation Statistics",
    class: AuthorityClass::Government,
    hosts: &["bts.gov"],
};
const US_BLS: AuthorityProfile = AuthorityProfile {
    id: "usBureauLaborStatistics",
    organization: "U.S. Bureau of Labor Statistics",
    class: AuthorityClass::Government,
    hosts: &["bls.gov"],
};
const US_DOT: AuthorityProfile = AuthorityProfile {
    id: "usDepartmentTransportation",
    organization: "U.S. Department of Transportation",
    class: AuthorityClass::Government,
    hosts: &["transportation.gov"],
};
const OECD: AuthorityProfile = AuthorityProfile {
    id: "organisationEconomicCooperationDevelopment",
    organization: "Organisation for Economic Co-operation and Development",
    class: AuthorityClass::Intergovernmental,
    hosts: &["oecd.org"],
};
const PORT_OF_LOS_ANGELES: AuthorityProfile = AuthorityProfile {
    id: "portOfLosAngeles",
    organization: "Port of Los Angeles",
    class: AuthorityClass::RegisteredFirstParty,
    hosts: &["portoflosangeles.org"],
};

const AUTHORITY_PROFILES: [AuthorityProfile; 6] =
    [US_EIA, US_BTS, US_BLS, US_DOT, OECD, PORT_OF_LOS_ANGELES];

const EIA_FUEL_QUERY: &str = "site:eia.gov diesel fuel release date weekly prices";
const BTS_FUEL_QUERY: &str = "site:bts.gov diesel fuel transportation statistics latest";
const BTS_FREIGHT_TSI_QUERY: &str = "site:bts.gov freight transportation services index latest";
const BTS_FREIGHT_PPI_QUERY: &str =
    "site:bts.gov transportation producer price index freight latest";
const DOT_FREIGHT_QUERY: &str = "site:transportation.gov freight conditions update";

const FUEL_QUERIES: [(&str, &str); 2] = [
    (EIA_FUEL_QUERY, "usEnergyInformationAdministration"),
    (BTS_FUEL_QUERY, "usBureauTransportationStatistics"),
];

const FREIGHT_QUERIES: [(&str, &str); 3] = [
    (BTS_FREIGHT_TSI_QUERY, "usBureauTransportationStatistics"),
    (BTS_FREIGHT_PPI_QUERY, "usBureauTransportationStatistics"),
    (DOT_FREIGHT_QUERY, "usDepartmentTransportation"),
];

// These are durable first-party landing or taxonomy pages, never dated release
// URLs. They are unlocked only by the exact registered query in a verified,
// signed decision-pack policy; the normal claim and freshness checks still
// decide whether retrieved content is usable evidence.
const REGISTERED_DIRECT_CONTEXTS: [RegisteredDirectContext; 5] = [
    RegisteredDirectContext {
        query: EIA_FUEL_QUERY,
        authority_profile: "usEnergyInformationAdministration",
        urls: &["https://www.eia.gov/petroleum/gasdiesel/"],
    },
    RegisteredDirectContext {
        query: BTS_FUEL_QUERY,
        authority_profile: "usBureauTransportationStatistics",
        urls: &["https://www.bts.gov/tags/fuel"],
    },
    RegisteredDirectContext {
        query: BTS_FREIGHT_TSI_QUERY,
        authority_profile: "usBureauTransportationStatistics",
        urls: &["https://www.bts.gov/tags/transportation-services-index"],
    },
    RegisteredDirectContext {
        query: BTS_FREIGHT_PPI_QUERY,
        authority_profile: "usBureauTransportationStatistics",
        urls: &["https://www.bts.gov/taxonomy/term/426"],
    },
    RegisteredDirectContext {
        query: DOT_FREIGHT_QUERY,
        authority_profile: "usDepartmentTransportation",
        urls: &["https://www.transportation.gov/tags/freight"],
    },
];

pub(crate) fn compile_research_policy(objective: &str) -> Result<ResearchPolicy, String> {
    let lowered = objective.to_ascii_lowercase();
    let mut subjects = Vec::new();
    for subject in [ResearchSubject::Fuel, ResearchSubject::Freight] {
        if contains_subject(&lowered, subject) {
            subjects.push(ResearchSubjectPolicy {
                subject,
                query_alternatives: registered_queries(subject),
            });
        }
    }
    if subjects.is_empty() {
        return Err(
            "The independent public research topic was not bounded enough to compile safely."
                .to_string(),
        );
    }
    let requirement = if subjects.len() > 1 && objective_uses_or(&lowered) {
        ResearchRequirement::AnyOf
    } else {
        ResearchRequirement::AllOf
    };
    let minimum_satisfied_subjects = match requirement {
        ResearchRequirement::AnyOf => 1,
        ResearchRequirement::AllOf => subjects.len(),
    };
    Ok(ResearchPolicy {
        version: RESEARCH_POLICY_VERSION,
        requirement,
        minimum_satisfied_subjects,
        subjects,
    })
}

pub(crate) fn validate_research_policy(policy: &ResearchPolicy) -> Result<(), String> {
    if policy.version != RESEARCH_POLICY_VERSION
        || policy.subjects.is_empty()
        || policy.subjects.len() > MAX_RESEARCH_SUBJECTS
    {
        return Err(
            "Decision-pack research policy version or subject count is invalid.".to_string(),
        );
    }
    let expected_minimum = match policy.requirement {
        ResearchRequirement::AnyOf => 1,
        ResearchRequirement::AllOf => policy.subjects.len(),
    };
    if policy.minimum_satisfied_subjects != expected_minimum {
        return Err(
            "Decision-pack research policy threshold does not match its requirement.".to_string(),
        );
    }
    let mut seen = std::collections::HashSet::new();
    for subject in &policy.subjects {
        if !seen.insert(subject.subject)
            || subject.query_alternatives.is_empty()
            || subject.query_alternatives.len() > MAX_QUERY_ALTERNATIVES_PER_SUBJECT
            || subject.query_alternatives != registered_queries(subject.subject)
        {
            return Err(
                "Decision-pack research policy contains an unregistered or modified query."
                    .to_string(),
            );
        }
    }
    Ok(())
}

pub(crate) fn policy_matches_objective(
    objective: &str,
    policy: &ResearchPolicy,
) -> Result<(), String> {
    let expected = compile_research_policy(objective)?;
    if policy != &expected {
        return Err(
            "Decision-pack research policy changed the approved subjects or Boolean requirement."
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
fn approved_registry_query_for_objective(objective: &str, query: &str) -> bool {
    compile_research_policy(objective)
        .ok()
        .is_some_and(|policy| {
            policy.subjects.iter().any(|subject| {
                subject
                    .query_alternatives
                    .iter()
                    .any(|alternative| alternative.query == query)
            })
        })
}

pub(crate) fn policy_digest(policy: &ResearchPolicy) -> Result<String, String> {
    validate_research_policy(policy)?;
    serde_json::to_vec(policy)
        .map(|encoded| crate::foundation::digest::sha256_hex(&encoded))
        .map_err(|error| error.to_string())
}

pub(crate) fn signed_policy_authority_for_query(
    objective: &str,
    policy: &ResearchPolicy,
    expected_digest: &str,
    query: &str,
) -> Option<AuthorityProfile> {
    signed_policy_authority_binding_for_query(objective, policy, expected_digest, query)
        .map(|binding| binding.profile)
}

pub(crate) fn signed_policy_authority_binding_for_query(
    objective: &str,
    policy: &ResearchPolicy,
    expected_digest: &str,
    query: &str,
) -> Option<SignedResearchAuthority> {
    policy_matches_objective(objective, policy).ok()?;
    (policy_digest(policy).ok()?.as_str() == expected_digest).then_some(())?;
    let alternative = policy
        .subjects
        .iter()
        .flat_map(|subject| &subject.query_alternatives)
        .find(|alternative| alternative.query == query)?;
    let profile = authority_profile(&alternative.authority_profile)?;
    let direct_context_urls = REGISTERED_DIRECT_CONTEXTS
        .iter()
        .find(|context| {
            context.query == alternative.query
                && context.authority_profile == alternative.authority_profile
        })
        .map(|context| context.urls)
        .unwrap_or_default();
    direct_context_urls
        .iter()
        .all(|url| profile_allows_url(profile, url))
        .then_some(SignedResearchAuthority {
            profile,
            direct_context_urls,
        })
}

pub(crate) fn authority_profile(id: &str) -> Option<AuthorityProfile> {
    AUTHORITY_PROFILES
        .iter()
        .find(|profile| profile.id == id)
        .copied()
}

pub(crate) fn authority_profile_for_url(value: &str) -> Option<AuthorityProfile> {
    AUTHORITY_PROFILES
        .iter()
        .find(|profile| profile_allows_url(**profile, value))
        .copied()
}

pub(crate) fn profile_allows_url(profile: AuthorityProfile, value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    profile
        .hosts
        .iter()
        .any(|approved| host == *approved || host.ends_with(&format!(".{approved}")))
}

fn registered_queries(subject: ResearchSubject) -> Vec<ResearchQueryAlternative> {
    let values: &[(&str, &str)] = match subject {
        ResearchSubject::Fuel => &FUEL_QUERIES,
        ResearchSubject::Freight => &FREIGHT_QUERIES,
    };
    values
        .iter()
        .map(|(query, authority_profile)| ResearchQueryAlternative {
            query: (*query).to_string(),
            authority_profile: (*authority_profile).to_string(),
        })
        .collect()
}

fn contains_subject(lowered: &str, subject: ResearchSubject) -> bool {
    lowered
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == subject.as_str())
}

fn objective_uses_or(lowered: &str) -> bool {
    let tokens = lowered
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens.windows(3).any(|window| {
        matches!(
            window,
            ["fuel", "or", "freight"] | ["freight", "or", "fuel"]
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_any_of_and_all_of_semantics() {
        let any = compile_research_policy("research official fuel or freight conditions").unwrap();
        assert_eq!(any.requirement, ResearchRequirement::AnyOf);
        assert_eq!(any.minimum_satisfied_subjects, 1);
        let all = compile_research_policy("research official fuel and freight conditions").unwrap();
        assert_eq!(all.requirement, ResearchRequirement::AllOf);
        assert_eq!(all.minimum_satisfied_subjects, 2);
        let long_clause = compile_research_policy(
            "research official fuel conditions and compare carrier options or official freight conditions",
        )
        .unwrap();
        assert_eq!(long_clause.requirement, ResearchRequirement::AllOf);
    }

    #[test]
    fn registry_rejects_mutation_and_lookalike_hosts() {
        let mut policy = compile_research_policy("research official freight conditions").unwrap();
        policy.subjects[0].query_alternatives[0]
            .query
            .push_str(" supplier secret");
        assert!(validate_research_policy(&policy).is_err());
        let bts = authority_profile("usBureauTransportationStatistics").unwrap();
        assert!(profile_allows_url(
            bts,
            "https://www.bts.gov/newsroom/freight"
        ));
        assert!(!profile_allows_url(
            bts,
            "https://www.bts.gov.example.com/newsroom/freight"
        ));
    }

    #[test]
    fn objective_cannot_authorize_an_unrequested_subject() {
        let freight_query = FREIGHT_QUERIES[0].0;
        assert!(!approved_registry_query_for_objective(
            "research official fuel conditions",
            freight_query
        ));
    }

    #[test]
    fn every_query_is_bounded_to_a_registered_authority_profile() {
        let policy =
            compile_research_policy("research official fuel and freight conditions").unwrap();
        for subject in policy.subjects {
            for alternative in subject.query_alternatives {
                assert!(authority_profile(&alternative.authority_profile).is_some());
            }
        }
    }

    #[test]
    fn every_signed_query_has_host_bound_stable_first_party_context() {
        let objective = "research official fuel and freight conditions";
        let policy = compile_research_policy(objective).unwrap();
        let digest = policy_digest(&policy).unwrap();
        for alternative in policy
            .subjects
            .iter()
            .flat_map(|subject| &subject.query_alternatives)
        {
            let binding = signed_policy_authority_binding_for_query(
                objective,
                &policy,
                &digest,
                &alternative.query,
            )
            .expect("registered signed query should resolve an authority binding");
            assert!(!binding.direct_context_urls.is_empty());
            assert!(binding
                .direct_context_urls
                .iter()
                .all(|url| profile_allows_url(binding.profile, url)));
            assert!(binding
                .direct_context_urls
                .iter()
                .all(|url| !url.contains("/newsroom/")));
        }
    }
}
