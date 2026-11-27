use super::inner::{ProxyShared, Listener, LiveGuard};
use super::serve::handle;
use super::ProxyError;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinSet;

const ACCEPT_RETRY: Duration = Duration::from_millis(10);
const ACCEPT_RETRY_MAX: Duration = Duration::from_secs(1);
const ACCEPT_ERROR_LIMIT: usize = 8;

struct AliveGuard(Arc<AtomicU64>, u64);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        let _ = self.0.compare_exchange(self.1, 0, Ordering::SeqCst, Ordering::SeqCst);
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
    let epoch = inner.listener_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    inner.alive.store(epoch, Ordering::SeqCst);
    let (done, finished) = oneshot::channel();
    let owner = inner.clone();
    inner.tasks.spawn(async move {
        accept_loop(owner, socket, signal, epoch).await;
        drop(done);
    });
    Ok((Listener { shutdown, finished }, port))
}

async fn bind_any() -> Result<TcpListener, ProxyError> {
    TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| ProxyError::Bind(error.to_string()))
}

async fn accept_loop(inner: Arc<ProxyShared>, socket: TcpListener, mut shutdown: watch::Receiver<bool>, epoch: u64) {
    let _alive = AliveGuard(inner.alive.clone(), epoch);
    let address = socket.local_addr().ok();
    let mut connections = JoinSet::new();
    let mut retry = ACCEPT_RETRY;
    let mut failing = false;
    let mut errors = 0;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = accept(&socket, &inner) => match accepted {
                Ok((stream, _)) => {
                    retry = ACCEPT_RETRY;
                    failing = false;
                    errors = 0;
                    let guard = LiveGuard::new(&inner.live);
                    connections.spawn(serve_connection(inner.clone(), stream, guard));
                }
                Err(error) => {
                    errors += 1;
                    if !failing {
                        inner.config.failures.report(crate::failures::MediaFailure {
                            kind: crate::api::Kind::Video,
                            url: address.map_or_else(String::new, |address| format!("http://{address}")),
                            stage: crate::failures::FailureStage::Store,
                            reason: format!("proxy listener refused connections: {error}"),
                        });
                        failing = true;
                    }
                    if errors >= ACCEPT_ERROR_LIMIT {
                        #[cfg(test)]
                        inner.accept_failures.send_replace(false);
                        super::rebind_after_accept_errors(inner.clone(), epoch);
                        let _ = shutdown.changed().await;
                        break;
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

async fn accept(socket: &TcpListener, inner: &ProxyShared) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    #[cfg(test)]
    {
        let mut signal = inner.accept_failures.subscribe();
        if *signal.borrow() {
            return Err(std::io::Error::other("injected accept error"));
        }
        return tokio::select! {
            result = socket.accept() => result,
            _ = signal.changed() => Err(std::io::Error::other("injected accept error")),
        };
    }
    #[cfg(not(test))]
    {
        let _ = inner;
        socket.accept().await
    }
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
