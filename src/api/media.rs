use axum::{
    body::Body,
    extract::{Path, Query},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use std::{convert::Infallible, sync::OnceLock, time::Duration};

use crate::{
    auth::AuthUser,
    error::AppError,
    media::{archive, remux::FfmpegRemuxer},
};
use serde::Deserialize;

static SAMPLE_MP4: OnceLock<Vec<u8>> = OnceLock::new();

const VIDEO: &[u8] = include_bytes!("../../assets/sample.h264");
const AUDIO: &[u8] = include_bytes!("../../assets/sample.aac");

const PLAYER_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<title>JT/T 1078 MSE Player</title>
<body>
<h3>JT/T 1078 MSE Player</h3>
<p>
<select id="session"><option value="">活动会话...</option></select>
SIM <input id="sim" value="138000000099" size="14">
channel <input id="channel" value="1" size="3">
<button id="live">加载实时快照</button>
<button id="demo">加载演示样本</button>
<button id="stream">连续播放</button>
<button id="stop">停止</button>
</p>
<video id="v" controls autoplay muted style="width:480px;background:#000"></video>
<p>
<button id="archiveList">归档列表</button>
<span id="archiveStatus"></span>
</p>
<div id="archiveListBox" style="font-size:13px;line-height:1.8"></div>
<p id="status">实时快照来自 TSP_MEDIA_UDP_ADDR；演示样本为内置媒体。</p>
<script>
const video = document.getElementById('v');
const status = document.getElementById('status');
let ms = null, source = null, controller = null;

function reset() {
  if (ms && ms.readyState === 'open') { try { ms.endOfStream(); } catch (e) {} }
  ms = new MediaSource();
  video.src = URL.createObjectURL(ms);
  source = null;
  ms.addEventListener('sourceopen', () => {
    const mime = 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"';
    if (!MediaSource.isTypeSupported(mime)) { status.textContent = 'MSE 不支持: ' + mime; return; }
    source = ms.addSourceBuffer(mime);
    source.addEventListener('updateend', () => {
      if (!source.updating && ms.readyState === 'open') { try { ms.endOfStream(); } catch (e) {} }
    });
  }, { once: true });
}

function authHeaders() {
  const token = localStorage.getItem('tsp_token');
  return token ? { Authorization: 'Bearer ' + token } : {};
}

async function load(url, label) {
  reset();
  status.textContent = label + ' 加载中...';
  const response = await fetch(url, { cache: 'no-store', headers: authHeaders() });
  if (!response.ok) { status.textContent = label + ' 不可用: ' + response.status; return; }
  const buffer = await response.arrayBuffer();
  const append = () => {
    if (!source) { setTimeout(append, 30); return; }
    if (source.updating) { setTimeout(append, 30); return; }
    source.appendBuffer(buffer);
    status.textContent = label + ' 已加载 ' + buffer.byteLength + ' bytes';
  };
  append();
}

async function refreshSessions() {
  try {
    const response = await fetch('/api/media/sessions', { cache: 'no-store', headers: authHeaders() });
    if (!response.ok) return;
    const data = await response.json();
    const select = document.getElementById('session');
    for (const item of data.sessions) {
      const option = document.createElement('option');
      option.value = item.sim + '/' + item.channel;
      option.textContent = item.sim + ' / ch' + item.channel + ' (v' + item.video_bytes + ' a' + item.audio_bytes + ')';
      select.append(option);
    }
  } catch (e) {}
}
refreshSessions();

async function appendChunk(bytes) {
  return new Promise((resolve) => {
    const tryAppend = () => {
      if (!source) { setTimeout(tryAppend, 30); return; }
      if (source.updating) { setTimeout(tryAppend, 30); return; }
      source.appendBuffer(bytes);
      source.addEventListener('updateend', resolve, { once: true });
    };
    tryAppend();
  });
}

async function startStream(sim, channel) {
  reset();
  controller = new AbortController();
  const signal = controller.signal;
  status.textContent = '连续播放连接中...';
  const response = await fetch('/api/media/live/' + encodeURIComponent(sim) + '/' + encodeURIComponent(channel) + '/stream', { cache: 'no-store', signal });
  if (!response.ok || !response.body) { status.textContent = '连续播放不可用: ' + response.status; return; }
  const reader = response.body.getReader();
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) { break; }
    await appendChunk(value);
    total += value.byteLength;
    status.textContent = '连续播放已接收 ' + total + ' bytes';
  }
  status.textContent = '连续播放结束，共 ' + total + ' bytes';
}

