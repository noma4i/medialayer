use super::inner::{ProxyShared, Listener, LiveGuard};
use super::serve::handle;
use super::ProxyError;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinSet;

const ACCEPT_RETRY: Duration = Duration::from_millis(10);
const ACCEPT_RETRY_MAX: Duration = Duration::from_secs(1);

struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
        #[cfg(test)]
        crate::test_signal::fire();
    }
}

pub(super) async fn spawn_listener(inner: &Arc<ProxyShared>, preferred_port: u16) -> Result<(Listener, u16), ProxyError> {
    let socket = match TcpListener::bind(("127.0.0.1", preferred_port)).await {
        Ok(socket) => socket,
        Err(_) if preferred_port != 0 => bind_any().await?,
        Err(error) => return Err(ProxyError::Bind(error.to_string())),
    };
    let port = socket.local_addr().map_err(|error| ProxyError::Bind(error.to_string()))?.port();
    let (shutdown, signal) = watch::channel(false);
    inner.alive.store(true, Ordering::SeqCst);
    let (done, finished) = oneshot::channel();
    let owner = inner.clone();
    inner.tasks.spawn(async move {
        accept_loop(owner, socket, signal).await;
        drop(done);
    });
    Ok((Listener { shutdown, finished }, port))
}

async fn bind_any() -> Result<TcpListener, ProxyError> {
    TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| ProxyError::Bind(error.to_string()))
}

async fn accept_loop(inner: Arc<ProxyShared>, socket: TcpListener, mut shutdown: watch::Receiver<bool>) {
    let _alive = AliveGuard(inner.alive.clone());
    let address = socket.local_addr().ok();
    let mut connections = JoinSet::new();
    let mut retry = ACCEPT_RETRY;
    let mut failing = false;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = socket.accept() => match accepted {
                Ok((stream, _)) => {
                    retry = ACCEPT_RETRY;
                    failing = false;
                    let guard = LiveGuard::new(&inner.live);
                    connections.spawn(serve_connection(inner.clone(), stream, guard));
                }
                Err(error) => {
                    if !failing {
                        inner.config.failures.report(crate::failures::MediaFailure {
                            kind: crate::api::Kind::Video,
                            url: address.map_or_else(String::new, |address| format!("http://{address}")),
                            stage: crate::failures::FailureStage::Store,
                            reason: format!("proxy listener refused connections: {error}"),
                        });
                        failing = true;
                    }
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        _ = tokio::time::sleep(retry) => {}
                    }
                    retry = (retry * 2).min(ACCEPT_RETRY_MAX);
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(socket);
    connections.shutdown().await;
}

async fn serve_connection(inner: Arc<ProxyShared>, stream: TcpStream, _live: LiveGuard) {
    let _ = stream.set_nodelay(true);
    let header_timeout = inner.config.header_read_timeout;
    let on_request = service_fn(move |request| {
        let inner = inner.clone();
        async move { Ok::<_, Infallible>(handle(inner, request).await) }
    });
    let connection = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout)
        .serve_connection(TokioIo::new(stream), on_request);
    let _ = connection.await;
}
