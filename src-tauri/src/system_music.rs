use crate::mcp_result::McpToolCallResult;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;

const MUSIC_BACKEND: &str = "media_player";
const DEFAULT_SONG_LIMIT: u32 = 1;
const MAX_SONG_LIMIT: u32 = 20;
const MUSIC_READ_TIMEOUT_SECONDS: u64 = 60;
const MUSIC_AUTHORIZATION_TIMEOUT_SECONDS: u64 = 45;
const MAX_METADATA_CHARACTERS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemMusicSong {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    date_added: String,
    date_added_timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MusicSongCandidate {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    date_added_timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MusicFailure {
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

impl MusicFailure {
    const fn new(code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            code,
            message,
            retryable,
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn read_system_music(max_songs: Option<u32>) -> McpToolCallResult {
    read_system_music_bounded(bounded_song_limit(max_songs)).await
}

pub(crate) async fn read_system_music_bounded(max_songs: u32) -> McpToolCallResult {
    let max_songs = bounded_song_limit(Some(max_songs));
    match tokio::time::timeout(
        std::time::Duration::from_secs(MUSIC_READ_TIMEOUT_SECONDS),
        native_music_songs(max_songs),
    )
    .await
    {
        Ok(Ok((candidates, has_more))) => music_success_result(
            finalize_music_songs(candidates, max_songs as usize),
            has_more,
        ),
        Ok(Err(failure)) => music_error_result(&failure),
        Err(_) => music_error_result(&MusicFailure::new(
            "music_read_timeout",
            "Music took too long to respond. Try again.",
            true,
        )),
    }
}

pub(crate) fn song_limit_from_arguments(arguments: &Value) -> Result<u32, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "Music arguments must be a JSON object.".to_string())?;
    let max_songs = object
        .get("max_songs")
        .or_else(|| object.get("maxSongs"))
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "Music maxSongs must be a positive whole number.".to_string())
        })
        .transpose()?;
    Ok(bounded_song_limit(max_songs))
}

fn bounded_song_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_SONG_LIMIT).clamp(1, MAX_SONG_LIMIT)
}

fn bounded_metadata(value: Option<String>) -> Option<String> {
    let value = value?.replace('\0', "");
    let bounded = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_METADATA_CHARACTERS)
        .collect::<String>();
    (!bounded.is_empty()).then_some(bounded)
}

fn retain_newest_candidate(
    candidates: &mut Vec<MusicSongCandidate>,
    candidate: MusicSongCandidate,
    limit: usize,
) {
    candidates.push(candidate);
    candidates.sort_by(|left, right| {
        right
            .date_added_timestamp_ms
            .cmp(&left.date_added_timestamp_ms)
            .then_with(|| left.title.cmp(&right.title))
    });
    candidates.truncate(limit);
}

fn finalize_music_songs(
    mut candidates: Vec<MusicSongCandidate>,
    limit: usize,
) -> Vec<SystemMusicSong> {
    candidates.sort_by(|left, right| {
        right
            .date_added_timestamp_ms
            .cmp(&left.date_added_timestamp_ms)
            .then_with(|| left.title.cmp(&right.title))
    });
    candidates
        .into_iter()
        .take(limit)
        .filter_map(|candidate| {
            let date_added = rfc3339_from_millis(candidate.date_added_timestamp_ms)?;
            Some(SystemMusicSong {
                title: bounded_metadata(candidate.title),
                artist: bounded_metadata(candidate.artist),
                album: bounded_metadata(candidate.album),
                date_added,
                date_added_timestamp_ms: candidate.date_added_timestamp_ms,
            })
        })
        .collect()
}

