use super::*;
use crate::fetch::{FetchError, Fetcher, Request, Response};
use crate::proxy::memory_store::MemoryStore;
use crate::proxy::test_client::request;
use crate::proxy::Config;
use futures_util::stream;

const URL: &str = "https://o.test/v.mp4";
const OFFSET: u64 = 50;
const WANTED: u64 = 10;

struct Claim {
    span: Option<&'static str>,
    body: Vec<u8>,
}

#[async_trait::async_trait]
impl Fetcher for Claim {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        let mut headers = HashMap::from([("Content-Length".to_string(), self.body.len().to_string())]);
        let status = if request.range.is_some() { 206 } else { 200 };
        if let (Some(span), 206) = (self.span, status) {
            headers.insert("Content-Range".to_string(), span.to_string());
        }
        let body = Box::pin(stream::iter(vec![Ok(Bytes::from(self.body.clone()))]));
        Ok(Response { status, headers, body })
    }
}

async fn proxy_claiming(span: Option<&'static str>, body: usize, store: MemoryStore) -> crate::proxy::Proxy {
    let fetcher = Arc::new(Claim { span, body: vec![7; body] });
    crate::proxy::Proxy::start(Config::default(), fetcher, Arc::new(store)).await.unwrap()
}

async fn resumed(span: Option<&'static str>, body: usize) -> (u64, Option<String>, bool) {
    let proxy = proxy_claiming(span, body, MemoryStore::new()).await;
    let mut resume = Resume::new(&proxy.inner, URL, &HashMap::new(), OFFSET);
    let (mut position, mut error) = (0u64, None);
    while let Some(item) = resume.next(&mut position, Some(WANTED)).await {
        if let Err(failure) = item {
            error = Some(failure.to_string());
            break;
        }
    }
    (position, error, resume.next(&mut position, Some(WANTED)).await.is_none())
}

#[tokio::test]
async fn inv_proxy_35_a_resume_rejects_an_origin_that_answers_another_offset() {
    let (position, error, _) = resumed(Some("bytes 0-9/100"), 10).await;

    assert_eq!(position, 0, "bytes of another range reached the player");
    assert!(error.is_some());
}

#[tokio::test]
async fn inv_proxy_35_a_resume_never_delivers_more_than_the_remainder() {
    let (position, error, finished) = resumed(Some("bytes 50-59/100"), 30).await;

    assert_eq!((position, error, finished), (WANTED, None, true));
}

#[tokio::test]
async fn inv_proxy_35_a_resume_reports_a_body_that_ends_before_the_remainder() {
    let (position, error, _) = resumed(Some("bytes 50-59/100"), 4).await;

    assert_eq!(position, 4);
    assert!(error.is_some(), "a short body ended the response as if it were complete");
}

#[tokio::test]
async fn inv_proxy_35_a_resume_accepts_a_range_answer_without_content_range() {
    let (position, error, finished) = resumed(None, 10).await;

    assert_eq!((position, error, finished), (WANTED, None, true));
}

#[tokio::test]
async fn inv_proxy_35_a_direct_response_rejects_an_origin_that_answers_another_offset() {
    let store = MemoryStore::new();
    store.fail_at(Some(crate::proxy::memory_store::Failure::Begin));
    let proxy = proxy_claiming(Some("bytes 5-14/100"), 10, store).await;
    let routed = url::Url::parse(&proxy.url_for(URL, &HashMap::new()).unwrap()).unwrap();
    let target = format!("{}?{}", routed.path(), routed.query().unwrap());

    let reply = request(proxy.port(), "GET", &target, &[("Range", "bytes=0-9")]).await;

    assert_eq!(reply.status, 502);
}
