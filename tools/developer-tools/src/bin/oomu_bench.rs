use oomu_lib::{
    foundation::clock::unix_time_ms_i64 as unix_time_ms,
    foundation::digest::{sha256_file_hex, sha256_hex},
    gemma::{GemmaService, InferRequest},
    network_policy::{
        resolve_destination, revalidate_destination, validate_connected_peer, DestinationTransport,
        ResolvedDestinationClass,
    },
};
use reqwest::{redirect::Policy, Certificate, Identity};
use serde::Serialize;
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
    time::{Duration, Instant},
};
use sysinfo::System;

const MAX_PROMPT_BYTES: usize = 128 * 1024;
const MAX_RUNS: usize = 100;
const MAX_WARMUPS: usize = 20;
const MAX_GENERATED_TOKENS: usize = 16_384;
const NETWORK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const NETWORK_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct BenchConfig {
    input: PathBuf,
    output: PathBuf,
    model_id: String,
    runs: usize,
    warmups: usize,
    max_tokens: usize,
    endpoint: Option<String>,
    client_identity_pem: Option<PathBuf>,
    ca_pem: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    schema_version: &'static str,
    evidence_kind: &'static str,
    generated_at_ms: i64,
    build: BuildIdentity,
    machine: MachineProfile,
    input: InputIdentity,
    model: ModelIdentity,
    policy: RunPolicy,
    raw_model_samples: Vec<ModelSample>,
    model_statistics: ModelStatistics,
    network_measurement: NetworkMeasurement,
    report_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildIdentity {
    package_version: &'static str,
    executable_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineProfile {
    os_name: String,
    os_version: String,
    architecture: &'static str,
    cpu_brand: String,
    logical_cpu_count: usize,
    total_memory_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputIdentity {
    prompt_bytes: usize,
    prompt_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelIdentity {
    id: String,
    architecture: String,
    format: String,
    weights_bytes: u64,
    weights_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunPolicy {
    requested_runs: usize,
    completed_warmups: usize,
    max_generated_tokens: usize,
    warmup_outputs_discarded: bool,
    failures_included_in_error_rate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelSample {
    run_index: usize,
    wall_clock_ms: u128,
    time_to_first_token_ms: Option<u128>,
    generated_tokens: usize,
    tokens_per_second: Option<f64>,
    resident_memory_before_bytes: u64,
    resident_memory_after_bytes: u64,
    output_bytes: usize,
    output_sha256: Option<String>,
    service_status: Option<String>,
    error_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelStatistics {
    successful_runs: usize,
    failed_runs: usize,
    error_rate: f64,
    total_generated_tokens: usize,
    aggregate_tokens_per_second: Option<f64>,
    p50_wall_clock_ms: Option<u128>,
    p95_wall_clock_ms: Option<u128>,
    p50_time_to_first_token_ms: Option<u128>,
    p95_time_to_first_token_ms: Option<u128>,
    peak_resident_memory_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum NetworkMeasurement {
    NotRequested,
    Measured {
        destination: String,
        destination_binding: String,
        tls_mode: &'static str,
        raw_samples: Vec<NetworkSample>,
        successful_connections: usize,
        failed_connections: usize,
        error_rate: f64,
        p50_setup_ms: Option<u128>,
        p95_setup_ms: Option<u128>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NetworkSample {
    run_index: usize,
    setup_ms: u128,
    http_status: Option<u16>,
    peer_policy_verified: bool,
    redirect_rejected: bool,
    error_code: Option<String>,
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(true) => process::exit(1),
        Ok(false) => {}
        Err(error) => {
            eprintln!("oomu_bench_error={error}");
            process::exit(2);
        }
    }
}

async fn run() -> Result<bool, String> {
    let config = parse_config()?;
    let prompt = read_bounded(&config.input, MAX_PROMPT_BYTES)?;
    let prompt_text = String::from_utf8(prompt.clone())
        .map_err(|_| "Benchmark input must be UTF-8 text.".to_string())?;
    if prompt_text.trim().is_empty() {
        return Err("Benchmark input must not be empty.".to_string());
    }

    let manifest = oomu_lib::gemma::inspect_local_model(&config.model_id)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    if manifest.format != "gguf" || manifest.compatibility != "ready" {
        return Err("The selected model is not a ready GGUF runtime asset.".to_string());
    }
    let weights_path = find_single_gguf(Path::new(&manifest.path))?;
    let model = ModelIdentity {
        id: manifest.id.clone(),
        architecture: manifest.architecture,
        format: manifest.format,
        weights_bytes: manifest.weights_bytes,
        weights_sha256: hash_file(&weights_path)?,
    };

    let service = GemmaService::new_loading();
    service
        .prepare_model_sync(&manifest.id)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;

    for warmup_index in 0..config.warmups {
        let request = inference_request(
            &prompt_text,
            format!("benchmark-warmup-{warmup_index}"),
            config.max_tokens,
        );
        service
            .infer_model_sync(&manifest.id, request)
            .map_err(|error| format!("warmup_failed:{}", error.code))?;
    }

    let mut raw_model_samples = Vec::with_capacity(config.runs);
    for run_index in 0..config.runs {
        let rss_before = resident_memory_bytes();
        let started = Instant::now();
        let response = service.infer_model_sync(
            &manifest.id,
            inference_request(
                &prompt_text,
                format!("benchmark-measured-{run_index}"),
                config.max_tokens,
            ),
        );
        let wall_clock_ms = started.elapsed().as_millis();
        let rss_after = resident_memory_bytes();
        let sample = match response {
            Ok(response) => {
                let elapsed_seconds = (wall_clock_ms as f64 / 1_000.0).max(f64::EPSILON);
                ModelSample {
                    run_index,
                    wall_clock_ms,
                    time_to_first_token_ms: Some(response.time_to_first_token_ms),
                    generated_tokens: response.generated_token_count,
                    tokens_per_second: Some(
                        response.generated_token_count as f64 / elapsed_seconds,
                    ),
                    resident_memory_before_bytes: rss_before,
                    resident_memory_after_bytes: rss_after,
                    output_bytes: response.text.len(),
                    output_sha256: Some(sha256_hex(response.text.as_bytes())),
                    service_status: Some(format!("{:?}", response.service_status).to_lowercase()),
                    error_code: None,
                }
            }
            Err(error) => ModelSample {
                run_index,
                wall_clock_ms,
                time_to_first_token_ms: None,
                generated_tokens: 0,
                tokens_per_second: None,
                resident_memory_before_bytes: rss_before,
                resident_memory_after_bytes: rss_after,
                output_bytes: 0,
                output_sha256: None,
                service_status: None,
                error_code: Some(error.code.to_string()),
            },
        };
        raw_model_samples.push(sample);
    }
    service.shutdown();

    let model_statistics = model_statistics(&raw_model_samples);
    let network_measurement = match config.endpoint.as_deref() {
        Some(endpoint) => {
            measure_network(
                endpoint,
                config.runs,
                config.client_identity_pem.as_deref(),
                config.ca_pem.as_deref(),
            )
            .await?
        }
        None => NetworkMeasurement::NotRequested,
    };
    let network_failed = matches!(
        &network_measurement,
        NetworkMeasurement::Measured {
            failed_connections,
            ..
        } if *failed_connections > 0
    );

    let mut report = BenchmarkReport {
        schema_version: "1.0.0",
        evidence_kind: "real_component_benchmark",
        generated_at_ms: unix_time_ms(),
        build: BuildIdentity {
            package_version: env!("CARGO_PKG_VERSION"),
            executable_sha256: hash_file(&env::current_exe().map_err(|error| error.to_string())?)?,
        },
        machine: machine_profile(),
        input: InputIdentity {
            prompt_bytes: prompt.len(),
            prompt_sha256: sha256_hex(&prompt),
        },
        model,
        policy: RunPolicy {
            requested_runs: config.runs,
            completed_warmups: config.warmups,
            max_generated_tokens: config.max_tokens,
            warmup_outputs_discarded: true,
            failures_included_in_error_rate: true,
        },
        raw_model_samples,
        model_statistics,
        network_measurement,
        report_sha256: String::new(),
    };
    report.report_sha256 =
        sha256_hex(&serde_json::to_vec(&report).map_err(|error| error.to_string())?);
    write_new_report(&config.output, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );

    Ok(report.model_statistics.failed_runs > 0 || network_failed)
}

fn inference_request(prompt: &str, session_id: String, max_tokens: usize) -> InferRequest {
    let mut request = InferRequest::new(prompt);
    request.session_id = Some(session_id);
    request.max_tokens = Some(max_tokens);
    request
}

async fn measure_network(
    endpoint: &str,
    runs: usize,
    client_identity_pem: Option<&Path>,
    ca_pem: Option<&Path>,
) -> Result<NetworkMeasurement, String> {
    let destination = resolve_destination(endpoint, DestinationTransport::RemoteMcpHttp, None)
        .await
        .map_err(|error| error.message)?;
    if destination.destination_class() != ResolvedDestinationClass::Public {
        return Err("Network benchmark destination must be public HTTPS.".to_string());
    }
    let destination = revalidate_destination(&destination)
        .await
        .map_err(|error| error.message)?;
    let identity = client_identity_pem
        .map(|path| read_bounded(path, 1024 * 1024))
        .transpose()?
        .map(|pem| Identity::from_pem(&pem).map_err(|error| error.to_string()))
        .transpose()?;
    let ca = ca_pem
        .map(|path| read_bounded(path, 1024 * 1024))
        .transpose()?
        .map(|pem| Certificate::from_pem(&pem).map_err(|error| error.to_string()))
        .transpose()?;
    let tls_mode = if identity.is_some() { "mtls" } else { "tls" };
    let mut samples = Vec::with_capacity(runs);

    for run_index in 0..runs {
        let started = Instant::now();
        let current = match revalidate_destination(&destination).await {
            Ok(current) => current,
            Err(_) => {
                samples.push(NetworkSample {
                    run_index,
                    setup_ms: started.elapsed().as_millis(),
                    http_status: None,
                    peer_policy_verified: false,
                    redirect_rejected: false,
                    error_code: Some("destination_revalidation_failed".to_string()),
                });
                continue;
            }
        };
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(NETWORK_CONNECT_TIMEOUT)
            .read_timeout(NETWORK_CONNECT_TIMEOUT)
            .timeout(NETWORK_TOTAL_TIMEOUT)
            .resolve_to_addrs(current.host(), &current.resolved_socket_addresses());
        if let Some(identity) = identity.clone() {
            builder = builder.identity(identity);
        }
        if let Some(ca) = ca.clone() {
            builder = builder.add_root_certificate(ca);
        }
        let response = match builder.build() {
            Ok(client) => client.head(current.url().clone()).send().await,
            Err(_) => {
                samples.push(NetworkSample {
                    run_index,
                    setup_ms: started.elapsed().as_millis(),
                    http_status: None,
                    peer_policy_verified: false,
                    redirect_rejected: false,
                    error_code: Some("tls_client_build_failed".to_string()),
                });
                continue;
            }
        };
        let sample = match response {
            Ok(response) => {
                let peer_verified =
                    validate_connected_peer(&current, response.remote_addr()).is_ok();
                let redirect = response.status().is_redirection();
                NetworkSample {
                    run_index,
                    setup_ms: started.elapsed().as_millis(),
                    http_status: Some(response.status().as_u16()),
                    peer_policy_verified: peer_verified,
                    redirect_rejected: redirect,
                    error_code: (!peer_verified)
                        .then(|| "peer_policy_mismatch".to_string())
                        .or_else(|| redirect.then(|| "redirect_rejected".to_string())),
                }
            }
            Err(_) => NetworkSample {
                run_index,
                setup_ms: started.elapsed().as_millis(),
                http_status: None,
                peer_policy_verified: false,
                redirect_rejected: false,
                error_code: Some("connection_failed".to_string()),
            },
        };
        samples.push(sample);
    }

    let successful_connections = samples
        .iter()
        .filter(|sample| sample.error_code.is_none())
        .count();
    let failed_connections = samples.len().saturating_sub(successful_connections);
    let successful_setup = samples
        .iter()
        .filter(|sample| sample.error_code.is_none())
        .map(|sample| sample.setup_ms)
        .collect::<Vec<_>>();
    Ok(NetworkMeasurement::Measured {
        destination: destination.canonical_url().to_string(),
        destination_binding: destination.binding_fingerprint().to_string(),
        tls_mode,
        raw_samples: samples,
        successful_connections,
        failed_connections,
        error_rate: failed_connections as f64 / runs as f64,
        p50_setup_ms: percentile(&successful_setup, 50),
        p95_setup_ms: percentile(&successful_setup, 95),
    })
}

fn model_statistics(samples: &[ModelSample]) -> ModelStatistics {
    let successful = samples
        .iter()
        .filter(|sample| sample.error_code.is_none())
        .collect::<Vec<_>>();
    let successful_runs = successful.len();
    let failed_runs = samples.len().saturating_sub(successful_runs);
    let total_generated_tokens = successful
        .iter()
        .map(|sample| sample.generated_tokens)
        .sum::<usize>();
    let total_seconds = successful
        .iter()
        .map(|sample| sample.wall_clock_ms as f64 / 1_000.0)
        .sum::<f64>();
    let wall = successful
        .iter()
        .map(|sample| sample.wall_clock_ms)
        .collect::<Vec<_>>();
    let ttft = successful
        .iter()
        .filter_map(|sample| sample.time_to_first_token_ms)
        .collect::<Vec<_>>();
    ModelStatistics {
        successful_runs,
        failed_runs,
        error_rate: failed_runs as f64 / samples.len() as f64,
        total_generated_tokens,
        aggregate_tokens_per_second: (total_seconds > 0.0)
            .then_some(total_generated_tokens as f64 / total_seconds),
        p50_wall_clock_ms: percentile(&wall, 50),
        p95_wall_clock_ms: percentile(&wall, 95),
        p50_time_to_first_token_ms: percentile(&ttft, 50),
        p95_time_to_first_token_ms: percentile(&ttft, 95),
        peak_resident_memory_bytes: samples
            .iter()
            .flat_map(|sample| {
                [
                    sample.resident_memory_before_bytes,
                    sample.resident_memory_after_bytes,
                ]
            })
            .max()
            .unwrap_or_default(),
    }
}

fn parse_config() -> Result<BenchConfig, String> {
    if env::args().any(|argument| {
        matches!(
            argument.as_str(),
            "--connection-simulations" | "--taskflow-harness"
        )
    }) {
        return Err(
            "Simulation and fixture harness modes were removed; provide real benchmark inputs."
                .to_string(),
        );
    }
    let input = required_path("--input")?;
    let output = required_path("--output")?;
    let model_id = arg_value("--model")
        .ok_or_else(|| "--model is required and must name a configured local model.".to_string())?;
    let runs = bounded_usize("--runs", 3, 1, MAX_RUNS)?;
    let warmups = bounded_usize("--warmups", 1, 0, MAX_WARMUPS)?;
    let max_tokens = bounded_usize("--max-tokens", 256, 1, MAX_GENERATED_TOKENS)?;
    let endpoint = arg_value("--endpoint");
    let client_identity_pem = optional_path("--client-identity-pem");
    let ca_pem = optional_path("--ca-pem");
    if client_identity_pem.is_some() && endpoint.is_none() {
        return Err("--client-identity-pem requires --endpoint.".to_string());
    }
    if ca_pem.is_some() && endpoint.is_none() {
        return Err("--ca-pem requires --endpoint.".to_string());
    }
    Ok(BenchConfig {
        input,
        output,
        model_id,
        runs,
        warmups,
        max_tokens,
        endpoint,
        client_identity_pem,
        ca_pem,
    })
}

fn bounded_usize(
    flag: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, String> {
    let value = arg_value(flag)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{flag} must be an integer."))
        })
        .transpose()?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{flag} must be between {minimum} and {maximum}."));
    }
    Ok(value)
}

fn required_path(flag: &str) -> Result<PathBuf, String> {
    optional_path(flag).ok_or_else(|| format!("{flag} is required."))
}

fn optional_path(flag: &str) -> Option<PathBuf> {
    arg_value(flag).map(PathBuf::from)
}

fn arg_value(flag: &str) -> Option<String> {
    let args = env::args().collect::<Vec<_>>();
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err(format!(
            "Input must be a regular file no larger than {max_bytes} bytes."
        ));
    }
    fs::read(path).map_err(|error| error.to_string())
}

fn find_single_gguf(model_dir: &Path) -> Result<PathBuf, String> {
    let mut matches = fs::read_dir(model_dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        })
        .collect::<Vec<_>>();
    matches.sort();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err("No GGUF weights file exists in the selected model directory.".to_string()),
        _ => Err("The selected model directory has ambiguous GGUF weights.".to_string()),
    }
}

fn write_new_report(path: &Path, report: &BenchmarkReport) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Benchmark report must be a new file: {error}"))?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

fn machine_profile() -> MachineProfile {
    let system = System::new_all();
    MachineProfile {
        os_name: System::name().unwrap_or_else(|| "unknown".to_string()),
        os_version: System::os_version().unwrap_or_else(|| "unknown".to_string()),
        architecture: env::consts::ARCH,
        cpu_brand: system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .filter(|brand| !brand.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
        logical_cpu_count: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
    }
}

fn resident_memory_bytes() -> u64 {
    let system = System::new_all();
    sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid))
        .map(|process| process.memory())
        .unwrap_or_default()
}

fn percentile(values: &[u128], percentile: usize) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted.get(index).copied()
}

fn hash_file(path: &Path) -> Result<String, String> {
    sha256_file_hex(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_computed_from_raw_samples() {
        assert_eq!(percentile(&[40, 10, 30, 20], 50), Some(20));
        assert_eq!(percentile(&[40, 10, 30, 20], 95), Some(30));
        assert_eq!(percentile(&[], 95), None);
    }

    #[test]
    fn statistics_include_observed_failures() {
        let samples = vec![
            ModelSample {
                run_index: 0,
                wall_clock_ms: 100,
                time_to_first_token_ms: Some(10),
                generated_tokens: 20,
                tokens_per_second: Some(200.0),
                resident_memory_before_bytes: 10,
                resident_memory_after_bytes: 20,
                output_bytes: 5,
                output_sha256: Some("digest".to_string()),
                service_status: Some("ready".to_string()),
                error_code: None,
            },
            ModelSample {
                run_index: 1,
                wall_clock_ms: 5,
                time_to_first_token_ms: None,
                generated_tokens: 0,
                tokens_per_second: None,
                resident_memory_before_bytes: 20,
                resident_memory_after_bytes: 20,
                output_bytes: 0,
                output_sha256: None,
                service_status: None,
                error_code: Some("real_failure".to_string()),
            },
        ];
        let statistics = model_statistics(&samples);
        assert_eq!(statistics.successful_runs, 1);
        assert_eq!(statistics.failed_runs, 1);
        assert_eq!(statistics.error_rate, 0.5);
        assert_eq!(statistics.total_generated_tokens, 20);
        assert_eq!(statistics.peak_resident_memory_bytes, 20);
    }
}
