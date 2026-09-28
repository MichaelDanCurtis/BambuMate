//! Line-delimited JSON-RPC as spoken by `codex app-server` over stdio.
//! Messages carry no "jsonrpc" field.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RpcError {
    #[error("{message} (code {code})")]
    Remote { code: i64, message: String },
    #[error("the agent process closed the connection")]
    Closed,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

pub struct RpcConnection {
    writer: AsyncMutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Pending,
    next_id: AtomicU64,
}

impl RpcConnection {
    pub fn start<R, W>(reader: R, writer: W) -> (Arc<Self>, mpsc::UnboundedReceiver<Incoming>)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::unbounded_channel();
        let conn = Arc::new(Self {
            writer: AsyncMutex::new(Box::new(writer)),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
        });
        tokio::spawn(read_loop(BufReader::new(reader), pending, tx));
        (conn, rx)
    }

    async fn write(&self, msg: &Value) -> Result<(), RpcError> {
        let mut line = msg.to_string();
        line.push('\n');
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|_| RpcError::Closed)?;
        w.flush().await.map_err(|_| RpcError::Closed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self
            .write(&json!({"id": id, "method": method, "params": params}))
            .await
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        rx.await.unwrap_or(Err(RpcError::Closed))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        let msg = match params {
            Some(p) => json!({"method": method, "params": p}),
            None => json!({"method": method}),
        };
        self.write(&msg).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "result": result})).await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "error": {"code": code, "message": message}}))
            .await
    }
}

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: BufReader<R>,
    pending: Pending,
    tx: mpsc::UnboundedSender<Incoming>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            tracing::debug!("codex: ignoring non-JSON line: {}", line.trim());
            continue;
        };
        let method = msg
            .get("method")
            .and_then(|m| m.as_str())
            .map(str::to_string);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, msg.get("id").cloned()) {
            (Some(method), Some(id)) => {
                let _ = tx.send(Incoming::Request { id, method, params });
            }
            (Some(method), None) => {
                let _ = tx.send(Incoming::Notification { method, params });
            }
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { continue };
                let Some(waiter) = pending.lock().unwrap().remove(&n) else {
                    continue;
                };
                let result = match msg.get("error") {
                    Some(err) => Err(RpcError::Remote {
                        code: err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                        message: err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error")
                            .to_string(),
                    }),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
            (None, None) => {}
        }
    }
    for (_, waiter) in pending.lock().unwrap().drain() {
        let _ = waiter.send(Err(RpcError::Closed));
    }
    // `tx` drops here, which ends the Incoming stream: the crash signal.
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    /// Bounds any await on a network/stream event so a regression in the
    /// implementation fails the test instead of hanging it forever.
    async fn with_timeout<F: std::future::Future>(fut: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(5), fut)
            .await
            .expect("test timed out waiting for an event that should have occurred")
    }

    struct Server {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Server {
        async fn read(&mut self) -> Value {
            let mut l = String::new();
            with_timeout(self.r.read_line(&mut l)).await.unwrap();
            serde_json::from_str(&l).unwrap()
        }
        async fn send(&mut self, v: Value) {
            self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        }
    }

    fn pair() -> (
        Arc<RpcConnection>,
        mpsc::UnboundedReceiver<Incoming>,
        Server,
    ) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        let (conn, rx) = RpcConnection::start(cr, cw);
        (
            conn,
            rx,
            Server {
                r: BufReader::new(sr),
                w: sw,
            },
        )
    }

    #[tokio::test]
    async fn request_resolves_with_matching_response_and_omits_jsonrpc() {
        let (conn, _rx, mut srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("account/read", json!({})).await });
        let msg = srv.read().await;
        assert_eq!(msg["method"], "account/read");
        assert!(msg.get("jsonrpc").is_none());
        srv.send(json!({"id": msg["id"], "result": {"account": null}}))
            .await;
        assert_eq!(
            with_timeout(call).await.unwrap().unwrap(),
            json!({"account": null})
        );
    }

    #[tokio::test]
    async fn remote_error_is_mapped() {
        let (conn, _rx, mut srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("thread/start", json!({})).await });
        let msg = srv.read().await;
        srv.send(json!({"id": msg["id"], "error": {"code": -32600, "message": "bad"}}))
            .await;
        assert_eq!(
            with_timeout(call).await.unwrap(),
            Err(RpcError::Remote {
                code: -32600,
                message: "bad".into()
            })
        );
    }

    #[tokio::test]
    async fn delivers_notifications_and_server_requests() {
        let (_conn, mut rx, mut srv) = pair();
        srv.send(json!({"method":"turn/started","params":{"threadId":"t"}}))
            .await;
        srv.send(json!({"id": 7, "method":"item/tool/call","params":{"tool":"bm_app_state"}}))
            .await;
        assert_eq!(
            with_timeout(rx.recv()).await.unwrap(),
            Incoming::Notification {
                method: "turn/started".into(),
                params: json!({"threadId":"t"})
            }
        );
        assert_eq!(
            with_timeout(rx.recv()).await.unwrap(),
            Incoming::Request {
                id: json!(7),
                method: "item/tool/call".into(),
                params: json!({"tool":"bm_app_state"})
            }
        );
    }

    #[tokio::test]
    async fn respond_and_notify_write_expected_lines() {
        let (conn, _rx, mut srv) = pair();
        conn.respond(json!(7), json!({"success": true}))
            .await
            .unwrap();
        assert_eq!(
            srv.read().await,
            json!({"id": 7, "result": {"success": true}})
        );
        conn.notify("initialized", None).await.unwrap();
        assert_eq!(srv.read().await, json!({"method": "initialized"}));
    }

    #[tokio::test]
    async fn closing_fails_pending_requests_and_ends_stream() {
        let (conn, mut rx, srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("model/list", json!({})).await });
        tokio::task::yield_now().await;
        drop(srv);
        assert_eq!(with_timeout(call).await.unwrap(), Err(RpcError::Closed));
        assert!(with_timeout(rx.recv()).await.is_none());
    }
}