document.getElementById('stop').onclick = async () => {
  if (controller) { controller.abort(); controller = null; }
  const selected = document.getElementById('session').value;
  const sim = selected ? selected.split('/')[0] : document.getElementById('sim').value.trim();
  const channel = selected ? selected.split('/')[1] : document.getElementById('channel').value.trim();
  try { await fetch('/api/media/sessions/' + encodeURIComponent(sim) + '/' + encodeURIComponent(channel), { method: 'DELETE', headers: authHeaders() }); } catch (e) {}
  reset();
  status.textContent = '已停止并清理会话';
  if (typeof refreshSessions === 'function') refreshSessions();
};

document.getElementById('stream').onclick = () => {
  const selected = document.getElementById('session').value;
  const sim = selected ? selected.split('/')[0] : document.getElementById('sim').value.trim();
  const channel = selected ? selected.split('/')[1] : document.getElementById('channel').value.trim();
  startStream(sim, channel);
};

document.getElementById('demo').onclick = () => load('/api/media/sample.mp4', '演示样本');
document.getElementById('live').onclick = () => {
  const selected = document.getElementById('session').value;
  const sim = selected ? selected.split('/')[0] : document.getElementById('sim').value.trim();
  const channel = selected ? selected.split('/')[1] : document.getElementById('channel').value.trim();
  load('/api/media/live/' + encodeURIComponent(sim) + '/' + encodeURIComponent(channel), '实时快照');
};

function currentTarget() {
  const selected = document.getElementById('session').value;
  const sim = selected ? selected.split('/')[0] : document.getElementById('sim').value.trim();
  const channel = selected ? selected.split('/')[1] : document.getElementById('channel').value.trim();
  return { sim, channel };
}

async function downloadArchive(key) {
  const response = await fetch('/api/media/archive/' + key.split('/').map(encodeURIComponent).join('/'), { cache: 'no-store', headers: authHeaders() });
  if (!response.ok) { status.textContent = '归档下载不可用: ' + response.status; return; }
  const blob = await response.blob();
  const link = document.createElement('a');
  link.href = URL.createObjectURL(blob);
  link.download = key.split('/').pop() || 'segment.bin';
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 5000);
}

function playArchive(key) {
  load('/api/media/archive/playback?key=' + encodeURIComponent(key), '归档回放');
}

async function refreshArchive() {
  const { sim, channel } = currentTarget();
  const box = document.getElementById('archiveListBox');
  const label = document.getElementById('archiveStatus');
  box.innerHTML = '';
  label.textContent = '查询中...';
  try {
    const response = await fetch('/api/media/archive?sim=' + encodeURIComponent(sim) + '&channel=' + encodeURIComponent(channel), { cache: 'no-store', headers: authHeaders() });
    if (!response.ok) { label.textContent = '归档不可用: ' + response.status; return; }
    const data = await response.json();
    label.textContent = data.objects.length + ' 个对象';
    for (const item of data.objects) {
      const row = document.createElement('div');
      const name = document.createElement('span');
      name.textContent = item.key + ' (' + item.size + ' bytes) ';
      const play = document.createElement('button');
      play.textContent = '播放';
      play.onclick = () => playArchive(item.key);
      const download = document.createElement('button');
      download.textContent = '下载';
      download.onclick = () => downloadArchive(item.key);
      row.append(name, play, download);
      box.append(row);
    }
    if (data.objects.length === 0) { label.textContent = '暂无归档'; }
  } catch (e) {
    label.textContent = '归档查询失败';
  }
}

