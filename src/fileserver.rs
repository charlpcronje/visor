//! Simple static file server. Serves files from a directory on localhost.
//! If index.html exists, serves it at /. Otherwise shows a directory listing.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn run(root: &str, port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let root = Arc::new(PathBuf::from(root).canonicalize().unwrap_or_else(|_| PathBuf::from(root)));

    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("Failed to start file server on {addr}: {e}"))?;

    eprintln!("Serving files from {} on http://{addr}", root.display());

    loop {
        let request = match server.recv() {
            Ok(r) => r,
            Err(_) => break,
        };

        let url_path = percent_decode(request.url());
        let url_path = url_path.trim_start_matches('/');

        // Resolve to filesystem path
        let fs_path = if url_path.is_empty() {
            root.as_ref().to_path_buf()
        } else {
            root.join(url_path)
        };

        // Security: ensure resolved path is under root
        let canonical = fs_path.canonicalize().unwrap_or_else(|_| fs_path.clone());
        if !canonical.starts_with(root.as_ref()) {
            let resp = tiny_http::Response::from_string("403 Forbidden")
                .with_status_code(403);
            let _ = request.respond(resp);
            continue;
        }

        if canonical.is_dir() {
            // Check for index.html
            let index = canonical.join("index.html");
            if index.is_file() {
                serve_file(request, &index);
            } else {
                serve_directory_listing(request, &canonical, &root, url_path);
            }
        } else if canonical.is_file() {
            serve_file(request, &canonical);
        } else {
            let resp = tiny_http::Response::from_string("404 Not Found")
                .with_status_code(404);
            let _ = request.respond(resp);
        }
    }

    Ok(())
}

fn serve_file(request: tiny_http::Request, path: &Path) {
    let mime = guess_mime(path);
    match std::fs::File::open(path) {
        Ok(file) => {
            let header = tiny_http::Header::from_bytes("Content-Type", mime).unwrap();
            let resp = tiny_http::Response::from_file(file)
                .with_header(header);
            let _ = request.respond(resp);
        }
        Err(_) => {
            let resp = tiny_http::Response::from_string("500 Internal Server Error")
                .with_status_code(500);
            let _ = request.respond(resp);
        }
    }
}

fn serve_directory_listing(
    request: tiny_http::Request,
    dir: &Path,
    _root: &Path,
    url_path: &str,
) {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html><html><head><meta charset='utf-8'>");
    html.push_str("<title>Directory listing</title>");
    html.push_str("<style>");
    html.push_str("body{font-family:'Segoe UI',sans-serif;background:#0d1117;color:#e6edf3;padding:20px;max-width:800px;margin:0 auto}");
    html.push_str("h1{color:#58a6ff;font-size:1.3em}");
    html.push_str("a{color:#58a6ff;text-decoration:none}a:hover{text-decoration:underline}");
    html.push_str(".entry{padding:6px 0;border-bottom:1px solid #21262d;display:flex;gap:10px}");
    html.push_str(".icon{width:20px;text-align:center}.size{color:#8b949e;min-width:80px;text-align:right}");
    html.push_str("</style></head><body>");

    let display_path = if url_path.is_empty() { "/" } else { url_path };
    html.push_str(&format!("<h1>Index of /{}</h1>", html_escape(display_path)));

    // Parent directory link
    if !url_path.is_empty() {
        let parent = url_path.rfind('/').map(|i| &url_path[..i]).unwrap_or("");
        html.push_str(&format!(
            "<div class='entry'><span class='icon'>📁</span><a href='/{}'>../</a><span class='size'>-</span></div>",
            parent
        ));
    }

    // List entries
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| {
            let is_file = e.file_type().map(|t| t.is_file()).unwrap_or(true);
            (is_file, e.file_name().to_ascii_lowercase())
        });

        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir {
                "-".to_string()
            } else {
                entry.metadata().map(|m| format_size(m.len())).unwrap_or("-".to_string())
            };
            let icon = if is_dir { "📁" } else { "📄" };
            let href = if url_path.is_empty() {
                format!("/{}", &name)
            } else {
                format!("/{}/{}", url_path, &name)
            };
            let display = if is_dir {
                format!("{}/", name)
            } else {
                name
            };
            html.push_str(&format!(
                "<div class='entry'><span class='icon'>{icon}</span><a href='{}'>{}</a><span class='size'>{size}</span></div>",
                html_escape(&href),
                html_escape(&display),
            ));
        }
    }

    html.push_str("</body></html>");

    let header = tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
    let resp = tiny_http::Response::from_string(html).with_header(header);
    let _ = request.respond(resp);
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "xml" => "application/xml; charset=utf-8",
        "txt" | "md" | "log" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().unwrap_or(b'0');
            let lo = bytes.next().unwrap_or(b'0');
            let val = hex(hi) * 16 + hex(lo);
            result.push(val as char);
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}
