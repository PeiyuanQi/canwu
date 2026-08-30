//! Small local static server used by interactive trace-producing examples.

use super::trace::{TRACE_MANIFEST_FILE, TRACE_STEPS_FILE};
use canwu_api::{BoundaryRecord, canonical_hash};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};
use serde_json::{Value, json};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
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
    let viewer_root = workspace_root.join("tools").join("trace-viewer");
    let artifacts_root = workspace_root.join("artifacts").join("traces");
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
        .spawn(move || {
            serve(
                &listener,
                &viewer_root,
                &artifacts_root,
                &trace_root,
                &server_shutdown,
            );
        })?;
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

fn serve(
    listener: &TcpListener,
    viewer_root: &Path,
    artifacts_root: &Path,
    trace_root: &Path,
    shutdown: &AtomicBool,
) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match stream {
            Ok(stream) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let connection_viewer_root = viewer_root.to_path_buf();
                let connection_artifacts_root = artifacts_root.to_path_buf();
                let connection_trace_root = trace_root.to_path_buf();
                let _ = thread::Builder::new()
                    .name("canwu-trace-viewer-connection".to_owned())
                    .spawn(move || {
                        serve_connection(
                            stream,
                            connection_viewer_root,
                            connection_artifacts_root,
                            connection_trace_root,
                        );
                    });
            }
            Err(_) => break,
        }
    }
}

// The owned root is moved into the connection worker so the worker can outlive
// the accept loop without borrowing the server thread's stack.
#[allow(clippy::needless_pass_by_value)]
fn serve_connection(
    mut stream: TcpStream,
    viewer_root: PathBuf,
    artifacts_root: PathBuf,
    trace_root: PathBuf,
) {
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
    let range_start = request_range_start(&request);
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
    if relative_path.contains('\\') {
        write_response(&mut stream, 403, "text/plain; charset=utf-8", b"Forbidden");
        return;
    }
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
    if relative_path == "__canwu_trace/verify.json" {
        serve_trace_verification(&mut stream, &trace_root);
        return;
    }
    let (requested_path, allowed_root) =
        if let Some(trace_path) = relative_path.strip_prefix("__canwu_trace/") {
            (trace_root.join(trace_path), trace_root.as_path())
        } else if let Some(viewer_path) = relative_path.strip_prefix("tools/trace-viewer/") {
            (viewer_root.join(viewer_path), viewer_root.as_path())
        } else if let Some(artifact_path) = relative_path.strip_prefix("artifacts/traces/") {
            (artifacts_root.join(artifact_path), artifacts_root.as_path())
        } else {
            write_response(&mut stream, 403, "text/plain; charset=utf-8", b"Forbidden");
            return;
        };
    let Ok(mut file_path) = fs::canonicalize(requested_path) else {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        return;
    };
    if file_path.is_dir() {
        file_path = file_path.join("index.html");
    }
    if !file_path.starts_with(allowed_root) || !file_path.is_file() {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        return;
    }
    serve_file(&mut stream, &file_path, range_start);
}

#[derive(Deserialize)]
struct BoundaryFrame {
    boundary: BoundaryRecord,
}

struct BoundaryHashMaterial<'a>(&'a BoundaryRecord);

impl Serialize for BoundaryHashMaterial<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let record = self.0;
        let mut material = serializer.serialize_struct("BoundaryHashMaterial", 18)?;
        material.serialize_field("id", &record.id)?;
        material.serialize_field("at", &record.at)?;
        material.serialize_field("correlation_id", &record.correlation_id)?;
        material.serialize_field("cadences", &record.cadences)?;
        if !record.admitted_attempts.is_empty() {
            material.serialize_field("admitted_attempts", &record.admitted_attempts)?;
        }
        material.serialize_field("admitted_commands", &record.admitted_commands)?;
        if !record.admitted_ingress.is_empty() {
            material.serialize_field("admitted_ingress", &record.admitted_ingress)?;
        }
        if !record.generated_ingress.is_empty() {
            material.serialize_field("generated_ingress", &record.generated_ingress)?;
        }
        material.serialize_field("admitted_events", &record.admitted_events)?;
        material.serialize_field("reservation_offers", &record.reservation_offers)?;
        material.serialize_field("reservation_requests", &record.reservation_requests)?;
        material.serialize_field("allocations", &record.allocations)?;
        material.serialize_field("random_draws", &record.random_draws)?;
        material.serialize_field("changes", &record.changes)?;
        if !record.record_changes.is_empty() {
            material.serialize_field("record_changes", &record.record_changes)?;
        }
        material.serialize_field("emissions", &record.emissions)?;
        material.serialize_field("state_hash", &record.state_hash)?;
        material.serialize_field("previous_hash", &record.previous_hash)?;
        material.end()
    }
}

fn serve_trace_verification(stream: &mut TcpStream, trace_root: &Path) {
    let result = verify_trace_boundaries(trace_root);
    let body = serde_json::to_vec(&result).unwrap_or_else(|_| b"{\"verified\":false}".to_vec());
    write_response(stream, 200, "application/json; charset=utf-8", &body);
}

