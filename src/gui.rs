use anyhow::{Context, Result};
use std::sync::Arc;
use std::thread;

use std::io::{Read, Seek, SeekFrom};

use crate::client;
use crate::models::{Request, LOG_DIR};

pub fn run(port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = Arc::new(
        tiny_http::Server::http(&addr)
            .map_err(|e| anyhow::anyhow!("Failed to start HTTP server on {addr}: {e}"))?,
    );

    println!("Visor GUI running at http://{addr}");

    // Spawn HTTP API server thread
    let server_clone = Arc::clone(&server);
    thread::spawn(move || {
        serve_api(server_clone);
    });

    // Open WebView2 window
    let url = format!("http://127.0.0.1:{port}");
    open_webview(&url)
}

fn serve_api(server: Arc<tiny_http::Server>) {
    loop {
        let request = match server.recv() {
            Ok(r) => r,
            Err(_) => break,
        };

        let path = request.url().to_string();
        let method = request.method().to_string();

        let (status, content_type, body) = match (method.as_str(), path.as_str()) {
            ("GET", "/") => (200, "text/html", dashboard_html().to_string()),
            ("GET", "/api/status") => handle_api_status(),
            ("GET", "/api/list") => handle_api_list(),
            ("POST", p) if p.starts_with("/api/stop/") => {
                let name = p.strip_prefix("/api/stop/").unwrap_or("");
                handle_api_stop(name)
            }
            ("POST", "/api/stop-all") => handle_api_stop_all(),
            ("POST", "/api/cleanup") => handle_api_cleanup(),
            ("GET", p) if p.starts_with("/api/log-content/") => {
                handle_api_log_content(p)
            }
            _ => (404, "text/plain", "Not found".to_string()),
        };

        let response = tiny_http::Response::from_string(body)
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", content_type).unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
            );
        let _ = request.respond(response);
    }
}

fn handle_api_status() -> (i32, &'static str, String) {
    match client::send_request(Request::Status) {
        Ok(resp) => (200, "application/json", serde_json::to_string(&resp).unwrap_or_default()),
        Err(e) => (500, "application/json", format!(r#"{{"error":"{}"}}"#, e)),
    }
}

fn handle_api_list() -> (i32, &'static str, String) {
    match client::send_request(Request::List {
        agent: None,
        group: None,
        json: true,
    }) {
        Ok(resp) => (200, "application/json", serde_json::to_string(&resp).unwrap_or_default()),
        Err(e) => (500, "application/json", format!(r#"{{"error":"{}"}}"#, e)),
    }
}

fn handle_api_stop(name: &str) -> (i32, &'static str, String) {
    let decoded = urlencoding_decode(name);
    match client::send_request(Request::Stop {
        name: Some(decoded),
        id: None,
        pid: None,
        agent: None,
        group: None,
        code: None,
    }) {
        Ok(resp) => (200, "application/json", serde_json::to_string(&resp).unwrap_or_default()),
        Err(e) => (500, "application/json", format!(r#"{{"error":"{}"}}"#, e)),
    }
}

fn handle_api_stop_all() -> (i32, &'static str, String) {
    match client::send_request(Request::StopAll { code: None }) {
        Ok(resp) => (200, "application/json", serde_json::to_string(&resp).unwrap_or_default()),
        Err(e) => (500, "application/json", format!(r#"{{"error":"{}"}}"#, e)),
    }
}

fn handle_api_cleanup() -> (i32, &'static str, String) {
    match client::send_request(Request::Cleanup) {
        Ok(resp) => (200, "application/json", serde_json::to_string(&resp).unwrap_or_default()),
        Err(e) => (500, "application/json", format!(r#"{{"error":"{}"}}"#, e)),
    }
}

fn handle_api_log_content(path: &str) -> (i32, &'static str, String) {
    // Path format: /api/log-content/<id>?offset=<n>
    let path_no_query = path.split('?').next().unwrap_or(path);
    let id = path_no_query.strip_prefix("/api/log-content/").unwrap_or("");
    let id = urlencoding_decode(id);

    // Parse offset from query string
    let offset: u64 = path
        .split('?')
        .nth(1)
        .and_then(|qs| {
            qs.split('&')
                .find(|p| p.starts_with("offset="))
                .and_then(|p| p.strip_prefix("offset="))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0);

    // Build log file path
    let log_path = format!("{}\\{}.log", LOG_DIR, id);

    match std::fs::File::open(&log_path) {
        Ok(mut file) => {
            let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if offset >= file_len {
                // No new content
                let body = format!(r#"{{"content":"","new_offset":{}}}"#, file_len);
                return (200, "application/json", body);
            }
            let _ = file.seek(SeekFrom::Start(offset));
            let mut buf = Vec::new();
            let _ = file.read_to_end(&mut buf);
            let new_offset = offset + buf.len() as u64;
            // Escape the content for JSON
            let content = String::from_utf8_lossy(&buf).to_string();
            let escaped = serde_json::to_string(&content).unwrap_or_else(|_| "\"\"".to_string());
            let body = format!(r#"{{"content":{},"new_offset":{}}}"#, escaped, new_offset);
            (200, "application/json", body)
        }
        Err(_) => {
            (404, "application/json", r#"{"error":"Log file not found"}"#.to_string())
        }
    }
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(b'0');
            let lo = chars.next().unwrap_or(b'0');
            let val = hex_val(hi) * 16 + hex_val(lo);
            result.push(val as char);
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Wraps a Win32 HWND so it implements HasWindowHandle for wry.
struct HwndWrapper(isize);

impl wry::raw_window_handle::HasWindowHandle for HwndWrapper {
    fn window_handle(
        &self,
    ) -> std::result::Result<
        wry::raw_window_handle::WindowHandle<'_>,
        wry::raw_window_handle::HandleError,
    > {
        let raw = wry::raw_window_handle::RawWindowHandle::Win32(
            wry::raw_window_handle::Win32WindowHandle::new(
                std::num::NonZero::new(self.0).unwrap(),
            ),
        );
        Ok(unsafe { wry::raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

fn open_webview(url: &str) -> Result<()> {
    use wry::WebViewBuilder;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::*;

    unsafe {
        let class_name = w!("VisorGUI");
        let window_name = w!("Visor Dashboard");

        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
            .unwrap_or_default();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_name,
            hInstance: hinstance.into(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            window_name,
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1200,
            800,
            None,
            None,
            wc.hInstance,
            None,
        )
        .context("CreateWindowExW failed")?;

        let wrapper = HwndWrapper(hwnd.0 as isize);

        let _webview = WebViewBuilder::new()
            .with_url(url)
            .build_as_child(&wrapper)
            .context("Failed to create WebView2")?;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    Ok(())
}

unsafe extern "system" fn wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_SIZE => {
            // Let the webview handle its own resizing
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn dashboard_html() -> &'static str {
    include_str!("dashboard.html")
}
