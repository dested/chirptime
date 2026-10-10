const ORIGIN = "http://localhost:9120";

export function mediaUrl(relPath: string): string {
  return `${ORIGIN}/files/${relPath}`;
}

/**
 * One frame of a track at an instant, as a JPEG (`fileserver.rs` /thumb).
 * 404 inside a gap or a cut. Round `epochMs` (see `thumbInstant`) so the cache hits.
 */
export function thumbUrl(sessionId: string, trackId: string, epochMs: number, width: number): string {
  return `${ORIGIN}/thumb/${encodeURIComponent(sessionId)}/${encodeURIComponent(trackId)}/${Math.floor(epochMs)}.jpg?w=${Math.round(width)}`;
}