fn rfc3339_from_millis(timestamp_ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn music_success_result(songs: Vec<SystemMusicSong>, truncated: bool) -> McpToolCallResult {
    let returned_count = songs.len();
    let structured = serde_json::json!({
        "backend": MUSIC_BACKEND,
        "code": "music_read_ok",
        "songs": songs,
        "returnedCount": returned_count,
        "truncated": truncated,
    });
    McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&structured["songs"])
                .unwrap_or_else(|_| "[]".to_string()),
        })],
        structured_content: Some(structured),
        is_error: false,
        meta: None,
        raw: None,
    }
}

fn music_error_result(failure: &MusicFailure) -> McpToolCallResult {
    let structured = serde_json::json!({
        "backend": MUSIC_BACKEND,
        "code": failure.code,
        "message": failure.message,
        "retryable": failure.retryable,
        "songs": [],
    });
    McpToolCallResult {
        content: vec![serde_json::json!({"type": "text", "text": failure.message})],
        structured_content: Some(structured),
        is_error: true,
        meta: None,
        raw: None,
    }
}

#[cfg(target_os = "macos")]
async fn native_music_songs(
    max_songs: u32,
) -> Result<(Vec<MusicSongCandidate>, bool), MusicFailure> {
    use block2::RcBlock;
    use objc2_media_player::{MPMediaLibrary, MPMediaLibraryAuthorizationStatus, MPMediaQuery};
    use std::sync::Mutex;

    let mut status = unsafe { MPMediaLibrary::authorizationStatus() };
    if status == MPMediaLibraryAuthorizationStatus::NotDetermined {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let sender = Mutex::new(Some(sender));
        {
            let handler = RcBlock::new(move |next_status: MPMediaLibraryAuthorizationStatus| {
                if let Ok(mut sender) = sender.lock() {
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(next_status);
                    }
                }
            });
            unsafe { MPMediaLibrary::requestAuthorization(&handler) };
        }
        status = tokio::time::timeout(
            std::time::Duration::from_secs(MUSIC_AUTHORIZATION_TIMEOUT_SECONDS),
            receiver,
        )
        .await
        .map_err(|_| {
            MusicFailure::new(
                "music_authorization_timeout",
                "Music took too long to respond. Try again.",
                true,
            )
        })?
        .map_err(|_| {
            MusicFailure::new(
                "music_authorization_cancelled",
                "Music did not finish the access request. Try again.",
                true,
            )
        })?;
    }

    if status == MPMediaLibraryAuthorizationStatus::Denied {
        return Err(MusicFailure::new(
            "music_permission_denied",
            "Media & Apple Music access is off. Allow OOMU in System Settings, then try again.",
            false,
        ));
    }
    if status == MPMediaLibraryAuthorizationStatus::Restricted {
        return Err(MusicFailure::new(
            "music_permission_restricted",
            "Media & Apple Music access is restricted on this Mac.",
            false,
        ));
    }
    if status != MPMediaLibraryAuthorizationStatus::Authorized {
        return Err(MusicFailure::new(
            "music_authorization_unknown",
            "Media & Apple Music access could not be verified.",
            true,
        ));
    }

    let (candidates, matched_count) = tokio::task::spawn_blocking(move || unsafe {
        let query = MPMediaQuery::songsQuery();
        let items = query.items();
        let matched_count = items.as_ref().map(|items| items.count()).unwrap_or(0);
        let mut candidates = Vec::with_capacity((max_songs as usize).min(matched_count));
        if let Some(items) = items {
            for index in 0..items.count() {
                let item = items.objectAtIndex(index);
                let seconds = item.dateAdded().timeIntervalSince1970();
                if !seconds.is_finite() {
                    continue;
                }
                retain_newest_candidate(
                    &mut candidates,
                    MusicSongCandidate {
                        title: item.title().map(|value| value.to_string()),
                        artist: item.artist().map(|value| value.to_string()),
                        album: item.albumTitle().map(|value| value.to_string()),
                        date_added_timestamp_ms: (seconds * 1_000.0).round() as i64,
                    },
                    max_songs as usize,
                );
            }
        }
        (candidates, matched_count)
    })
    .await
    .map_err(|_| {
        MusicFailure::new(
            "music_library_read_failed",
            "Your Music library couldn't be read right now. Try again.",
            true,
        )
    })?;

    Ok((candidates, matched_count > max_songs as usize))
}

