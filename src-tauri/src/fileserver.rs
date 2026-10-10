use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use warp::Filter;
use warp::http::{Response, StatusCode};
use warp::hyper::Body;
use tokio::io::AsyncReadExt;

use crate::capture;
use crate::session::{self, Thumb, ThumbLookup, ThumbMiss};
use crate::state::AppState;

/// At most this many ffmpeg thumbnail extractions at once. A ribbon asks for
/// dozens together; cache hits never wait for a slot.
const THUMB_SLOTS: usize = 3;
static THUMB_GATE: Semaphore = Semaphore::const_new(THUMB_SLOTS);

pub async fn start(state: Arc<Mutex<AppState>>, port: u16) {
    warp::serve(routes(state)).run(([127, 0, 0, 1], port)).await;
}

fn routes(
    state: Arc<Mutex<AppState>>,
) -> impl Filter<Extract = (Response<Body>,), Error = warp::Rejection> + Clone {
    let state = warp::any().map(move || state.clone());

    let files = warp::path("files")
        .and(warp::path::tail())
        .and(warp::header::optional::<String>("range"))
        .and(state.clone())
        .and_then(serve_file);

    // GET /thumb/<session_id>/<track_id>/<epoch_ms>.jpg?w=<width>
    let thumb = warp::path!("thumb" / String / String / String)
        .and(warp::get())
        .and(warp::query::<ThumbQuery>())
        .and(state)
        .and_then(serve_thumb);

    files.or(thumb).unify()
}

#[derive(serde::Deserialize)]
struct ThumbQuery {
    /// Lenient: anything unparseable falls back to the default width.
    w: Option<String>,
}

fn plain(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(msg.to_string()))
        .unwrap()
}

fn miss_text(m: ThumbMiss) -> &'static str {
    match m {
        ThumbMiss::NoSession => "No such session",
        ThumbMiss::NoTrack => "No such video track",
        ThumbMiss::Cut => "Deleted",
        ThumbMiss::Gap => "No footage at that time",
        ThumbMiss::NotReady => "Segment not finished yet",
        ThumbMiss::NoFrame => "No frame at that time",
    }
}

/// The parts of a `/thumb` path, validated: two allowlisted ids and an epoch.
fn parse_thumb_path(session_id: &str, track_id: &str, file: &str) -> Option<(String, String, i64)> {
    let session_id = urlencoding::decode(session_id).ok()?.to_string();
    let track_id = urlencoding::decode(track_id).ok()?.to_string();
    crate::edit::check_id(&session_id).ok()?;
    crate::edit::check_id(&track_id).ok()?;
    let epoch_ms = file.strip_suffix(".jpg")?.parse::<i64>().ok()?;
    Some((session_id, track_id, epoch_ms))
}

/// `find_ffmpeg` spawns a process per call; remember the first hit.
fn ffmpeg_path() -> Option<String> {
    static FOUND: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let mut found = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    if found.is_none() {
        *found = capture::find_ffmpeg();
    }
    found.clone()
}

async fn serve_thumb(
    session_id: String,
    track_id: String,
    file: String,
    query: ThumbQuery,
    state: Arc<Mutex<AppState>>,
) -> Result<Response<Body>, warp::Rejection> {
    let Some((session_id, track_id, epoch_ms)) = parse_thumb_path(&session_id, &track_id, &file) else {
        return Ok(plain(StatusCode::NOT_FOUND, "Not found"));
    };
    let width = session::clamp_thumb_width(query.w.and_then(|w| w.parse().ok()));
    let root = {
        let s = state.lock().await;
        crate::recorder::sessions_root(&capture::get_output_dir(&s.config))
    };
    let dir = root.join(&session_id);

    // Security: prevent traversal (the ids are allowlisted too)
    match (root.canonicalize(), dir.canonicalize()) {
        (Ok(base), Ok(target)) if target.starts_with(&base) => {}
        (Ok(_), Ok(_)) => return Ok(plain(StatusCode::FORBIDDEN, "Forbidden")),
        _ => return Ok(plain(StatusCode::NOT_FOUND, miss_text(ThumbMiss::NoSession))),
    }

    let result = thumb_for(dir, track_id, epoch_ms, width).await;
    Ok(match result {
        Ok(Thumb::Ready(path)) => match tokio::fs::read(&path).await {
            Ok(data) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "image/jpeg")
                .header("Content-Length", data.len())
                // A thumbnail can be deleted with its footage; no copy may outlive it.
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "*")
                .body(Body::from(data))
                .unwrap(),
            // Purged between the lookup and the read: it was just cut.
            Err(_) => plain(StatusCode::NOT_FOUND, miss_text(ThumbMiss::Cut)),
        },
        Ok(Thumb::Missing(m)) => plain(StatusCode::NOT_FOUND, miss_text(m)),
        Err(e) => {
            eprintln!("[chirptime] thumb {}/{}: {}", session_id, epoch_ms, e);
            plain(StatusCode::INTERNAL_SERVER_ERROR, &e)
        }
    })
}

