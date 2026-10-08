use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub async fn get(port: u16, target: &str) -> Reply {
    request(port, "GET", target, &[]).await
}

pub async fn request(port: u16, method: &str, target: &str, headers: &[(&str, &str)]) -> Reply {
    let mut stream = open(port, method, target, headers).await;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap_or_default();
    parse(&raw)
}

pub async fn open(port: u16, method: &str, target: &str, headers: &[(&str, &str)]) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.expect("connect");
    let mut text = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes()).await.expect("write request");
    stream
}

pub async fn read_at_least(stream: &mut TcpStream, bytes: usize) -> usize {
    let mut total = 0;
    let mut buffer = vec![0u8; 16 * 1024];
    while total < bytes {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => total += read,
        }
    }
    total
}

fn parse(raw: &[u8]) -> Reply {
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n").expect("response head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|line| line.split(' ').nth(1)).and_then(|code| code.parse().ok()).expect("status");
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let body = &raw[split + 4..];
    let chunked = headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked"));
    Reply { status, headers, body: if chunked { dechunk(body) } else { body.to_vec() } }
}

fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = body.windows(2).position(|window| window == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&body[..line_end]).trim(), 16).unwrap_or(0);
        if size == 0 || body.len() < line_end + 2 + size {
            break;
        }
        out.extend_from_slice(&body[line_end + 2..line_end + 2 + size]);
        body = &body[(line_end + 2 + size + 2).min(body.len())..];
    }
    out
}
