//! RPC transport owns the socket. Request handlers never block its reader.
use crate::protocol::Wire;
use anyhow::{bail, ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Message {
    Request {
        id: String,
        method: String,
        payload: Value,
    },
    Response {
        id: String,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Option<String>,
    },
    Event {
        name: String,
        payload: Value,
    },
}
pub enum Incoming {
    Request {
        id: String,
        method: String,
        payload: Value,
    },
    Event {
        name: String,
        payload: Value,
    },
}
enum Outgoing {
    Request {
        method: String,
        payload: Value,
        reply: oneshot::Sender<Result<Value>>,
        timeout: Duration,
    },
    Frame(Value),
}
#[derive(Clone)]
pub struct Session {
    sender: mpsc::Sender<Outgoing>,
    pub closed: CancellationToken,
}
struct Pending {
    deadline: tokio::time::Instant,
    reply: oneshot::Sender<Result<Value>>,
}
impl Session {
    pub fn spawn(mut wire: Wire) -> (Self, mpsc::Receiver<Incoming>) {
        let (sender, mut outgoing) = mpsc::channel(64);
        let (incoming, receiver) = mpsc::channel(32);
        let closed = CancellationToken::new();
        let done = closed.clone();
        tokio::spawn(async move {
            let mut sequence = 0u64;
            let mut pending = HashMap::<String, Pending>::new();
            let mut expiry = tokio::time::interval(Duration::from_millis(100));
            let result: Result<()> = async {
                loop {
                    tokio::select! {
                        _ = done.cancelled() => break,
                        _ = expiry.tick() => {
                            let now = tokio::time::Instant::now();
                            pending.retain(|_, item| !item.reply.is_closed());
                            let expired: Vec<_> = pending.iter().filter(|(_,p)|p.deadline <= now).map(|(id,_)|id.clone()).collect();
                            for id in expired { if let Some(p) = pending.remove(&id) { let _ = p.reply.send(Err(anyhow::anyhow!("对端响应超时"))); } }
                        }
                        next = outgoing.recv() => {
                            let Some(next) = next else { break };
                            let frame = match next {
                                Outgoing::Frame(v) => v,
                                Outgoing::Request { method, payload, reply, timeout } => {
                                    if pending.len() >= 64 { let _ = reply.send(Err(anyhow::anyhow!("控制请求过多"))); continue; }
                                    sequence = sequence.checked_add(1).context("请求序号耗尽")?;
                                    let id = sequence.to_string();
                                    pending.insert(id.clone(), Pending { deadline: tokio::time::Instant::now() + timeout, reply });
                                    json!({"kind":"request","id":id,"method":method,"payload":payload})
                                }
                            };
                            let sealed = wire.codec.seal(&frame)?;
                            tokio::time::timeout(Duration::from_secs(5), wire.socket.send(sealed)).await.context("控制发送超时")??;
                        }
                        next = wire.socket.next() => {
                            let text = next.context("对端已断开")??;
                            let value = wire.codec.open(&text)?;
                            match serde_json::from_value::<Message>(value)? {
                                Message::Response { id, result, error } => {
                                    ensure!(id.len() <= 32 && error.as_ref().is_none_or(|s|s.len() <= 1600), "控制响应无效");
                                    if let Some(p) = pending.remove(&id) { let _ = p.reply.send(match error { Some(e) => Err(anyhow::anyhow!(e)), None => Ok(result) }); }
                                }
                                Message::Request { id, method, payload } => {
                                    ensure!(!id.is_empty() && id.len() <= 32 && method.len() <= 40, "控制请求无效");
                                    incoming.try_send(Incoming::Request { id, method, payload }).context("控制请求处理队列已满")?;
                                }
                                Message::Event { name, payload } => {
                                    ensure!(name.len() <= 40, "控制事件无效");
                                    incoming.try_send(Incoming::Event { name, payload }).context("控制事件处理队列已满")?;
                                }
                            }
                        }
                    }
                }
                Ok(())
            }.await;
            done.cancel();
            let reason = result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "共享会话已结束".into());
            for (_, p) in pending {
                let _ = p.reply.send(Err(anyhow::anyhow!(reason.clone())));
            }
        });
        (Self { sender, closed }, receiver)
    }
    pub async fn request(&self, method: &str, payload: Value, timeout: Duration) -> Result<Value> {
        ensure!(!self.closed.is_cancelled(), "共享会话已结束");
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Outgoing::Request {
                method: method.into(),
                payload,
                reply,
                timeout,
            })
            .await
            .context("控制通道已关闭")?;
        tokio::select! {
            _ = self.closed.cancelled() => bail!("共享会话已结束"),
            result = tokio::time::timeout(timeout + Duration::from_secs(1), receive) => result.context("控制请求超时")?.context("控制通道已关闭")?,
        }
    }
    pub async fn reply(&self, id: String, result: Result<Value>) -> Result<()> {
        let frame = match result {
            Ok(result) => json!({"kind":"response","id":id,"result":result}),
            Err(e) => {
                json!({"kind":"response","id":id,"error":e.to_string().chars().take(400).collect::<String>()})
            }
        };
        self.sender
            .send(Outgoing::Frame(frame))
            .await
            .context("控制通道已关闭")
    }
    pub async fn notify(&self, name: &str, payload: Value) -> Result<()> {
        self.sender
            .send(Outgoing::Frame(
                json!({"kind":"event","name":name,"payload":payload}),
            ))
            .await
            .context("控制通道已关闭")
    }
    pub fn close(&self) {
        self.closed.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Wire;
    async fn pair() -> (
        Session,
        mpsc::Receiver<Incoming>,
        Session,
        mpsc::Receiver<Incoming>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            Wire::handshake(listener.accept().await.unwrap().0, [4; 32], true)
                .await
                .unwrap()
        });
        let client = Wire::handshake(
            tokio::net::TcpStream::connect(address).await.unwrap(),
            [4; 32],
            false,
        )
        .await
        .unwrap();
        let (a, ar) = Session::spawn(server.await.unwrap());
        let (b, br) = Session::spawn(client);
        (a, ar, b, br)
    }
    #[tokio::test]
    async fn reciprocal_requests_do_not_block_reader() {
        let (a, mut ar, b, mut br) = pair().await;
        let ac = a.clone();
        let bc = b.clone();
        let ha = tokio::spawn(async move {
            if let Some(Incoming::Request {
                id,
                method,
                payload,
            }) = ar.recv().await
            {
                assert_eq!(method, "identify");
                ac.reply(id, Ok(payload)).await.unwrap();
            }
        });
        let hb = tokio::spawn(async move {
            if let Some(Incoming::Request {
                id,
                method,
                payload,
            }) = br.recv().await
            {
                assert_eq!(method, "identify");
                bc.reply(id, Ok(payload)).await.unwrap();
            }
        });
        let (x, y) = tokio::join!(
            a.request("identify", json!("A"), Duration::from_secs(2)),
            b.request("identify", json!("B"), Duration::from_secs(2))
        );
        assert_eq!(x.unwrap(), "A");
        assert_eq!(y.unwrap(), "B");
        ha.await.unwrap();
        hb.await.unwrap();
        a.close();
        b.close();
    }
    #[tokio::test]
    async fn deadlines_and_close_release_pending_requests() {
        let (a, _ar, b, _br) = pair().await;
        assert!(a
            .request("ignored", Value::Null, Duration::from_millis(50))
            .await
            .unwrap_err()
            .to_string()
            .contains("超时"));
        let ac = a.clone();
        let pending = tokio::spawn(async move {
            ac.request("ignored", Value::Null, Duration::from_secs(10))
                .await
        });
        tokio::task::yield_now().await;
        a.close();
        assert!(tokio::time::timeout(Duration::from_millis(500), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        b.close();
    }
}
