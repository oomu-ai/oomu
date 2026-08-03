use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT_ENCODING, ETAG, IF_RANGE, LOCATION, RANGE},
    redirect::Policy,
    Client, Response, StatusCode,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    time::{sleep, timeout},
};
use url::Url;

use super::{
    manifest::RecommendedModelAsset,
    network::{
        enforce_header_ceiling, sanitize_etag, validate_content_range, validate_declared_length,
        validate_network_destination,
    },
    partial_io::{open_partial_for_download, partial_length, reset_partial},
    state::InstallError,
};

pub(crate) use super::partial_io::verify_asset_file;

const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RETRIES: usize = 3;

#[derive(Clone, Debug)]
pub(crate) struct AssetDownloadProgress {
    pub asset_downloaded_bytes: u64,
    pub aggregate_downloaded_bytes: u64,
    pub etag: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadOutcome {
    pub bytes: u64,
    pub etag: Option<String>,
}

type ProgressCallback = Arc<dyn Fn(AssetDownloadProgress) + Send + Sync>;

struct PreparedResponse {
    response: Response,
    existing_bytes: u64,
    etag: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Downloader {
    client: Client,
    allow_local_fixture: bool,
}

impl Downloader {
    pub(crate) fn production() -> Result<Self, InstallError> {
        Self::build(false)
    }

