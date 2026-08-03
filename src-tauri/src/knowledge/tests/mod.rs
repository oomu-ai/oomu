use super::*;

fn temp_knowledge_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oomu-knowledge-{label}-{}", random_grant_id()))
}

mod evidence;
mod ingestion;
mod retrieval;
mod sync;