document.getElementById('archiveList').onclick = refreshArchive;
</script>
</body>
</html>"#;

pub async fn player() -> Html<&'static str> {
    Html(PLAYER_HTML)
}

pub async fn sample_mp4() -> Result<Response, AppError> {
    if let Some(bytes) = SAMPLE_MP4.get() {
        return Ok(mp4_response(bytes));
    }
    let program = std::env::var("TSP_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let remuxer = FfmpegRemuxer::new(program);
    let bytes = remuxer.to_fmp4(Some(VIDEO), Some(AUDIO)).map_err(|error| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "media_unavailable",
            format!("{error:?}"),
        )
    })?;
    let bytes = SAMPLE_MP4.get_or_init(|| bytes);
    Ok(mp4_response(bytes))
}

pub async fn sessions(user: AuthUser) -> Result<Response, AppError> {
    user.require_write()?;
    let items: Vec<_> = crate::media::hub::global()
        .list()
        .into_iter()
        .map(|(sim, channel, video, audio)| {
            serde_json::json!({
                "sim": sim,
                "channel": channel,
                "video_bytes": video,
                "audio_bytes": audio,
            })
        })
        .collect();
    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "sessions": items })),
    )
        .into_response())
}

pub async fn live_stream(
    user: AuthUser,
    Path((sim, channel)): Path<(String, u8)>,
) -> Result<Response, AppError> {
    user.require_write()?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, Infallible>>(8);
    tokio::spawn(async move {
        let program = std::env::var("TSP_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
        let remuxer = FfmpegRemuxer::new(program);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some((video, audio)) = crate::media::hub::global().take(&sim, channel) else {
                continue;
            };
            let result = remuxer.to_fmp4(
                (!video.is_empty()).then_some(video.as_slice()),
                (!audio.is_empty()).then_some(audio.as_slice()),
            );
            match result {
                Ok(bytes) => {
                    if tx.send(Ok(axum::body::Bytes::from(bytes))).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(?error, "live media remux failed");
                    break;
                }
            }
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    Ok(Body::from_stream(stream).into_response())
}

pub async fn close_session(
    user: AuthUser,
    Path((sim, channel)): Path<(String, u8)>,
) -> Result<StatusCode, AppError> {
    user.require_write()?;
    if crate::media::hub::global().clear(&sim, channel) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found("媒体会话不存在"))
    }
}

pub async fn live_mp4(
    user: AuthUser,
    Path((sim, channel)): Path<(String, u8)>,
) -> Result<Response, AppError> {
    user.require_write()?;
    let Some((video, audio)) = crate::media::hub::global().snapshot(&sim, channel) else {
        return Err(AppError::not_found("没有该会话的媒体数据"));
    };
    let program = std::env::var("TSP_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let remuxer = FfmpegRemuxer::new(program);
    let bytes = remuxer
        .to_fmp4(
            (!video.is_empty()).then_some(video.as_slice()),
            (!audio.is_empty()).then_some(audio.as_slice()),
        )
        .map_err(|error| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "media_unavailable",
                format!("{error:?}"),
            )
        })?;
    Ok(mp4_response(&bytes))
}

#[derive(Debug, Deserialize)]
pub struct ArchiveQuery {
    pub sim: String,
    pub channel: u8,
}

