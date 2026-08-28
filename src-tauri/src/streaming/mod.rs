//! A loopback HTTP server that serves files out of in-progress torrents.
//!
//! Watching while downloading needs two things a plain file path cannot give:
//! reads that *wait* for pieces that have not arrived yet, and seeking that
//! tells the engine to fetch the pieces around the new position. librqbit's
//! `FileStream` is `AsyncRead + AsyncSeek` over exactly that, so wrapping it in
//! an HTTP endpoint with `Range` support gives any player — mpv included — a
//! normal seekable stream.
//!
//! The listener is bound to `127.0.0.1` on an ephemeral port and every URL
//! carries a random token, so nothing on the network, and no other local
//! program guessing URLs, can read the user's files through it.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use crate::engine::Engine;
use crate::error::{AppError, AppResult};

/// How much an open-ended range serves before the player asks again.
///
/// librqbit prioritises the pieces just past an open stream's read position, so
/// a longer response keeps that priority in force for longer — which is what
/// makes playback download in order instead of scattering across the file.
const DEFAULT_CHUNK: u64 = 64 * 1024 * 1024;
/// How many times to retry opening a torrent that is still starting up.
const OPEN_ATTEMPTS: usize = 10;
const OPEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
// There is deliberately no "stall timeout" here. Ending a response because
// pieces were slow to arrive looks to the player exactly like the end of the
// file — so seeking into a part that had not downloaded yet made it skip to the
// next episode. Responses now end only when the client goes away or playback is
// stopped, which is what `cancel` below is for.

pub struct StreamServer {
    port: u16,
    token: String,
    /// Cancels every in-flight response, e.g. when the player is closed.
    cancel: Arc<parking_lot::Mutex<CancellationToken>>,
}

#[derive(Clone)]
struct ServerState {
    engine: Arc<Engine>,
    token: String,
    cancel: Arc<parking_lot::Mutex<CancellationToken>>,
}

#[derive(Deserialize)]
struct Auth {
    token: String,
}

impl StreamServer {
    /// Binds to a free loopback port and starts serving.
    pub async fn start(engine: Arc<Engine>) -> AppResult<Self> {
        let token = random_token();
        let cancel = Arc::new(parking_lot::Mutex::new(CancellationToken::new()));
        let state = ServerState {
            engine,
            token: token.clone(),
            cancel: cancel.clone(),
        };

        let app = axum::Router::new()
            .route("/stream/{info_hash}/{file_id}", get(stream_file))
            .with_state(state);

        // Port 0 lets the OS pick; binding to loopback keeps this off the LAN.
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .map_err(|e| AppError::msg(format!("не удалось запустить стрим-сервер: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::msg(format!("стрим-сервер без адреса: {e}")))?
            .port();

        tauri::async_runtime::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("stream server stopped: {e}");
            }
        });

        tracing::info!("stream server listening on 127.0.0.1:{port}");
        Ok(Self {
            port,
            token,
            cancel,
        })
    }

    /// URL a player should open for one file of one torrent.
    pub fn url_for(&self, info_hash: &str, file_id: usize) -> String {
        format!(
            "http://127.0.0.1:{}/stream/{}/{}?token={}",
            self.port,
            info_hash.to_uppercase(),
            file_id,
            self.token
        )
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Ends every response that is still being served.
    ///
    /// Called when playback stops: without it, teardown waits on a read that is
    /// still expecting torrent pieces, which is what made closing feel slow.
    pub fn abort_all(&self) {
        let mut guard = self.cancel.lock();
        guard.cancel();
        // A fresh token, so the next playback is not born cancelled.
        *guard = CancellationToken::new();
    }
}

async fn stream_file(
    State(state): State<ServerState>,
    Path((info_hash, file_id)): Path<(String, usize)>,
    Query(auth): Query<Auth>,
    headers: HeaderMap,
) -> Response {
    if auth.token != state.token {
        return StatusCode::FORBIDDEN.into_response();
    }

    // A torrent can need a moment before it is streamable. Answering 404 here
    // makes the player give up for good, so a short retry is worth it.
    let mut stream = match open_with_retry(&state, &info_hash, file_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("stream {info_hash}/{file_id} unavailable: {e}");
            return (StatusCode::NOT_FOUND, e.to_string()).into_response();
        }
    };

    let total = stream.len;
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_range(v, total));

    let (start, end) = match range {
        Some(Ok(r)) => r,
        // A syntactically valid header we cannot satisfy must not be answered
        // with the whole file — players rely on 416 to correct themselves.
        Some(Err(())) => {
            let mut resp = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            resp.headers_mut().insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{total}")).unwrap(),
            );
            return resp;
        }
        None => (0, total.saturating_sub(1)),
    };

    if start > 0 {
        if let Err(e) = stream.reader.seek(std::io::SeekFrom::Start(start)).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    let length = end.saturating_sub(start) + 1;
    let cancel = state.cancel.lock().clone();
    let body = Body::from_stream(cancellable_body(stream.reader.take(length), cancel));

    let mut resp = Response::new(body);
    let partial = range.is_some();
    *resp.status_mut() = if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    let h = resp.headers_mut();
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).unwrap(),
    );
    // Players sniff the container themselves; a generic type avoids guessing
    // wrong for the many formats a tracker release can be.
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if partial {
        h.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")).unwrap(),
        );
    }
    resp
}

