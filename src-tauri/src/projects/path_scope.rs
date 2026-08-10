use crate::db::PersistenceEngine;
use rusqlite::params;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) fn single_active_project_root(
    engine: &PersistenceEngine,
    project_id: &str,
) -> Result<PathBuf, String> {
    let local_roots = active_project_roots_for_kind(engine, project_id, "local_folder")?;
    let roots = if local_roots.is_empty() {
        active_project_roots_for_kind(engine, project_id, "knowledge_directory")?
    } else {
        local_roots
    };
    match roots.len() {
        0 => Err(
            "This Project has no available approved folder. Open the Project and choose its folder."
                .to_string(),
        ),
        1 => Ok(roots.into_iter().next().expect("one checked Project root")),
        _ => Err(
            "This Project has more than one approved folder. Choose one Project folder before this work runs."
                .to_string(),
        ),
    }
}

pub(crate) fn active_project_evidence_roots(
    engine: &PersistenceEngine,
    project_id: &str,
) -> Result<Vec<PathBuf>, String> {
    let local_roots = active_project_roots_for_kind(engine, project_id, "local_folder")?;
    if !local_roots.is_empty() {
        return Ok(local_roots.into_iter().collect());
    }
    let roots = active_project_roots_for_kind(engine, project_id, "knowledge_directory")?;
    if roots.is_empty() {
        return Err(
            "This Project has no available approved folder. Open the Project and choose its folder."
                .to_string(),
        );
    }
    Ok(roots.into_iter().collect())
}

fn active_project_roots_for_kind(
    engine: &PersistenceEngine,
    project_id: &str,
    source_kind: &str,
) -> Result<BTreeSet<PathBuf>, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT canonical_path FROM project_sources WHERE project_id=?1 AND source_kind=?2 AND grant_state='active' ORDER BY canonical_path",
        )
        .map_err(|error| error.to_string())?;
    let stored = statement
        .query_map(params![project_id, source_kind], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let mut roots = BTreeSet::new();
    for value in stored {
        let path = PathBuf::from(value);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            "The approved Project folder is unavailable. Open the Project and choose the folder again."
                .to_string()
        })?;
        let canonical = fs::canonicalize(&path).map_err(|_| {
            "The approved Project folder is unavailable. Open the Project and choose the folder again."
                .to_string()
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() || canonical != path {
            return Err(
                "The approved Project folder identity changed. Choose the folder again before this work runs."
                    .to_string(),
            );
        }
        roots.insert(canonical);
    }
    Ok(roots)
}

pub(crate) fn resolve_project_output_path(root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(raw_path.trim());
    if requested.as_os_str().is_empty()
        || requested
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(
            "The scheduled file destination must stay inside the approved Project folder."
                .to_string(),
        );
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let resolved = resolve_missing_suffix(&candidate)?;
    if resolved == root || !resolved.starts_with(root) || resolved.file_name().is_none() {
        return Err(
            "The scheduled file destination must stay inside the approved Project folder."
                .to_string(),
        );
    }
    match fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            "The scheduled file destination must be a real file or a new file inside the Project."
                .to_string(),
        ),
        Ok(_) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(resolved),
        Err(_) => Err("The scheduled file destination could not be inspected safely.".to_string()),
    }
}

fn resolve_missing_suffix(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(
                        "The scheduled file destination contains a symbolic link.".to_string()
                    );
                }
                if ancestor != path && !metadata.is_dir() {
                    return Err(
                        "The scheduled file destination has a parent that is not a folder."
                            .to_string(),
                    );
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = ancestor.file_name().ok_or_else(|| {
                    "The scheduled file destination has no safe parent folder.".to_string()
                })?;
                missing.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| {
                    "The scheduled file destination has no safe parent folder.".to_string()
                })?;
            }
            Err(_) => {
                return Err(
                    "The scheduled file destination could not be inspected safely.".to_string(),
                )
            }
        }
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(|_| {
        "The scheduled file destination has no available parent folder.".to_string()
    })?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
