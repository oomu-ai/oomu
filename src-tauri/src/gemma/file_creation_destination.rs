use std::{
    env,
    path::{Path, PathBuf},
};

pub(super) fn inferred_file_destination(
    objective: &str,
    lowered: &str,
    format: &str,
    content: Option<&str>,
) -> Option<String> {
    explicit_file_destination(objective, format).or_else(|| {
        let folder = requested_standard_user_folder(lowered).unwrap_or("Downloads");
        let home = env::var_os("HOME").map(PathBuf::from)?;
        let filename = inferred_file_stem(content, format);
        Some(
            home.join(folder)
                .join(format!("{filename}.{format}"))
                .to_string_lossy()
                .to_string(),
        )
    })
}

fn explicit_file_destination(objective: &str, format: &str) -> Option<String> {
    let lowered = objective.to_ascii_lowercase();
    let suffix = format!(".{format}");
    let end = lowered.rfind(&suffix)? + suffix.len();
    let prefix = &objective[..end];
    let lowered_prefix = &lowered[..end];
    let start = ["~/", "/users/", "/volumes/", "/private/", "/tmp/"]
        .iter()
        .flat_map(|marker| lowered_prefix.match_indices(marker).map(|(index, _)| index))
        .filter(|index| absolute_destination_starts_at(prefix, *index))
        .max()?;
    let candidate = prefix[start..]
        .trim_matches(|character: char| matches!(character, '"' | '\'' | '`' | ' '))
        .to_string();
    normalize_absolute_file_destination(&candidate)
}

fn absolute_destination_starts_at(value: &str, index: usize) -> bool {
    index == 0
        || value[..index].chars().next_back().is_some_and(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.' | '~')
        })
}

fn normalize_absolute_file_destination(candidate: &str) -> Option<String> {
    if let Some(relative) = candidate.strip_prefix("~/") {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(relative).to_string_lossy().to_string());
    }
    Path::new(candidate)
        .is_absolute()
        .then(|| candidate.to_string())
}

fn requested_standard_user_folder(lowered: &str) -> Option<&'static str> {
    [
        ("downloads", "Downloads"),
        ("download", "Downloads"),
        ("documents", "Documents"),
        ("desktop", "Desktop"),
    ]
    .into_iter()
    .filter_map(|(alias, canonical)| {
        lowered.match_indices(alias).find_map(|(index, _)| {
            let before = lowered[..index].chars().next_back();
            let after = lowered[index + alias.len()..].chars().next();
            (before.is_none_or(|character| !character.is_ascii_alphanumeric())
                && after.is_none_or(|character| !character.is_ascii_alphanumeric()))
            .then_some((index, canonical))
        })
    })
    .min_by_key(|(index, _)| *index)
    .map(|(_, folder)| folder)
}

fn inferred_file_stem(content: Option<&str>, format: &str) -> String {
    let mut stem = String::new();
    let mut pending_separator = false;
    for character in content.unwrap_or_default().trim().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !stem.is_empty() {
                stem.push('_');
            }
            stem.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else if !stem.is_empty() {
            pending_separator = true;
        }
        if stem.len() >= 64 {
            break;
        }
    }
    while stem.ends_with('_') {
        stem.pop();
    }
    if !stem.is_empty() {
        return stem;
    }
    match format {
        "xlsx" | "xls" | "csv" => "spreadsheet".to_string(),
        "pptx" => "presentation".to_string(),
        _ => "document".to_string(),
    }
}
