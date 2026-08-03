use std::{
    env, fs,
    path::{Path, PathBuf},
};

const RELEASE_DIR: &str = "release/pre_alpha";
const DOCS_DIR: &str = "release/pre_alpha/docs";
const ICONS_DIR: &str = "release/pre_alpha/icons";
const SCHEMAS_DIR: &str = "release/pre_alpha/schemas";

fn main() {
    if let Err(error) = stage_pre_alpha() {
        eprintln!("Beta staging failed: {error}");
        std::process::exit(1);
    }
}

fn stage_pre_alpha() -> Result<(), String> {
    ensure_directories()?;
    stage_docs()?;
    stage_icons()?;
    stage_schemas()?;
    println!(
        "Beta support files staged at {RELEASE_DIR}. This command does not create release proof; run `npm run build:prod` to build, sanitize, sign, notarize, staple, and manifest a candidate."
    );
    Ok(())
}

fn ensure_directories() -> Result<(), String> {
    for dir in [RELEASE_DIR, DOCS_DIR, ICONS_DIR, SCHEMAS_DIR] {
        fs::create_dir_all(dir).map_err(|error| format!("failed to create {dir}: {error}"))?;
    }
    Ok(())
}

fn stage_docs() -> Result<(), String> {
    let doc_root = env::var_os("OOMU_RELEASE_DOCS_SOURCE_DIR")
        .map(PathBuf::from)
        .ok_or(
            "Set OOMU_RELEASE_DOCS_SOURCE_DIR to the directory containing OOMU_HLD_ROADMAP.md and OOMU_REQUIREMENTS.md.",
        )?;
    let sources = [
        (
            doc_root.join("OOMU_HLD_ROADMAP.md"),
            Path::new(DOCS_DIR).join("OOMU_HLD_ROADMAP.md"),
        ),
        (
            doc_root.join("OOMU_REQUIREMENTS.md"),
            Path::new(DOCS_DIR).join("OOMU_REQUIREMENTS.md"),
        ),
    ];
    copy_all(&sources)
}

fn stage_icons() -> Result<(), String> {
    let sources = [
        (
            PathBuf::from("src-tauri/icons/icon.png"),
            Path::new(ICONS_DIR).join("icon.png"),
        ),
        (
            PathBuf::from("src/app/favicon.ico"),
            Path::new(ICONS_DIR).join("favicon.ico"),
        ),
    ];
    copy_all(&sources)
}

fn stage_schemas() -> Result<(), String> {
    let sources = [
        (
            PathBuf::from("src-tauri/gen/schemas/desktop-schema.json"),
            Path::new(SCHEMAS_DIR).join("desktop-schema.json"),
        ),
        (
            PathBuf::from("src-tauri/gen/schemas/macOS-schema.json"),
            Path::new(SCHEMAS_DIR).join("macOS-schema.json"),
        ),
    ];
    copy_all(&sources)
}

fn copy_all(sources: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    for (source, destination) in sources {
        fs::copy(source, destination).map_err(|error| {
            format!(
                "failed to stage {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}
