//! Pure-Rust local translation / API gateway (no Node.js).
//! Listens on 127.0.0.1:18989, scrubbing Chinese text before forwarding
//! to api.anthropic.com via system curl (TLS handled by the OS toolchain).

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static PROXY_RUNNING: AtomicBool = AtomicBool::new(false);
static REQ_COUNTER: AtomicU32 = AtomicU32::new(0);

const PORT: u16 = 18989;
const TARGET_HOST: &str = "api.anthropic.com";

fn shield_dir() -> PathBuf {
    let mut home = env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    home.push(".claude-shield");
    home
}

fn proxy_log_path() -> PathBuf {
    shield_dir().join("proxy.log")
}

fn proxy_pid_path() -> PathBuf {
    shield_dir().join("proxy.pid")
}

fn append_log(msg: &str) {
    let _ = fs::create_dir_all(shield_dir());
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(proxy_log_path())
    {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn translate_chunk(text: &str) -> Result<String, String> {
    if !contains_cjk(text) {
        return Ok(text.to_string());
    }
    let output = Command::new("curl")
        .args([
            "-s",
            "-G",
            "https://translate.googleapis.com/translate_a/single",
            "--data-urlencode",
            "client=gtx",
            "--data-urlencode",
            "sl=zh-CN",
            "--data-urlencode",
            "tl=en",
            "--data-urlencode",
            "dt=t",
            "--data-urlencode",
            &format!("q={}", text),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("translate curl failed".into());
    }
    let res = String::from_utf8_lossy(&output.stdout);
    // Response shape: [[["translated","src",...],...],...]
    if let Some(start) = res.find("[[[\"") {
        let after = &res[start + 4..];
        if let Some(end) = after.find("\",\"") {
            return Ok(after[..end].replace("\\\"", "\"").replace("\\\\", "\\"));
        }
    }
    // Fallback: walk consecutive ["translated","src" pairs and join translations
    let mut out = String::new();
    let bytes = res.as_bytes();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'"' {
            if let Ok((piece, next)) = read_json_string(bytes, i + 1) {
                // Heuristic: first string in each pair is the translation
                out.push_str(&piece);
                i = next;
                // Skip the source string if present
                while i < bytes.len() && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'"' {
                    if let Ok((_, next2)) = read_json_string(bytes, i) {
                        i = next2;
                    }
                }
                // Move to next array element after this pair's closing
                continue;
            }
        }
        i += 1;
        // Only take the first top-level batch
        if !out.is_empty() && bytes.get(i) == Some(&b']') {
            break;
        }
    }
    if !out.is_empty() {
        return Ok(out);
    }
    Err("translate parse failed".into())
}

fn translate_text(text: &str) -> Result<String, String> {
    if !contains_cjk(text) {
        return Ok(text.to_string());
    }
    // Preserve fenced code blocks and inline code
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let before = &rest[..start];
        result.push_str(&translate_plain(before)?);
        let after_fence = &rest[start + 3..];
        if let Some(end) = after_fence.find("```") {
            result.push_str("```");
            result.push_str(&after_fence[..end]);
            result.push_str("```");
            rest = &after_fence[end + 3..];
        } else {
            result.push_str(&rest[start..]);
            return Ok(result);
        }
    }
    result.push_str(&translate_plain(rest)?);
    Ok(result)
}

fn translate_plain(text: &str) -> Result<String, String> {
    if !contains_cjk(text) {
        return Ok(text.to_string());
    }
    // Split on inline `code`
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let before = &rest[..start];
        out.push_str(&translate_chunk(before)?);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            out.push('`');
            out.push_str(&after[..end]);
            out.push('`');
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[start..]);
            return Ok(out);
        }
    }
    out.push_str(&translate_chunk(rest)?);
    Ok(out)
}

