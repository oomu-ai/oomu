use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    p0_contracts::{EvidenceClass, ProjectId, TaskRunId},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Component, Path},
};

fn analysis_id() -> String {
    let mut bytes = [0u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("analysis_{}", hex::encode(bytes))
}

fn parse_row(line: &str, delimiter: char) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek() == Some(&'"') {
                value.push('"');
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if ch == delimiter && !quoted {
            values.push(value.trim().to_string());
            value.clear();
        } else {
            value.push(ch)
        }
    }
    if quoted {
        return Err("The data file has an unfinished quoted value.".into());
    }
    values.push(value.trim().to_string());
    Ok(values)
}

fn analyze_text(raw: &str) -> Result<(String, Value, Value, Value), String> {
    let mut lines = raw.lines().filter(|line| !line.trim().is_empty());
    let header_line = lines
        .next()
        .ok_or_else(|| "The data file is empty.".to_string())?;
    let delimiter = if header_line.matches('\t').count() > header_line.matches(',').count() {
        '\t'
    } else {
        ','
    };
    let headers = parse_row(header_line, delimiter)?;
    if headers.is_empty() || headers.len() > 100 {
        return Err("The data file needs between 1 and 100 columns.".into());
    }
    let mut rows = Vec::new();
    for line in lines.take(100_001) {
        if rows.len() >= 100_000 {
            return Err("This analysis is limited to 100,000 rows.".into());
        }
        let row = parse_row(line, delimiter)?;
        if row.len() != headers.len() {
            return Err(format!(
                "Row {} has a different number of columns.",
                rows.len() + 2
            ));
        }
        rows.push(row)
    }
    if rows.is_empty() {
        return Err("The data file has headings but no rows.".into());
    }
    let mut numeric = Vec::new();
    for (index, name) in headers.iter().enumerate() {
        let values = rows
            .iter()
            .filter_map(|row| row[index].parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .collect::<Vec<_>>();
        if values.len() * 10 >= rows.len() * 8 {
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            numeric.push(json!({"name":name,"count":values.len(),"minimum":min,"maximum":max,"average":mean,"values":values.into_iter().take(12).collect::<Vec<_>>() }))
        }
    }
    let answer = if numeric.is_empty() {
        format!("OOMU checked {} rows across {} columns. The file contains primarily labels or categories, so no numeric comparison was made.",rows.len(),headers.len())
    } else {
        let first = &numeric[0];
        format!("OOMU checked {} rows across {} columns. For {}, the average is {:.2}, ranging from {:.2} to {:.2}.",rows.len(),headers.len(),first["name"].as_str().unwrap_or("the first numeric column"),first["average"].as_f64().unwrap_or(0.0),first["minimum"].as_f64().unwrap_or(0.0),first["maximum"].as_f64().unwrap_or(0.0))
    };
    let table = json!({"headers":headers.iter().take(10).collect::<Vec<_>>(),"rows":rows.iter().take(20).map(|row|row.iter().take(10).collect::<Vec<_>>()).collect::<Vec<_>>(),"totalRows":rows.len(),"truncated":rows.len()>20||headers.len()>10});
    let chart = numeric
        .first()
        .map(|series| json!({"title":series["name"],"values":series["values"]}))
        .unwrap_or_else(|| json!({"title":Value::Null,"values":[]}));
    let method = json!({"steps":["Read the approved Project file","Checked every row for a consistent shape","Calculated numeric summaries","Checked the result before saving it"],"engine":"OOMU built-in table analysis","networkUsed":false,"dependencies":[],"randomnessUsed":false,"limits":{"rows":100000,"columns":100,"fileBytes":10485760}});
    Ok((answer, table, chart, method))
}

fn read_source(
    engine: &PersistenceEngine,
    request: &RunAnalysisRequest,
) -> Result<(String, String), String> {
    let relative = Path::new(&request.relative_path);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("The selected file is outside the approved Project folder.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let root:String=connection.query_row("SELECT canonical_path FROM project_sources WHERE project_id=?1 AND source_id=?2 AND grant_state='active'",params![request.project_id,request.source_id],|row|row.get(0)).map_err(|_|"The approved Project folder is unavailable.".to_string())?;
    let root = fs::canonicalize(root)
        .map_err(|_| "The approved Project folder is unavailable.".to_string())?;
    let candidate = fs::canonicalize(root.join(relative))
        .map_err(|_| "The selected data file is unavailable.".to_string())?;
    if !candidate.starts_with(&root)
        || candidate
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("The selected file failed Project containment checks.".into());
    }
    let metadata = candidate.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 10 * 1024 * 1024 {
        return Err("Choose a regular CSV or TSV file smaller than 10 MB.".into());
    }
    let extension = candidate
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "csv" | "tsv") {
        return Err("OOMU currently analyzes CSV and TSV files here.".into());
    }
    let raw = fs::read_to_string(&candidate)
        .map_err(|_| "The data file must be UTF-8 text.".to_string())?;
    Ok((
        candidate
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("Data file")
            .to_string(),
        raw,
    ))
}

