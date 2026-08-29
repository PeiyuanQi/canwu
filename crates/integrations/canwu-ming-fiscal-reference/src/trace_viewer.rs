//! Small local static server used by interactive trace-producing examples.

use super::trace::{TRACE_MANIFEST_FILE, TRACE_STEPS_FILE};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug)]
pub enum TraceViewerError {
    Io(io::Error),
    InvalidWorkspace(String),
    InvalidTrace(String),
    Browser(String),
}

impl Display for TraceViewerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "trace viewer I/O failed: {error}"),
            Self::InvalidWorkspace(message) => {
                write!(formatter, "invalid trace viewer workspace: {message}")
            }
            Self::InvalidTrace(message) => {
                write!(formatter, "invalid trace viewer trace: {message}")
            }
            Self::Browser(message) => {
                write!(formatter, "could not open trace viewer browser: {message}")
            }
        }
    }
}

impl std::error::Error for TraceViewerError {}

impl From<io::Error> for TraceViewerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A local viewer server. Keep this value alive while the browser is in use.
pub struct TraceViewerHandle {
    url: String,
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl TraceViewerHandle {
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Open the viewer with the platform's default browser.
    pub fn open_browser(&self) -> Result<(), TraceViewerError> {
        open_browser(self.url())
    }

    /// Keep serving until the process is interrupted.
    pub fn wait(mut self) {
        self.join_server();
    }

    /// Stop serving and wait for the local server thread to exit.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        self.join_server();
    }