/// Minimal JSON string rewriter for "content" / "text" string values.
fn translate_json_payload(body: &str) -> Result<String, String> {
    if !contains_cjk(body) {
        return Ok(body.to_string());
    }
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for "content" or "text" keys followed by string values
        if matches_key(bytes, i, b"\"content\"") || matches_key(bytes, i, b"\"text\"") {
            let key_len = if bytes[i + 1] == b'c' { 9 } else { 6 }; // "content" / "text"
            out.push_str(std::str::from_utf8(&bytes[i..i + key_len]).unwrap_or(""));
            i += key_len;
            // skip whitespace and colon
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b':' {
                out.push(':');
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                let (raw, next) = read_json_string(bytes, i)?;
                let translated = translate_text(&raw)?;
                if translated != raw {
                    let n = REQ_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
                    append_log(&format!(
                        "Translation #{}: {} -> {}",
                        n,
                        truncate(&raw, 80),
                        truncate(&translated, 80)
                    ));
                    let _ = writeln!(
                        std::io::stderr(),
                        "\x1b[36m[Shield Translate]\x1b[0m {} \x1b[32m->\x1b[0m {}",
                        truncate(&raw, 60),
                        truncate(&translated, 60)
                    );
                }
                out.push('"');
                out.push_str(&escape_json(&translated));
                out.push('"');
                i = next;
                continue;
            }
        }
        // Copy one UTF-8 character (not one byte) to preserve CJK / emoji.
        let ch = next_utf8_char(bytes, i)?;
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

fn matches_key(bytes: &[u8], i: usize, key: &[u8]) -> bool {
    i + key.len() <= bytes.len() && &bytes[i..i + key.len()] == key
}

fn next_utf8_char(bytes: &[u8], i: usize) -> Result<char, String> {
    let s = std::str::from_utf8(&bytes[i..]).map_err(|e| format!("invalid utf-8: {}", e))?;
    s.chars()
        .next()
        .ok_or_else(|| "unexpected end of input".into())
}

fn read_json_string(bytes: &[u8], start: usize) -> Result<(String, usize), String> {
    // start points at opening quote
    let mut i = start + 1;
    let mut raw = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            if i + 1 >= bytes.len() {
                return Err("bad escape".into());
            }
            let n = bytes[i + 1];
            match n {
                b'"' | b'\\' | b'/' => {
                    raw.push(n as char);
                    i += 2;
                }
                b'n' => {
                    raw.push('\n');
                    i += 2;
                }
                b'r' => {
                    raw.push('\r');
                    i += 2;
                }
                b't' => {
                    raw.push('\t');
                    i += 2;
                }
                b'u' if i + 5 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[i + 2..i + 6]).unwrap_or("0000");
                    if let Ok(cp) = u32::from_str_radix(hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            raw.push(ch);
                        }
                    }
                    i += 6;
                }
                _ => {
                    raw.push(n as char);
                    i += 2;
                }
            }
        } else if c == b'"' {
            return Ok((raw, i + 1));
        } else {
            let ch = next_utf8_char(bytes, i)?;
            raw.push(ch);
            i += ch.len_utf8();
        }
    }
    Err("unterminated string".into())
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    let mut t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        t.push_str("...");
    }
    t
}

