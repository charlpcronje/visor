//! Static file server with optional TypeScript transpilation.
//! If index.html exists, serves it at /. Otherwise shows a directory listing.
//! With `ts: true`, .ts and .tsx files are transpiled to JS on the fly using
//! the first available transpiler (esbuild > swc > bun > tsc).

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

struct TsCache {
    entries: HashMap<PathBuf, CacheEntry>,
}

struct CacheEntry {
    js: String,
    modified: SystemTime,
}

pub fn run(root: &str, port: u16, ts: bool, transpiler_path: Option<&str>) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let root = Arc::new(PathBuf::from(root).canonicalize().unwrap_or_else(|_| PathBuf::from(root)));

    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("Failed to start file server on {addr}: {e}"))?;

    // Use the pre-resolved transpiler path if given, otherwise try detection
    let transpiler = if ts {
        transpiler_path
            .map(|p| p.to_string())
            .or_else(|| detect_transpiler())
    } else {
        None
    };
    if ts {
        match &transpiler {
            Some(t) => eprintln!("TypeScript transpilation enabled (using {t})"),
            None => eprintln!("Warning: --ts enabled but no transpiler found (install esbuild, swc, or bun)"),
        }
    }

    eprintln!("Serving files from {} on http://{addr}", root.display());

    let cache: Arc<Mutex<TsCache>> = Arc::new(Mutex::new(TsCache {
        entries: HashMap::new(),
    }));

    loop {
        let request = match server.recv() {
            Ok(r) => r,
            Err(_) => break,
        };

        let url_path = percent_decode(request.url());
        // Strip query string
        let url_path = url_path.split('?').next().unwrap_or(&url_path);
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
            let index = canonical.join("index.html");
            if index.is_file() {
                serve_file(request, &index, &transpiler, &cache);
            } else {
                serve_directory_listing(request, &canonical, &root, url_path);
            }
        } else if canonical.is_file() {
            serve_file(request, &canonical, &transpiler, &cache);
        } else {
            // If .ts was requested without extension, try adding it
            let ts_path = canonical.with_extension("ts");
            let tsx_path = canonical.with_extension("tsx");
            if ts && ts_path.is_file() {
                serve_file(request, &ts_path, &transpiler, &cache);
            } else if ts && tsx_path.is_file() {
                serve_file(request, &tsx_path, &transpiler, &cache);
            } else {
                let resp = tiny_http::Response::from_string("404 Not Found")
                    .with_status_code(404);
                let _ = request.respond(resp);
            }
        }
    }

    Ok(())
}

fn serve_file(
    request: tiny_http::Request,
    path: &Path,
    transpiler: &Option<String>,
    cache: &Arc<Mutex<TsCache>>,
) {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let is_ts = matches!(ext, "ts" | "tsx" | "mts" | "cts");

    // Transpile TypeScript if we have a transpiler
    if is_ts {
        if let Some(ref t) = transpiler {
            if let Some(js) = transpile_cached(path, t, cache) {
                let header = tiny_http::Header::from_bytes(
                    "Content-Type", "application/javascript; charset=utf-8",
                ).unwrap();
                let resp = tiny_http::Response::from_string(js).with_header(header);
                let _ = request.respond(resp);
                return;
            }
        }
        // No transpiler or transpile failed — serve raw with JS MIME type
        // (browser modules might still work with type annotations stripped by import maps)
    }

    let mime = guess_mime(path);
    match std::fs::File::open(path) {
        Ok(file) => {
            let header = tiny_http::Header::from_bytes("Content-Type", mime).unwrap();
            let resp = tiny_http::Response::from_file(file).with_header(header);
            let _ = request.respond(resp);
        }
        Err(_) => {
            let resp = tiny_http::Response::from_string("500 Internal Server Error")
                .with_status_code(500);
            let _ = request.respond(resp);
        }
    }
}

/// Transpile a .ts/.tsx file, using a cache keyed on file path + modification time.
fn transpile_cached(
    path: &Path,
    transpiler: &str,
    cache: &Arc<Mutex<TsCache>>,
) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;

    // Check cache
    {
        let cache = cache.lock().ok()?;
        if let Some(entry) = cache.entries.get(path) {
            if entry.modified == modified {
                return Some(entry.js.clone());
            }
        }
    }

    // Transpile
    let js = run_transpiler(transpiler, path)?;

    // Store in cache
    {
        if let Ok(mut cache) = cache.lock() {
            cache.entries.insert(path.to_path_buf(), CacheEntry {
                js: js.clone(),
                modified,
            });
        }
    }

    Some(js)
}

/// Run the transpiler on a file and return the JavaScript output.
fn run_transpiler(transpiler: &str, path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let path_str = raw.strip_prefix(r"\\?\").unwrap_or(&raw);

    // Determine which transpiler this is (could be a full path like C:\...\esbuild.cmd)
    let tname = std::path::Path::new(transpiler)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(transpiler);

    // Run through cmd /c so .cmd wrappers (npm global installs) work
    let output = match tname {
        "esbuild" => {
            std::process::Command::new("cmd")
                .args(["/c", transpiler, "--bundle=false", "--format=esm"])
                .arg(path_str)
                .output()
                .ok()?
        }
        "swc" => {
            std::process::Command::new("cmd")
                .args(["/c", transpiler, "compile", "--filename"])
                .arg(path_str)
                .output()
                .ok()?
        }
        "bun" => {
            std::process::Command::new("cmd")
                .args(["/c", transpiler, "build", "--no-bundle", "--target=browser"])
                .arg(path_str)
                .output()
                .ok()?
        }
        "tsc" => {
            // tsc doesn't output to stdout easily, use a temp approach
            // Write to a temp file and read it back
            let out_path = path.with_extension("js.tmp");
            let result = std::process::Command::new("cmd")
                .args(["/c", transpiler,
                    "--target", "ES2020",
                    "--module", "ES2020",
                    "--moduleResolution", "node",
                    "--outFile",
                ])
                .arg(&out_path)
                .arg(path_str)
                .output()
                .ok()?;
            if result.status.success() {
                let js = std::fs::read_to_string(&out_path).ok()?;
                let _ = std::fs::remove_file(&out_path);
                return Some(js);
            }
            // tsc failed, try just stripping types with --isolatedModules
            return None;
        }
        _ => return None,
    };

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("Transpile error for {}: {}", path_str, err);
        None
    }
}

/// Detect the first available TypeScript transpiler.
fn detect_transpiler() -> Option<String> {
    for cmd in &["esbuild", "swc", "bun", "tsc"] {
        if std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(cmd.to_string());
        }
    }
    None
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

    if !url_path.is_empty() {
        let parent = url_path.rfind('/').map(|i| &url_path[..i]).unwrap_or("");
        html.push_str(&format!(
            "<div class='entry'><span class='icon'>📁</span><a href='/{}'>../</a><span class='size'>-</span></div>",
            parent
        ));
    }

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
        "ts" | "mts" => "application/javascript; charset=utf-8", // serve as JS even raw
        "tsx" => "application/javascript; charset=utf-8",
        "jsx" => "application/javascript; charset=utf-8",
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
        "map" => "application/json",
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