/// Parses a single-range `Range: bytes=…` header.
///
/// Returns `None` when the header is not a byte range at all (the caller then
/// serves the whole file), and `Some(Err(()))` when it is well-formed but
/// unsatisfiable.
type RangeResult = Option<Result<(u64, u64), ()>>;

fn parse_range(value: &str, total: u64) -> RangeResult {
    let spec = value.trim().strip_prefix("bytes=")?;
    // Multi-range requests are legal but no player needs them; serving the
    // first range is the conventional simplification.
    let spec = spec.split(',').next()?.trim();
    let (from, to) = spec.split_once('-')?;

    if total == 0 {
        return Some(Err(()));
    }

    let result = if from.is_empty() {
        // `-N`: the last N bytes.
        let n: u64 = to.parse().ok()?;
        if n == 0 {
            return Some(Err(()));
        }
        (total.saturating_sub(n), total - 1)
    } else {
        let start: u64 = from.parse().ok()?;
        if start >= total {
            return Some(Err(()));
        }
        let end = if to.is_empty() {
            // An open-ended range is capped so the first request does not pull
            // the whole file through the torrent before playback starts.
            (start + DEFAULT_CHUNK - 1).min(total - 1)
        } else {
            to.parse::<u64>().ok()?.min(total - 1)
        };
        if end < start {
            return Some(Err(()));
        }
        (start, end)
    };
    Some(Ok(result))
}

/// Opens a file stream, tolerating a torrent that is still coming up.
async fn open_with_retry(
    state: &ServerState,
    info_hash: &str,
    file_id: usize,
) -> AppResult<crate::engine::FileStream> {
    let mut last = None;
    for attempt in 0..OPEN_ATTEMPTS {
        match state.engine.file_stream(info_hash, file_id).await {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            tokio::time::sleep(OPEN_RETRY_DELAY).await;
        }
    }
    Err(last.unwrap_or_else(|| AppError::msg("поток недоступен")))
}

/// Streams a reader until it ends or playback is stopped.
///
/// A slow read is *waited on*, never cut short: to a player, a truncated
/// response is indistinguishable from the end of the file, and mid-file that
/// makes it skip to the next item. Only an explicit cancel ends it early, which
/// is what keeps closing the player instant.
fn cancellable_body<R>(
    reader: R,
    cancel: CancellationToken,
) -> impl futures::Stream<Item = std::io::Result<bytes::Bytes>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let inner = ReaderStream::new(reader);
    futures::stream::unfold((inner, cancel), |(mut inner, cancel)| async move {
        use futures::StreamExt;
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::debug!("stream cancelled, ending response");
                None
            }
            item = inner.next() => item.map(|item| (item, (inner, cancel))),
        }
    })
}

fn random_token() -> String {
    // Not a secret that needs to survive an attacker with local code execution;
    // it only stops other local processes from guessing a stream URL.
    use std::hash::{BuildHasher, Hasher, RandomState};
    let a = RandomState::new().build_hasher().finish();
    let b = RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_ended_range_is_capped() {
        // A player asking for "everything from 0" should not make us pull the
        // whole film before the first frame.
        let (start, end) = parse_range("bytes=0-", 1_000_000_000).unwrap().unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, DEFAULT_CHUNK - 1);
    }

    #[test]
    fn explicit_range_is_honoured() {
        let (start, end) = parse_range("bytes=100-199", 1000).unwrap().unwrap();
        assert_eq!((start, end), (100, 199));
    }

    #[test]
    fn suffix_range_returns_the_tail() {
        // mpv reads the tail of a file to find the index in some containers.
        let (start, end) = parse_range("bytes=-500", 1000).unwrap().unwrap();
        assert_eq!((start, end), (500, 999));
    }

    #[test]
    fn range_past_the_end_is_unsatisfiable() {
        assert_eq!(parse_range("bytes=2000-", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=500-400", 1000), Some(Err(())));
    }

    #[test]
    fn end_is_clamped_to_the_file() {
        let (_, end) = parse_range("bytes=0-99999", 1000).unwrap().unwrap();
        assert_eq!(end, 999);
    }

    #[test]
    fn non_byte_ranges_fall_back_to_the_whole_file() {
        assert!(parse_range("items=0-10", 1000).is_none());
        assert!(parse_range("bytes=abc", 1000).is_none());
    }

    #[test]
    fn tokens_differ_between_runs() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 32);
    }
}