    fn join_server(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

impl Drop for TraceViewerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start a localhost-only static server and point the viewer at one trace.
pub fn start_trace_viewer(
    workspace_root: impl AsRef<Path>,
    trace_directory: impl AsRef<Path>,
    requested_port: u16,
) -> Result<TraceViewerHandle, TraceViewerError> {
    let workspace_root = canonicalize_path(workspace_root.as_ref())?;
    let trace_directory = canonicalize_path(trace_directory.as_ref())?;
    let viewer_index = workspace_root
        .join("tools")
        .join("trace-viewer")
        .join("index.html");
    if !viewer_index.is_file() {
        return Err(TraceViewerError::InvalidWorkspace(format!(
            "missing {}",
            viewer_index.display()
        )));
    }
    if !trace_directory.is_dir() {
        return Err(TraceViewerError::InvalidTrace(format!(
            "trace directory does not exist: {}",
            trace_directory.display()
        )));
    }
    for file in [TRACE_MANIFEST_FILE, TRACE_STEPS_FILE] {
        if !trace_directory.join(file).is_file() {
            return Err(TraceViewerError::InvalidTrace(format!(
                "trace directory is missing {file}: {}",
                trace_directory.display()
            )));
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", requested_port))?;
    let port = listener.local_addr()?.port();
    let trace_root = trace_directory;
    let address = listener.local_addr()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let server = thread::Builder::new()
        .name("canwu-trace-viewer".to_owned())
        .spawn(move || serve(&listener, &workspace_root, &trace_root, &server_shutdown))?;
    let url = format!("http://127.0.0.1:{port}/tools/trace-viewer/?trace=/__canwu_trace/");
    Ok(TraceViewerHandle {
        url,
        address,
        shutdown,
        server: Some(server),
    })
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, TraceViewerError> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(fs::canonicalize(resolved)?)
}

fn serve(listener: &TcpListener, root: &Path, trace_root: &Path, shutdown: &AtomicBool) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match stream {
            Ok(stream) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let connection_root = root.to_path_buf();
                let connection_trace_root = trace_root.to_path_buf();
                let _ = thread::Builder::new()
                    .name("canwu-trace-viewer-connection".to_owned())
                    .spawn(move || {
                        serve_connection(stream, connection_root, connection_trace_root);
                    });
            }
            Err(_) => break,
        }
    }
}

// The owned root is moved into the connection worker so the worker can outlive
// the accept loop without borrowing the server thread's stack.
#[allow(clippy::needless_pass_by_value)]
fn serve_connection(mut stream: TcpStream, root: PathBuf, trace_root: PathBuf) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut request = [0_u8; 16 * 1024];
    let mut bytes_read = 0;
    while bytes_read < request.len() {
        let Ok(read) = stream.read(&mut request[bytes_read..]) else {
            return;
        };
        if read == 0 {
            break;
        }
        bytes_read += read;
        if request[..bytes_read]
            .windows(4)
            .any(|window| window == b"\r\n\r\n")
        {
            break;
        }
    }
    if bytes_read == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&request[..bytes_read]);
    let Some(request_line) = request.lines().next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return;
    };
    let Some(target) = parts.next() else {
        return;
    };
    if method != "GET" {
        write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"Method Not Allowed",
        );
        return;
    }
    let path = target.split('?').next().unwrap_or("/");
    let Some(relative_path) = decode_request_path(path) else {
        write_response(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            b"Bad Request",
        );
        return;
    };
    let relative_path = relative_path.trim_start_matches('/');
    if !relative_path.starts_with("tools/trace-viewer/")
        && !relative_path.starts_with("artifacts/traces/")
        && !relative_path.starts_with("__canwu_trace/")
    {
        write_response(&mut stream, 403, "text/plain; charset=utf-8", b"Forbidden");
        return;
    }
    if relative_path.split('/').any(|component| component == "..") {
        write_response(&mut stream, 403, "text/plain; charset=utf-8", b"Forbidden");
        return;
    }
    let requested_path = if let Some(trace_path) = relative_path.strip_prefix("__canwu_trace/") {
        trace_root.join(trace_path)
    } else {
        root.join(if relative_path.is_empty() {
            "tools/trace-viewer/index.html"
        } else {
            relative_path
        })
    };
    let Ok(mut file_path) = fs::canonicalize(requested_path) else {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        return;
    };
    if file_path.is_dir() {
        file_path = file_path.join("index.html");
    }
    let allowed_root = if relative_path.starts_with("__canwu_trace/") {
        &trace_root
    } else {
        &root
    };
    if !file_path.starts_with(allowed_root) || !file_path.is_file() {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        return;
    }
    let Ok(body) = fs::read(&file_path) else {
        write_response(
            &mut stream,
            500,
            "text/plain; charset=utf-8",
            b"Read Failed",
        );
        return;
    };
    write_response(&mut stream, 200, content_type(&file_path), &body);
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("jsonl" | "ndjson") => "application/x-ndjson; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn decode_request_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn open_browser(url: &str) -> Result<(), TraceViewerError> {
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    result
        .map(|_| ())
        .map_err(|error| TraceViewerError::Browser(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn viewer_mount_serves_trace_files_and_shutdowns_cleanly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("crate should be nested below the workspace root");
        let trace_directory =
            std::env::temp_dir().join(format!("canwu-trace-viewer-test-{}", std::process::id()));
        fs::create_dir_all(&trace_directory).expect("test trace directory should be created");
        fs::write(
            trace_directory.join(TRACE_MANIFEST_FILE),
            br#"{"status":"complete"}"#,
        )
        .expect("test manifest should be written");
        fs::write(trace_directory.join(TRACE_STEPS_FILE), b"{}\n")
            .expect("test steps should be written");

        let mut viewer =
            start_trace_viewer(workspace, &trace_directory, 0).expect("viewer should start");
        let response = http_get(viewer.address, "/__canwu_trace/manifest.json");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"status\":\"complete\""));
        let forbidden = http_get(viewer.address, "/Cargo.toml");
        assert!(forbidden.starts_with("HTTP/1.1 403 Forbidden"));

        viewer.shutdown();
        assert!(TcpStream::connect(viewer.address).is_err());
        fs::remove_dir_all(trace_directory).expect("test trace directory should be removed");
    }

    fn http_get(address: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).expect("viewer should accept connections");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .expect("request should be written");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response should be readable");
        response
    }
}