/// Lookup, then (only on a cache miss) wait for an extraction slot and render.
async fn thumb_for(dir: PathBuf, track_id: String, epoch_ms: i64, width: u32) -> Result<Thumb, String> {
    let d = dir.clone();
    let lookup = tokio::task::spawn_blocking(move || session::thumb_lookup(&d, &track_id, epoch_ms, width))
        .await
        .map_err(|e| e.to_string())?;
    let job = match lookup {
        Ok(ThumbLookup::Cached(path)) => return Ok(Thumb::Ready(path)),
        Ok(ThumbLookup::Render(job)) => job,
        Err(m) => return Ok(Thumb::Missing(m)),
    };
    let _slot = THUMB_GATE.acquire().await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let ffmpeg = ffmpeg_path().ok_or("FFmpeg not found")?;
        session::render_thumb(&ffmpeg, &dir, &job)
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn serve_file(
    tail: warp::path::Tail,
    range_header: Option<String>,
    state: Arc<Mutex<AppState>>,
) -> Result<Response<Body>, warp::Rejection> {
    let rel_path = urlencoding::decode(tail.as_str()).unwrap_or_default().to_string();

    let output_dir = {
        let s = state.lock().await;
        capture::get_output_dir(&s.config)
    };

    let file_path = output_dir.join(&rel_path);

    // Security: prevent traversal
    if let (Ok(base), Ok(target)) = (output_dir.canonicalize(), file_path.canonicalize()) {
        if !target.starts_with(&base) {
            return Ok(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("Forbidden"))
                .unwrap());
        }
    }

    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not found"))
                .unwrap());
        }
    };

    let total_size = metadata.len();
    let mime = mime_for_path(&file_path);

    if let Some(range_str) = range_header {
        if let Some(range) = range_str.strip_prefix("bytes=") {
            let parts: Vec<&str> = range.splitn(2, '-').collect();
            let start: u64 = parts[0].parse().unwrap_or(0);
            let end: u64 = if parts.len() > 1 && !parts[1].is_empty() {
                parts[1].parse().unwrap_or(total_size - 1)
            } else {
                total_size - 1
            };
            let end = std::cmp::min(end, total_size - 1);
            let length = end - start + 1;

            let mut file = match tokio::fs::File::open(&file_path).await {
                Ok(f) => f,
                Err(_) => {
                    return Ok(Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Read error"))
                        .unwrap());
                }
            };

            use tokio::io::AsyncSeekExt;
            let _ = file.seek(std::io::SeekFrom::Start(start)).await;
            let mut buf = vec![0u8; length as usize];
            let _ = file.read_exact(&mut buf).await;

            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", mime)
                .header("Accept-Ranges", "bytes")
                .header("Content-Range", format!("bytes {}-{}/{}", start, end, total_size))
                .header("Content-Length", length)
                .header("Access-Control-Allow-Origin", "*")
                .body(Body::from(buf))
                .unwrap());
        }
    }

    // Full file
    let data = match tokio::fs::read(&file_path).await {
        Ok(d) => d,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Read error"))
                .unwrap());
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", total_size)
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(data))
        .unwrap())
}