fn active_https_proxy() -> Option<String> {
    for key in ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY", "http_proxy", "HTTP_PROXY"] {
        if let Ok(v) = env::var(key) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn forward_via_curl(method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    let url = format!("https://{}{}", TARGET_HOST, path);
    let tmp_dir = shield_dir().join("tmp");
    let _ = fs::create_dir_all(&tmp_dir);
    let body_path = tmp_dir.join(format!("req-{}.bin", std::process::id()));
    let hdr_path = tmp_dir.join(format!("hdr-{}.bin", std::process::id()));
    let out_path = tmp_dir.join(format!("out-{}.bin", std::process::id()));
    fs::write(&body_path, body).map_err(|e| e.to_string())?;

    let mut cmd = Command::new("curl");
    cmd.arg("-sS")
        .arg("-X")
        .arg(method)
        .arg(&url)
        .arg("-D")
        .arg(&hdr_path)
        .arg("-o")
        .arg(&out_path)
        .arg("--max-time")
        .arg("120");

    if let Some(proxy) = active_https_proxy() {
        cmd.arg("-x").arg(proxy);
    }

    for (k, v) in headers {
        let lk = k.to_ascii_lowercase();
        if lk == "host" || lk == "content-length" || lk == "connection" || lk == "transfer-encoding" {
            continue;
        }
        cmd.arg("-H").arg(format!("{}: {}", k, v));
    }
    if !body.is_empty() {
        cmd.arg("--data-binary").arg(format!("@{}", body_path.display()));
    }

    let status = cmd.status().map_err(|e| e.to_string())?;
    let hdr_raw = fs::read_to_string(&hdr_path).unwrap_or_default();
    let out = fs::read(&out_path).unwrap_or_default();
    let _ = fs::remove_file(&body_path);
    let _ = fs::remove_file(&hdr_path);
    let _ = fs::remove_file(&out_path);

    if !status.success() && out.is_empty() {
        return Err("upstream curl failed".into());
    }

    let mut code: u16 = 502;
    let mut resp_headers = Vec::new();
    for (idx, line) in hdr_raw.lines().enumerate() {
        if idx == 0 {
            // HTTP/1.1 200 OK
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                code = parts[1].parse().unwrap_or(502);
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let lk = key.to_ascii_lowercase();
            if lk == "transfer-encoding" || lk == "content-length" || lk == "connection" {
                continue;
            }
            resp_headers.push((key, v.trim().to_string()));
        }
    }
    Ok((code, resp_headers, out))
}

fn read_http_request(stream: &mut TcpStream) -> Result<(String, String, Vec<(String, String)>, Vec<u8>), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_header_end(&buf) {
            let header_bytes = &buf[..pos];
            let header_str = String::from_utf8_lossy(header_bytes);
            let mut lines = header_str.lines();
            let request_line = lines.next().unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut content_length = 0usize;
            for line in lines {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim().to_string();
                    let val = v.trim().to_string();
                    if key.eq_ignore_ascii_case("content-length") {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.push((key, val));
                }
            }
            let mut body = buf[pos..].to_vec();
            while body.len() < content_length {
                let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
            if body.len() > content_length {
                body.truncate(content_length);
            }
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("GET").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            return Ok((method, path, headers, body));
        }
        if buf.len() > 1024 * 1024 {
            return Err("headers too large".into());
        }
    }
    Err("incomplete request".into())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn handle_client(mut stream: TcpStream) {
    let result = (|| -> Result<(), String> {
        let (method, path, headers, body) = read_http_request(&mut stream)?;
        let mut body = body;
        let ct = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");

        if method.eq_ignore_ascii_case("POST") && ct.contains("application/json") {
            let text = String::from_utf8_lossy(&body);
            match translate_json_payload(&text) {
                Ok(translated) => body = translated.into_bytes(),
                Err(e) if e.contains("translate") || e.contains("parse") || e.contains("curl") => {
                    let msg = "Service Unavailable: Translation gateway failed. Request blocked to prevent Chinese text leak.";
                    let resp = format!(
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        msg.len(),
                        msg
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    return Ok(());
                }
                Err(_) => {}
            }
        }

        match forward_via_curl(&method, &path, &headers, &body) {
            Ok((code, rh, out)) => {
                let mut resp = format!("HTTP/1.1 {} OK\r\n", code);
                // Fix status text roughly
                resp = format!("HTTP/1.1 {}\r\n", code);
                for (k, v) in rh {
                    resp.push_str(&format!("{}: {}\r\n", k, v));
                }
                resp.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", out.len()));
                stream.write_all(resp.as_bytes()).map_err(|e| e.to_string())?;
                stream.write_all(&out).map_err(|e| e.to_string())?;
            }
            Err(e) => {
                let msg = format!("Bad Gateway: {}", e);
                let resp = format!(
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    msg.len(),
                    msg
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        append_log(&format!("client error: {}", e));
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// Run proxy in the current process (blocking).
pub fn run_foreground() -> Result<(), String> {
    let _ = fs::create_dir_all(shield_dir());
    let listener = TcpListener::bind(("127.0.0.1", PORT)).map_err(|e| {
        format!("Failed to bind 127.0.0.1:{} — {}", PORT, e)
    })?;
    PROXY_RUNNING.store(true, Ordering::SeqCst);
    let pid = std::process::id();
    let _ = fs::write(proxy_pid_path(), pid.to_string());
    append_log(&format!("Pure-Rust translation proxy listening on 127.0.0.1:{} pid={}", PORT, pid));
    for conn in listener.incoming() {
        if !PROXY_RUNNING.load(Ordering::SeqCst) {
            break;
        }
        match conn {
            Ok(stream) => {
                thread::spawn(move || handle_client(stream));
            }
            Err(e) => append_log(&format!("accept error: {}", e)),
        }
    }
    Ok(())
}

/// Spawn a detached child running `claude-shield proxy`.
pub fn start_background(self_exe: &std::path::Path) -> Result<u32, String> {
    let _ = fs::create_dir_all(shield_dir());
    if is_running() {
        return Err("Translation proxy already running".into());
    }
    let log = File::create(proxy_log_path()).map_err(|e| e.to_string())?;
    let err = File::create(proxy_log_path()).map_err(|e| e.to_string())?;
    let child = Command::new(self_exe)
        .arg("proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();
    let _ = fs::write(proxy_pid_path(), pid.to_string());
    // Give it a moment to bind
    thread::sleep(Duration::from_millis(200));
    Ok(pid)
}

pub fn stop() -> bool {
    PROXY_RUNNING.store(false, Ordering::SeqCst);
    let path = proxy_pid_path();
    if !path.exists() {
        return false;
    }
    let Ok(pid_str) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(pid) = pid_str.trim().parse::<u32>() else {
        let _ = fs::remove_file(&path);
        return false;
    };
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = fs::remove_file(&path);
    true
}

pub fn is_running() -> bool {
    let path = proxy_pid_path();
    if !path.exists() {
        return false;
    }
    let Ok(pid_str) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(pid) = pid_str.trim().parse::<u32>() else {
        return false;
    };
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {}", pid))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

pub fn local_base_url() -> String {
    format!("http://127.0.0.1:{}", PORT)
}