pub async fn archive_list(
    _user: AuthUser,
    Query(query): Query<ArchiveQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let archive = archive::global().ok_or_else(archive_unavailable)?;
    let prefix = archive.media_prefix(&query.sim, query.channel);
    let objects = archive.list_prefix(&prefix).await.map_err(archive_error)?;
    Ok(Json(serde_json::json!({
        "prefix": prefix,
        "objects": objects,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ArchivePlaybackQuery {
    pub key: String,
}

pub async fn archive_playback(
    _user: AuthUser,
    Query(query): Query<ArchivePlaybackQuery>,
) -> Result<Response, AppError> {
    let archive = archive::global().ok_or_else(archive_unavailable)?;
    archive
        .validate_key(&query.key)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    let (video_key, audio_key) = archive_pair(&query.key)?;

    let video = archive.get(&video_key).await.map_err(archive_error)?;
    let audio = match audio_key {
        Some(key) => match archive.get(&key).await {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                tracing::debug!(%error, %key, "archive audio sidecar unavailable");
                None
            }
        },
        None => None,
    };
    let program = std::env::var("TSP_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let remuxer = FfmpegRemuxer::new(program);
    let bytes = remuxer
        .to_fmp4(Some(&video), audio.as_deref())
        .map_err(|error| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "archive_playback_failed",
                format!("{error:?}"),
            )
        })?;
    Ok(mp4_response(&bytes))
}

pub async fn archive_get(_user: AuthUser, Path(key): Path<String>) -> Result<Response, AppError> {
    let archive = archive::global().ok_or_else(archive_unavailable)?;
    archive
        .validate_key(&key)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    let bytes = archive.get(&key).await.map_err(archive_error)?;
    let content_type = if key.ends_with(".h264") {
        "video/h264"
    } else if key.ends_with(".aac") {
        "audio/aac"
    } else {
        "application/octet-stream"
    };
    let filename = key.rsplit('/').next().unwrap_or("segment.bin");
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "private, max-age=3600"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{filename}\"")[..],
            ),
        ],
        bytes,
    )
        .into_response())
}

fn archive_pair(key: &str) -> Result<(String, Option<String>), AppError> {
    if let Some(stem) = key.strip_suffix(".h264") {
        Ok((key.to_string(), Some(format!("{stem}.aac"))))
    } else if let Some(stem) = key.strip_suffix(".aac") {
        Ok((format!("{stem}.h264"), Some(key.to_string())))
    } else {
        Err(AppError::bad_request(
            "archive playback requires a .h264 or .aac object",
        ))
    }
}

fn archive_unavailable() -> AppError {
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "archive_unavailable",
        "RustFS archive is not configured",
    )
}

fn archive_error(error: archive::ArchiveError) -> AppError {
    AppError::new(StatusCode::BAD_GATEWAY, "archive_error", error.to_string())
}

fn mp4_response(bytes: &[u8]) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "video/mp4"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        bytes.to_vec(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_html_uses_mse_and_sample_endpoint() {
        assert!(PLAYER_HTML.contains("MediaSource"));
        assert!(PLAYER_HTML.contains("/api/media/sample.mp4"));
        assert!(PLAYER_HTML.contains("/api/media/live/"));
        assert!(PLAYER_HTML.contains("/api/media/sessions"));
        assert!(PLAYER_HTML.contains("/api/media/archive"));
        assert!(PLAYER_HTML.contains("/api/media/archive/playback"));
        assert!(PLAYER_HTML.contains("archiveList"));
        assert!(PLAYER_HTML.contains("/stream"));
        assert!(PLAYER_HTML.contains("id=\"stop\""));
        assert!(PLAYER_HTML.contains("avc1.42E01E"));
        assert!(PLAYER_HTML.contains("mp4a.40.2"));
    }

    #[test]
    fn archive_playback_pairs_video_and_audio_keys() {
        let (video, audio) = archive_pair("media/1/1/2026/11/24/080001-000000.h264").unwrap();
        assert!(video.ends_with(".h264"));
        assert_eq!(
            audio.as_deref(),
            Some("media/1/1/2026/11/24/080001-000000.aac")
        );
        let (video, audio) = archive_pair("media/1/1/2026/11/24/080001-000000.aac").unwrap();
        assert!(video.ends_with(".h264"));
        assert_eq!(
            audio.as_deref(),
            Some("media/1/1/2026/11/24/080001-000000.aac")
        );
        assert!(archive_pair("media/1/1/file.mp4").is_err());
    }
}