pub(crate) fn run(
    engine: &PersistenceEngine,
    request: &RunAnalysisRequest,
) -> Result<AnalysisView, String> {
    ProjectId::parse(&request.project_id)?;
    TaskRunId::parse(&request.task_run_id)?;
    crate::tools::task_runtime::require_bound_task(
        engine,
        &request.task_run_id,
        &request.project_id,
    )?;
    let started = unix_time_ms_i64();
    let (_file_name, raw) = read_source(engine, request)?;
    let input_sha = sha256_hex(raw.as_bytes());
    let (answer, table, chart, method) = analyze_text(&raw)?;
    let environment =
        json!({"engine":"oomu_builtin_tabular_v1","locale":"C","network":"denied","seed":0});
    let environment_sha = sha256_hex(environment.to_string().as_bytes());
    let output = serde_json::to_string(&json!({"answer":answer,"table":table,"chart":chart}))
        .map_err(|e| e.to_string())?;
    let output_sha = sha256_hex(output.as_bytes());
    let id = analysis_id();
    let completed = unix_time_ms_i64();
    engine.open_connection().map_err(|e|e.to_string())?.execute("INSERT INTO analysis_runs (analysis_id,project_id,task_run_id,source_id,relative_path,state,answer,table_json,chart_json,method_json,input_sha256,output_sha256,environment_sha256,started_at_ms,completed_at_ms) VALUES (?1,?2,?3,?4,?5,'completed',?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![id,request.project_id,request.task_run_id,request.source_id,request.relative_path,answer,table.to_string(),chart.to_string(),method.to_string(),input_sha,output_sha,environment_sha,started,completed]).map_err(|e|e.to_string())?;
    crate::tools::task_runtime::record_event(
        engine,
        &request.task_run_id,
        "analysis.result_verified",
        EvidenceClass::VerifiedPostcondition,
        json!({"analysisId":id,"inputSha256":input_sha,"outputSha256":output_sha,"environmentSha256":environment_sha,"networkUsed":false,"mutationAttempts":0}),
    )?;
    get(engine, &id)
}

fn get(engine: &PersistenceEngine, id: &str) -> Result<AnalysisView, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    connection.query_row("SELECT analysis_id,project_id,task_run_id,relative_path,answer,table_json,chart_json,method_json,input_sha256,output_sha256,environment_sha256,completed_at_ms FROM analysis_runs WHERE analysis_id=?1",params![id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?,row.get::<_,i64>(11)?))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Analysis result was not found.".to_string()).and_then(|r|Ok(AnalysisView{analysis_id:r.0,project_id:r.1,task_run_id:r.2,file_name:Path::new(&r.3).file_name().and_then(|v|v.to_str()).unwrap_or("Data file").to_string(),answer:r.4,table:serde_json::from_str(&r.5).map_err(|e|e.to_string())?,chart:serde_json::from_str(&r.6).map_err(|e|e.to_string())?,method:serde_json::from_str(&r.7).map_err(|e|e.to_string())?,input_sha256:r.8,output_sha256:r.9,environment_sha256:r.10,completed_at_ms:r.11}))
}

pub(crate) fn list(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<Vec<AnalysisView>, String> {
    TaskRunId::parse(task_run_id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement=connection.prepare("SELECT analysis_id FROM analysis_runs WHERE task_run_id=?1 ORDER BY completed_at_ms DESC").map_err(|e|e.to_string())?;
    let ids = statement
        .query_map(params![task_run_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    drop(connection);
    ids.into_iter().map(|id| get(engine, &id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analysis_is_deterministic_and_rejects_uneven_rows() {
        let first = analyze_text("name,value\na,2\nb,4").unwrap();
        let second = analyze_text("name,value\na,2\nb,4").unwrap();
        assert_eq!(first.0, second.0);
        assert!(analyze_text("a,b\n1").is_err());
    }
}
