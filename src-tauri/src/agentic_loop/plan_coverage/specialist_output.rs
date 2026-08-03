use super::*;

pub(super) fn markdown_output(objective: &str, semantic_cues: &[&str]) -> Option<String> {
    let candidates = objective_output_file_references(objective)
        .into_iter()
        .filter(|evidence| {
            Path::new(&evidence.path).is_absolute()
                && file_format(&evidence.path).as_deref() == Some("md")
        })
        .collect::<Vec<_>>();
    if let [candidate] = candidates.as_slice() {
        return Some(normalize_path(&candidate.path));
    }
    let lowered = objective.to_ascii_lowercase();
    let scored = candidates
        .into_iter()
        .map(|candidate| {
            let (start, end) = clause_bounds(&lowered, candidate.start, candidate.end);
            let clause = &lowered[start..end];
            let path = candidate.path.to_ascii_lowercase();
            let score = semantic_cues
                .iter()
                .map(|cue| usize::from(clause.contains(cue)) * 2 + usize::from(path.contains(cue)))
                .sum::<usize>();
            (score, normalize_path(&candidate.path))
        })
        .collect::<Vec<_>>();
    let best = scored.iter().map(|(score, _)| *score).max()?;
    (best > 0 && scored.iter().filter(|(score, _)| *score == best).count() == 1).then(|| {
        scored
            .into_iter()
            .find(|(score, _)| *score == best)
            .expect("unique specialist Markdown output")
            .1
    })
}
