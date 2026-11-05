use futures_util::StreamExt;
use http_body_util::BodyExt;
use super::flight::Rejection;
use bytes::Bytes;
use futures_util::Stream;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{Empty, Full, StreamBody};
use hyper::body::Frame;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Response, StatusCode};
use std::collections::HashMap;
use std::io;

pub(super) type ProxyBody = UnsyncBoxBody<Bytes, io::Error>;
pub(super) type Headers = Vec<(&'static str, String)>;
pub(super) type Upstream = HashMap<String, String>;
pub(super) type Failure = Box<Response<ProxyBody>>;

pub(super) fn media_headers(content_type: &str, length: u64, content_range: Option<String>) -> Headers {
    let mut headers: Headers = vec![
        ("content-type", content_type.to_string()),
        ("accept-ranges", "bytes".to_string()),
        ("content-length", length.to_string()),
    ];
    if let Some(value) = content_range {
        headers.push(("content-range", value));
    }
    headers
}

pub(super) fn unsatisfiable(total: u64) -> Response<ProxyBody> {
    respond(416, vec![("content-range", format!("bytes */{total}")), ("content-length", "0".to_string())], empty(), false)
}

pub(super) fn rejected(rejection: &Rejection, head_only: bool) -> Response<ProxyBody> {
    let mut headers: Headers = vec![("content-length", rejection.body.len().to_string())];
    if let Some(content_type) = &rejection.content_type {
        headers.push(("content-type", content_type.clone()));
    }
    respond(rejection.status, headers, full(rejection.body.clone()), head_only)
}

pub(super) fn text(status: u16, message: &str) -> Response<ProxyBody> {
    let headers = vec![("content-type", "text/plain; charset=utf-8".to_string()), ("content-length", message.len().to_string())];
    respond(status, headers, full(Bytes::from(message.to_string())), false)
}

pub(super) fn respond(status: u16, headers: Headers, body: ProxyBody, head_only: bool) -> Response<ProxyBody> {
    let mut response = Response::new(if head_only { empty() } else { body });
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in headers {
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(HeaderName::from_static(name), value);
        }
    }
    response
}

pub(super) fn full(bytes: Bytes) -> ProxyBody {
    Full::new(bytes).map_err(|never| match never {}).boxed_unsync()
}

pub(super) fn empty() -> ProxyBody {
    Empty::<Bytes>::new().map_err(|never| match never {}).boxed_unsync()
}

pub(super) fn streamed(source: impl Stream<Item = Result<Bytes, io::Error>> + Send + 'static) -> ProxyBody {
    StreamBody::new(source.map(|chunk| chunk.map(Frame::data))).boxed_unsync()
}