fn mime_for_path(path: &PathBuf) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("mp4") => "video/mp4",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Cut, CutState, Rect, Segment, SessionManifest, Track, TrackStatus};

    const ID: &str = "2026-01-04_09-00-00";
    const T0: i64 = 1_767_517_200_000;

    fn track(id: &str, kind: &str, segments: Vec<Segment>) -> Track {
        Track {
            id: id.to_string(),
            kind: kind.to_string(),
            label: id.to_string(),
            monitor: None,
            codec: String::new(),
            width: 0,
            height: 0,
            fps: 0,
            start_epoch_ms: T0,
            dir: id.to_string(),
            segments,
            gaps: Vec::new(),
            status: TrackStatus::Stopped,
            error: None,
            restarts: 0,
        }
    }

    /// An output dir holding one session: `screen-0` with one 10s segment
    /// listed but not on disk, a `mic`, and a cut over t+20s..t+30s. No ffmpeg
    /// runs here: the 200 case is a cache hit.
    fn fixture(test: &str) -> (PathBuf, Arc<Mutex<AppState>>) {
        let out = std::env::temp_dir()
            .join("chirptime-fileserver")
            .join(format!("{}-{}", test, std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let dir = crate::recorder::sessions_root(&out).join(ID);
        std::fs::create_dir_all(&dir).unwrap();
        let seg = |file: &str, start: i64| Segment {
            file: file.to_string(),
            start_epoch_ms: start,
            duration_ms: 10_000,
            bytes: 1,
        };
        let mut m = SessionManifest::new(ID.to_string(), T0, 10, Rect::default());
        m.tracks.push(track("screen-0", "screen", vec![seg("a.mp4", T0), seg("c.mp4", T0 + 20_000)]));
        m.tracks.push(track("mic", "mic", vec![seg("m.mp4", T0)]));
        m.cuts.push(Cut {
            id: "cut-1".to_string(),
            from_epoch_ms: T0 + 20_000,
            to_epoch_ms: T0 + 30_000,
            tracks: None,
            created_epoch_ms: T0,
            state: CutState::Final,
            freed_bytes: 0,
        });
        crate::session::write_manifest(&dir, &m).unwrap();
        let mut s = AppState::default();
        s.config.output_dir = out.to_string_lossy().to_string();
        (dir, Arc::new(Mutex::new(s)))
    }

    async fn get(state: &Arc<Mutex<AppState>>, path: &str) -> Response<warp::hyper::body::Bytes> {
        warp::test::request().method("GET").path(path).reply(&routes(state.clone())).await
    }

    #[test]
    fn thumb_paths_are_validated() {
        assert_eq!(
            parse_thumb_path(ID, "screen-0", "1767517200000.jpg"),
            Some((ID.to_string(), "screen-0".to_string(), T0))
        );
        assert!(parse_thumb_path("..", "screen-0", "1.jpg").is_none());
        assert!(parse_thumb_path("%2E%2E", "screen-0", "1.jpg").is_none());
        assert!(parse_thumb_path(ID, "..%5Cx", "1.jpg").is_none());
        assert!(parse_thumb_path(ID, "screen-0", "1.png").is_none());
        assert!(parse_thumb_path(ID, "screen-0", "abc.jpg").is_none());
    }

    #[tokio::test]
    async fn thumb_route_serves_the_cache_and_404s_everything_else() {
        let (dir, state) = fixture("route");
        let at = |ms: i64| format!("/thumb/{}/screen-0/{}.jpg", ID, T0 + ms);

        // Cache hit: no ffmpeg needed. An oversized w is clamped to 960.
        let cached = session::thumb_path(&dir, "screen-0", 960, T0 + 5_000);
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"\xFF\xD8jpeg").unwrap();
        let r = get(&state, &format!("{}?w=5000", at(5_000))).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-type"], "image/jpeg");
        assert_eq!(r.headers()["cache-control"], "no-store");
        assert_eq!(r.body().as_ref(), b"\xFF\xD8jpeg");

        // A thumbnail inside a cut is never served, even when a stale one is cached.
        let stale = session::thumb_path(&dir, "screen-0", 320, T0 + 25_000);
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, b"\xFF\xD8old").unwrap();

        for path in [
            at(25_000),                                            // inside the cut
            at(15_000),                                            // a gap
            at(5_000),                                             // segment listed, file not on disk (w=320 not cached)
            format!("/thumb/{}/mic/{}.jpg", ID, T0 + 5_000),       // no picture
            format!("/thumb/{}/webcam/{}.jpg", ID, T0 + 5_000),    // no such track
            format!("/thumb/2026-01-01_00-00-00/screen-0/{}.jpg", T0), // no such session
            format!("/thumb/..%5C{}/screen-0/{}.jpg", ID, T0),     // traversal
            format!("/thumb/{}/screen-0/{}.png", ID, T0 + 5_000),  // not .jpg
        ] {
            let r = get(&state, &path).await;
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{}", path);
        }
        // The existing /files route still answers.
        let r = get(&state, &format!("/files/sessions/{}/session.json", ID)).await;
        assert_eq!(r.status(), StatusCode::OK);
    }
}
