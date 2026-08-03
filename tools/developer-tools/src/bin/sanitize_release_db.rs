use std::path::PathBuf;

fn main() {
    let db_path = match std::env::args_os().nth(1) {
        Some(path) => PathBuf::from(path),
        None => {
            eprintln!("Usage: sanitize_release_db <path-to-oomu_state.sqlite>");
            std::process::exit(2);
        }
    };

    match oomu_lib::db::sanitize_release_database_at(&db_path) {
        Ok(report) => {
            println!("Sanitized release database: {}", report.path.display());
            for purge in report.purged_tables {
                println!("  {}: {} rows deleted", purge.table, purge.rows_deleted);
            }
        }
        Err(error) => {
            eprintln!("Release database sanitation failed: {error}");
            std::process::exit(1);
        }
    }
}