    fn build(allow_local_fixture: bool) -> Result<Self, InstallError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("OOMU/recommended-model-installer")
            .build()
            .map_err(|error| {
                InstallError::new(
                    "model_install_transport_unavailable",
                    true,
                    error.to_string(),
                )
            })?;
        Ok(Self {
            client,
            allow_local_fixture,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_local_fixture() -> Result<Self, InstallError> {
        Self::build(true)
    }

    pub(crate) async fn download_asset(
        &self,
        asset: &RecommendedModelAsset,
        partial_path: &Path,
        known_etag: Option<String>,
        aggregate_before_asset: u64,
        cancellation: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<DownloadOutcome, InstallError> {
        let mut last_error = None;
        let mut etag = sanitize_etag(known_etag.as_deref());
        for attempt in 0..MAX_RETRIES {
            if cancellation.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            let existing = partial_length(partial_path).await?;
            if existing > asset.bytes {
                reset_partial(partial_path).await?;
                etag = None;
            }
            match self
                .download_attempt(
                    asset,
                    partial_path,
                    &mut etag,
                    aggregate_before_asset,
                    Arc::clone(&cancellation),
                    Arc::clone(&progress),
                )
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(error) if error.code == "model_install_cancelled" => return Err(error),
                Err(error) if error.retryable && attempt + 1 < MAX_RETRIES => {
                    last_error = Some(error);
                    cancellable_backoff(
                        Duration::from_millis(400_u64.saturating_mul(1_u64 << attempt)),
                        &cancellation,
                    )
                    .await?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            InstallError::new(
                "model_install_download_failed",
                true,
                "bounded retry loop ended without an outcome",
            )
        }))
    }

    async fn download_attempt(
        &self,
        asset: &RecommendedModelAsset,
        partial_path: &Path,
        known_etag: &mut Option<String>,
        aggregate_before_asset: u64,
        cancellation: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<DownloadOutcome, InstallError> {
        let existing = normalize_partial_for_attempt(partial_path, known_etag).await?;
        let prior_etag = known_etag.clone();
        let response = self
            .request_with_safe_redirects(&asset.url, existing, prior_etag.as_deref())
            .await?;
        let prepared =
            prepare_response(response, partial_path, asset.bytes, existing, prior_etag).await?;
        *known_etag = prepared.etag.clone();
        let written = stream_response_to_partial(
            prepared,
            partial_path,
            asset.bytes,
            aggregate_before_asset,
            cancellation,
            progress,
        )
        .await?;
        if let Err(error) = verify_asset_file(asset, partial_path).await {
            if matches!(
                error.code,
                "model_install_integrity_mismatch" | "model_install_size_mismatch"
            ) {
                let _ = tokio::fs::remove_file(partial_path).await;
            }
            return Err(error);
        }
        Ok(DownloadOutcome {
            bytes: written,
            etag: known_etag.clone(),
        })
    }

    async fn request_with_safe_redirects(
        &self,
        initial_url: &str,
        offset: u64,
        etag: Option<&str>,
    ) -> Result<Response, InstallError> {
        let mut url = Url::parse(initial_url).map_err(|error| {
            InstallError::new("model_install_manifest_invalid", false, error.to_string())
        })?;
        for redirect_index in 0..=MAX_REDIRECTS {
            validate_network_destination(&url, self.allow_local_fixture).await?;
            let mut request = self
                .client
                .get(url.clone())
                .header(ACCEPT_ENCODING, "identity");
            if offset > 0 {
                request = request.header(RANGE, format!("bytes={offset}-"));
                if let Some(etag) = etag {
                    request = request.header(IF_RANGE, etag);
                }
            }
            let response = request.send().await.map_err(|error| {
                InstallError::new("model_install_download_failed", true, error.to_string())
            })?;
            enforce_header_ceiling(response.headers())?;
            if !response.status().is_redirection() {
                return Ok(response);
            }
            if redirect_index == MAX_REDIRECTS {
                return Err(InstallError::new(
                    "model_install_redirect_limit",
                    false,
                    "redirect count exceeded release policy",
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    InstallError::new(
                        "model_install_redirect_invalid",
                        false,
                        "redirect omitted a valid Location header",
                    )
                })?;
            url = url.join(location).map_err(|error| {
                InstallError::new("model_install_redirect_invalid", false, error.to_string())
            })?;
        }
        Err(InstallError::new(
            "model_install_redirect_limit",
            false,
            "redirect loop ended unexpectedly",
        ))
    }
}

async fn normalize_partial_for_attempt(
    partial_path: &Path,
    known_etag: &Option<String>,
) -> Result<u64, InstallError> {
    let existing = partial_length(partial_path).await?;
    if existing > 0 && known_etag.is_none() {
        reset_partial(partial_path).await?;
        Ok(0)
    } else {
        Ok(existing)
    }
}

async fn prepare_response(
    response: Response,
    partial_path: &Path,
    asset_bytes: u64,
    existing_bytes: u64,
    prior_etag: Option<String>,
) -> Result<PreparedResponse, InstallError> {
    let status = response.status();
    if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
        return Err(InstallError::new(
            "model_install_http_status_invalid",
            status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
            format!("unexpected status {}", status.as_u16()),
        ));
    }
    enforce_header_ceiling(response.headers())?;
    let response_etag = sanitize_etag(
        response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok()),
    );
    let existing_bytes = if status == StatusCode::PARTIAL_CONTENT {
        validate_partial_response(
            &response,
            partial_path,
            asset_bytes,
            existing_bytes,
            prior_etag.as_deref(),
            response_etag.as_deref(),
        )
        .await?;
        existing_bytes
    } else {
        if existing_bytes > 0 {
            reset_partial(partial_path).await?;
        }
        0
    };
    validate_declared_length(response.headers(), asset_bytes - existing_bytes)?;
    Ok(PreparedResponse {
        response,
        existing_bytes,
        etag: response_etag,
    })
}

async fn validate_partial_response(
    response: &Response,
    partial_path: &Path,
    asset_bytes: u64,
    existing_bytes: u64,
    prior_etag: Option<&str>,
    response_etag: Option<&str>,
) -> Result<(), InstallError> {
    if let Err(error) = validate_content_range(response.headers(), existing_bytes, asset_bytes) {
        reset_partial(partial_path).await?;
        return Err(error);
    }
    if existing_bytes == 0 {
        return Err(InstallError::new(
            "model_install_content_range_invalid",
            true,
            "server returned partial content for a new asset",
        ));
    }
    if prior_etag.is_some() && response_etag != prior_etag {
        reset_partial(partial_path).await?;
        return Err(InstallError::new(
            "model_install_validator_changed",
            true,
            "remote validator changed during resume",
        ));
    }
    Ok(())
}

async fn stream_response_to_partial(
    prepared: PreparedResponse,
    partial_path: &Path,
    asset_bytes: u64,
    aggregate_before_asset: u64,
    cancellation: Arc<AtomicBool>,
    progress: ProgressCallback,
) -> Result<u64, InstallError> {
    let output =
        open_partial_for_download(partial_path, prepared.existing_bytes > 0).map_err(|error| {
            InstallError::new("model_install_write_failed", true, error.to_string())
        })?;
    let mut output = tokio::fs::File::from_std(output);
    let mut written = prepared.existing_bytes;
    let mut stream = prepared.response.bytes_stream();
    loop {
        if cancellation.load(Ordering::Acquire) {
            output.sync_all().await.map_err(|error| {
                InstallError::new("model_install_write_failed", true, error.to_string())
            })?;
            return Err(cancelled_error());
        }
        let next = timeout(IDLE_READ_TIMEOUT, stream.next())
            .await
            .map_err(|_| {
                InstallError::new(
                    "model_install_read_timeout",
                    true,
                    "response produced no bytes before the idle deadline",
                )
            })?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|error| {
            InstallError::new("model_install_download_failed", true, error.to_string())
        })?;
        written = checked_download_length(written, chunk.len() as u64, asset_bytes)?;
        output.write_all(&chunk).await.map_err(|error| {
            InstallError::new("model_install_write_failed", true, error.to_string())
        })?;
        progress(AssetDownloadProgress {
            asset_downloaded_bytes: written,
            aggregate_downloaded_bytes: aggregate_before_asset + written,
            etag: prepared.etag.clone(),
        });
    }
    output.sync_all().await.map_err(|error| {
        InstallError::new("model_install_write_failed", true, error.to_string())
    })?;
    if written != asset_bytes {
        return Err(InstallError::new(
            "model_install_size_mismatch",
            true,
            format!("downloaded {written} of {asset_bytes} bytes"),
        ));
    }
    Ok(written)
}

fn checked_download_length(
    written: u64,
    chunk_bytes: u64,
    asset_bytes: u64,
) -> Result<u64, InstallError> {
    let written = written.checked_add(chunk_bytes).ok_or_else(|| {
        InstallError::new(
            "model_install_size_mismatch",
            false,
            "download byte counter overflowed",
        )
    })?;
    if written > asset_bytes {
        return Err(InstallError::new(
            "model_install_size_mismatch",
            false,
            "response exceeded the immutable asset byte ceiling",
        ));
    }
    Ok(written)
}

async fn cancellable_backoff(
    duration: Duration,
    cancellation: &AtomicBool,
) -> Result<(), InstallError> {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        if cancellation.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        let step = (duration - elapsed).min(Duration::from_millis(100));
        sleep(step).await;
        elapsed += step;
    }
    Ok(())
}

fn cancelled_error() -> InstallError {
    InstallError::new(
        "model_install_cancelled",
        true,
        "native cancellation flag was set",
    )
}
