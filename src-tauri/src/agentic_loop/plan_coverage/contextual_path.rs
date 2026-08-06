use std::path::Path;

pub(in crate::agentic_loop) fn requests_contextual_path_grounding(objective: &str) -> bool {
    objective.to_ascii_lowercase().contains("decision pack")
        || super::requests_evidence_bound_decision_pack(objective)
        || (!super::objective_input_file_references(objective).is_empty()
            && !super::objective_output_file_references(objective).is_empty())
        || super::objective_output_file_references(objective)
            .iter()
            .any(|reference| {
                let path = Path::new(&reference.path);
                !path.is_absolute()
                    && path
                        .parent()
                        .is_some_and(|parent| !parent.as_os_str().is_empty())
            })
}
