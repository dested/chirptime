use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;
use warp::http::{Response, StatusCode};
use warp::hyper::Body;
use tokio::io::AsyncReadExt;

use crate::capture;
use crate::state::AppState;

pub async fn start(state: Arc<Mutex<AppState>>, port: u16) {
    let state = warp::any().map(move || state.clone());

    let files = warp::path("files")
        .and(warp::path::tail())
        .and(warp::header::optional::<String>("range"))
        .and(state)
        .and_then(serve_file);

    warp::serve(files).run(([127, 0, 0, 1], port)).await;
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