fn verify_trace_boundaries(trace_root: &Path) -> Value {
    let Ok(file) = fs::File::open(trace_root.join(TRACE_STEPS_FILE)) else {
        return json!({"verified": false, "frames_checked": 0, "errors": ["steps.jsonl could not be opened"]});
    };
    let mut errors = Vec::new();
    let mut frames_checked = 0usize;
    let mut previous_hash: Option<String> = None;
    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let Ok(line) = line else {
            errors.push(format!("line {} could not be read", line_index + 1));
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<BoundaryFrame>(&line) else {
            errors.push(format!("line {} is not valid JSON", line_index + 1));
            break;
        };
        let boundary = frame.boundary;
        let expected = boundary.hash.as_str();
        match canonical_hash("canwu.boundary-record.v1", &BoundaryHashMaterial(&boundary)) {
            Ok(computed) if computed == expected => {}
            Ok(_) => errors.push(format!("line {} boundary BLAKE3 mismatch", line_index + 1)),
            Err(error) => errors.push(format!("line {} hash failed: {error}", line_index + 1)),
        }
        if let Some(previous) = previous_hash.as_deref()
            && boundary.previous_hash != previous
        {
            errors.push(format!("line {} boundary chain mismatch", line_index + 1));
        }
        previous_hash = Some(expected.to_owned());
        frames_checked = frames_checked.saturating_add(1);
    }
    json!({
        "verified": errors.is_empty(),
        "algorithm": "BLAKE3",
        "scope": "boundary content and previous-hash chain",
        "frames_checked": frames_checked,
        "errors": errors,
    })
}

fn serve_file(stream: &mut TcpStream, file_path: &Path, range_start: Option<u64>) {
    if let Some(start) = range_start {
        let Ok(mut file) = fs::File::open(file_path) else {
            write_response(stream, 500, "text/plain; charset=utf-8", b"Read Failed");
            return;
        };
        let Ok(file_length) = file.metadata().map(|metadata| metadata.len()) else {
            write_response(stream, 500, "text/plain; charset=utf-8", b"Read Failed");
            return;
        };
        if start >= file_length {
            let headers = format!(
                "Accept-Ranges: bytes\r\nContent-Range: bytes */{file_length}\r\nX-Canwu-File-Length: {file_length}\r\n"
            );
            write_response_with_headers(
                stream,
                416,
                "text/plain; charset=utf-8",
                b"Range Not Satisfiable",
                &headers,
            );
            return;
        }
        if file.seek(SeekFrom::Start(start)).is_err() {
            write_response(stream, 500, "text/plain; charset=utf-8", b"Read Failed");
            return;
        }
        let mut body = Vec::new();
        if file.read_to_end(&mut body).is_err() {
            write_response(stream, 500, "text/plain; charset=utf-8", b"Read Failed");
            return;
        }
        let end = file_length - 1;
        let headers = format!(
            "Accept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{file_length}\r\nX-Canwu-File-Length: {file_length}\r\n"
        );
        write_response_with_headers(stream, 206, content_type(file_path), &body, &headers);
        return;
    }
    let Ok(body) = fs::read(file_path) else {
        write_response(stream, 500, "text/plain; charset=utf-8", b"Read Failed");
        return;
    };
    write_response_with_headers(
        stream,
        200,
        content_type(file_path),
        &body,
        "Accept-Ranges: bytes\r\n",
    );
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    write_response_with_headers(stream, status, content_type, body, "");
}

fn write_response_with_headers(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &str,
) {
    let status_text = match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra_headers}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

fn request_range_start(request: &str) -> Option<u64> {
    request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.eq_ignore_ascii_case("range") {
            return None;
        }
        value
            .trim()
            .strip_prefix("bytes=")?
            .strip_suffix('-')?
            .parse()
            .ok()
    })
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
        let ranged = http_get_with_headers(
            viewer.address,
            "/__canwu_trace/steps.jsonl",
            "Range: bytes=1-\r\n",
        );
        assert!(ranged.starts_with("HTTP/1.1 206 Partial Content"));
        assert!(ranged.contains("Content-Range: bytes 1-2/3"));
        assert!(ranged.ends_with("}\n"));
        let exhausted = http_get_with_headers(
            viewer.address,
            "/__canwu_trace/steps.jsonl",
            "Range: bytes=3-\r\n",
        );
        assert!(exhausted.starts_with("HTTP/1.1 416 Range Not Satisfiable"));
        assert!(exhausted.contains("X-Canwu-File-Length: 3"));
        let forbidden = http_get(viewer.address, "/Cargo.toml");
        assert!(forbidden.starts_with("HTTP/1.1 403 Forbidden"));
        let encoded_windows_traversal = http_get(
            viewer.address,
            "/tools/trace-viewer/%5c..%5c..%5cCargo.toml",
        );
        assert!(encoded_windows_traversal.starts_with("HTTP/1.1 403 Forbidden"));

        viewer.shutdown();
        assert!(TcpStream::connect(viewer.address).is_err());
        fs::remove_dir_all(trace_directory).expect("test trace directory should be removed");
    }

    fn http_get(address: SocketAddr, path: &str) -> String {
        http_get_with_headers(address, path, "")
    }

    fn http_get_with_headers(address: SocketAddr, path: &str, headers: &str) -> String {
        let mut stream = TcpStream::connect(address).expect("viewer should accept connections");
        stream
            .write_all(
                format!(
                    "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Connection: close\r\n\r\n"
                )
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