#[cfg(not(target_os = "macos"))]
async fn native_music_songs(
    _max_songs: u32,
) -> Result<(Vec<MusicSongCandidate>, bool), MusicFailure> {
    Err(MusicFailure::new(
        "music_unavailable",
        "Media & Apple Music access is available only in the OOMU app on macOS.",
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(title: &str, timestamp_ms: i64) -> MusicSongCandidate {
        MusicSongCandidate {
            title: Some(title.to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            date_added_timestamp_ms: timestamp_ms,
        }
    }

    #[test]
    fn song_limits_are_bounded_before_native_access() {
        assert_eq!(bounded_song_limit(None), 1);
        assert_eq!(bounded_song_limit(Some(0)), 1);
        assert_eq!(bounded_song_limit(Some(8)), 8);
        assert_eq!(bounded_song_limit(Some(200)), 20);
        assert_eq!(
            song_limit_from_arguments(&serde_json::json!({"maxSongs": 3})).unwrap(),
            3
        );
        assert!(song_limit_from_arguments(&serde_json::json!({"maxSongs": "all"})).is_err());
    }

    #[test]
    fn newest_song_metadata_is_sorted_and_bounded() {
        let songs = finalize_music_songs(
            vec![
                candidate("Old", 1_000),
                candidate("Newest", 3_000),
                candidate("Middle", 2_000),
            ],
            2,
        );
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].title.as_deref(), Some("Newest"));
        assert_eq!(songs[1].title.as_deref(), Some("Middle"));
        assert_eq!(songs[0].date_added, "1970-01-01T00:00:03.000Z");
    }

    #[test]
    fn newest_candidate_accumulator_stays_bounded() {
        let mut songs = Vec::new();
        for timestamp in 0..10_000 {
            retain_newest_candidate(
                &mut songs,
                candidate(&format!("Song {timestamp}"), timestamp),
                3,
            );
        }
        assert_eq!(songs.len(), 3);
        assert_eq!(songs[0].date_added_timestamp_ms, 9_999);
        assert_eq!(songs[2].date_added_timestamp_ms, 9_997);
    }

    #[test]
    fn overall_music_read_deadline_is_bounded() {
        assert_eq!(MUSIC_READ_TIMEOUT_SECONDS, 60);
        assert!(MUSIC_AUTHORIZATION_TIMEOUT_SECONDS < MUSIC_READ_TIMEOUT_SECONDS);
    }

    #[test]
    fn typed_results_never_expose_raw_native_payloads() {
        let success = music_success_result(
            vec![SystemMusicSong {
                title: Some("Song".to_string()),
                artist: Some("Artist".to_string()),
                album: Some("Album".to_string()),
                date_added: "2026-07-12T12:00:00.000Z".to_string(),
                date_added_timestamp_ms: 1,
            }],
            false,
        );
        assert!(!success.is_error);
        assert!(success.raw.is_none());
        assert_eq!(
            success.structured_content.as_ref().unwrap()["returnedCount"],
            1
        );

        let error = music_error_result(&MusicFailure::new(
            "music_permission_denied",
            "Media & Apple Music access is off.",
            false,
        ));
        assert!(error.is_error);
        assert!(error.raw.is_none());
        assert_eq!(
            error.structured_content.as_ref().unwrap()["songs"],
            serde_json::json!([])
        );
    }

    #[test]
    fn metadata_is_trimmed_and_bounded() {
        let oversized = format!("  {}  ", "A".repeat(MAX_METADATA_CHARACTERS + 50));
        let bounded = bounded_metadata(Some(oversized)).unwrap();
        assert_eq!(bounded.chars().count(), MAX_METADATA_CHARACTERS);
        assert!(!bounded.contains('\0'));
    }
}
