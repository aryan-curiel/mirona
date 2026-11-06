//! MCP over streamable HTTP on 127.0.0.1 (NGR-168), for clients that can't spawn a process.
//! Every request carries `Authorization: Bearer <token>`; the token (one per client, in
//! `config.toml`) names the client for the activity log. Config is reread per request, so a
//! revoked token or turning MCP off takes effect at once. `mcp_http` only says whether the app
//! runs this server; `mirona mcp --http` serves regardless.

use crate::{Server, load};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The path clients connect to: `http://127.0.0.1:<port>/mcp`.
pub const PATH: &str = "/mcp";

/// The client name a request authenticated as, for the activity log.
#[derive(Clone)]
pub(crate) struct ClientName(pub String);

/// A running HTTP server. Dropping it shuts it down.
pub struct HttpServer {
    pub port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpServer {
    /// Blocks until the server stops (the CLI: until Ctrl-C).
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            s.send(()).ok();
        }
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

/// A new client token: 32 random bytes, hex.
pub fn new_token() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}

/// Binds 127.0.0.1:`port` (0: any free port, see [`HttpServer::port`]) and serves on a
/// background thread. Fails right away when the port is taken.
pub fn serve_http(app_dir: PathBuf, port: u16) -> Result<HttpServer, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("can't listen on 127.0.0.1:{port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return eprintln!("mirona-mcp: http: {e}"),
        };
        rt.block_on(run(app_dir, listener, rx));
    });
    Ok(HttpServer {
        port,
        stop: Some(tx),
        thread: Some(thread),
    })
}

async fn run(
    app_dir: PathBuf,
    listener: std::net::TcpListener,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => return eprintln!("mirona-mcp: http: {e}"),
    };
    let dir = app_dir.clone();
    let mcp = StreamableHttpService::new(
        move || Ok(Server::new(dir.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    loop {
        let stream = tokio::select! {
            _ = &mut stop => break,
            r = listener.accept() => match r {
                Ok((s, _)) => s,
                Err(e) => { eprintln!("mirona-mcp: http: {e}"); continue }
            },
        };
        let (mcp, app_dir) = (mcp.clone(), app_dir.clone());
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |mut req: Request<Incoming>| {
                let (mcp, app_dir) = (mcp.clone(), app_dir.clone());
                async move {
                    let resp = match authorize(&app_dir, &req) {
                        Ok(client) => {
                            req.extensions_mut().insert(ClientName(client));
                            mcp.handle(req).await
                        }
                        Err((status, msg)) => plain(status, msg),
                    };
                    Ok::<_, std::convert::Infallible>(resp)
                }
            });
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await
            {
                eprintln!("mirona-mcp: http: {e}");
            }
        });
    }
    // Dropping the runtime ends the open connections and sessions.
}

/// The client name for a request's bearer token, or why it's refused.
fn authorize<B>(app_dir: &Path, req: &Request<B>) -> Result<String, (StatusCode, String)> {
    if req.uri().path() != PATH {
        return Err((StatusCode::NOT_FOUND, format!("MCP lives at {PATH}")));
    }
    let cfg = load(app_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .0;
    if !cfg.mcp_enabled {
        return Err((StatusCode::SERVICE_UNAVAILABLE, crate::DISABLED.into()));
    }
    let token = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .trim();
    cfg.mcp_tokens
        .iter()
        .find(|(_, t)| !t.is_empty() && same(t.as_bytes(), token.as_bytes()))
        .map(|(name, _)| name.clone())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Missing or unknown token. Add a client in Mirona's settings and send \
                 `Authorization: Bearer <token>`."
                    .into(),
            )
        })
}

/// Constant-time equality, so response timing doesn't leak a token byte by byte.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn plain(
    status: StatusCode,
    msg: String,
) -> Response<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>> {
    let mut r = Response::new(Full::new(Bytes::from(msg)).boxed());
    *r.status_mut() = status;
    if status == StatusCode::UNAUTHORIZED {
        r.headers_mut().insert(
            http::header::WWW_AUTHENTICATE,
            http::HeaderValue::from_static("Bearer"),
        );
    }
    r
}
