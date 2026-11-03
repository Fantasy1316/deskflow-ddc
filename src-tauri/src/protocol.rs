use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, ensure, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LinesCodec};

pub const MAX_FRAME: usize = 128 * 1024;
pub fn pairing_key(code: &str) -> Result<[u8; 32]> {
    let compact = code.trim().strip_prefix("SB1-").unwrap_or(code.trim());
    let key = URL_SAFE_NO_PAD.decode(compact).context("配对码格式无效")?;
    ensure!(URL_SAFE_NO_PAD.encode(&key) == compact, "配对码格式无效");
    key.try_into()
        .map_err(|_| anyhow::anyhow!("配对码长度无效"))
}
pub fn new_pairing() -> String {
    let mut key = [0; 32];
    rand::thread_rng().fill_bytes(&mut key);
    format!("SB1-{}", URL_SAFE_NO_PAD.encode(key))
}
pub fn nonce() -> String {
    let mut bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    seq: String,
    data: String,
    tag: String,
}
pub struct Codec {
    tx: [u8; 36],
    rx: [u8; 36],
    sent: u64,
    received: u64,
}
impl Codec {
    pub fn new(key: &[u8; 32], client: &str, server: &str, host: bool) -> Result<Self> {
        ensure!(client.len() == 64 && server.len() == 64, "握手随机数无效");
        let salt = hex::decode(format!("{client}{server}"))?;
        let hk = Hkdf::<Sha256>::new(Some(&salt), key);
        let mut s2c = [0; 36];
        let mut c2s = [0; 36];
        hk.expand(b"screenbridge-v1/server-to-client", &mut s2c)
            .map_err(|_| anyhow::anyhow!("密钥派生失败"))?;
        hk.expand(b"screenbridge-v1/client-to-server", &mut c2s)
            .map_err(|_| anyhow::anyhow!("密钥派生失败"))?;
        let (tx, rx) = if host { (s2c, c2s) } else { (c2s, s2c) };
        Ok(Self {
            tx,
            rx,
            sent: 0,
            received: 0,
        })
    }
    fn iv(material: &[u8; 36], seq: u64) -> [u8; 12] {
        let mut iv = [0; 12];
        iv[..4].copy_from_slice(&material[32..]);
        iv[4..].copy_from_slice(&seq.to_be_bytes());
        iv
    }
    pub fn seal(&mut self, value: &Value) -> Result<String> {
        let bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() <= MAX_FRAME / 2, "控制消息过大");
        let seq = self.sent;
        let iv = Self::iv(&self.tx, seq);
        let aad = format!("screenbridge-v1:{seq}");
        let cipher = Aes256Gcm::new_from_slice(&self.tx[..32]).unwrap();
        let data = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &bytes,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("加密失败"))?;
        self.sent = self.sent.checked_add(1).context("消息序号耗尽")?;
        let (data, tag) = data.split_at(data.len() - 16);
        Ok(serde_json::to_string(&Frame {
            seq: seq.to_string(),
            data: STANDARD.encode(data),
            tag: STANDARD.encode(tag),
        })?)
    }
    pub fn open(&mut self, text: &str) -> Result<Value> {
        ensure!(text.len() <= MAX_FRAME, "控制消息过大");
        let f: Frame = serde_json::from_str(text)?;
        ensure!(f.seq == self.received.to_string(), "控制消息序号不匹配");
        let mut data = STANDARD.decode(f.data)?;
        let tag = STANDARD.decode(f.tag)?;
        ensure!(tag.len() == 16, "认证标记无效");
        data.extend(tag);
        let iv = Self::iv(&self.rx, self.received);
        let aad = format!("screenbridge-v1:{}", self.received);
        let cipher = Aes256Gcm::new_from_slice(&self.rx[..32]).unwrap();
        let bytes = cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &data,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("配对认证失败"))?;
        self.received = self.received.checked_add(1).context("消息序号耗尽")?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
pub struct Wire {
    pub socket: Framed<TcpStream, LinesCodec>,
    pub codec: Codec,
}
impl Wire {
    pub async fn handshake(stream: TcpStream, key: [u8; 32], host: bool) -> Result<Self> {
        stream.set_nodelay(true)?;
        let mut socket = Framed::new(stream, LinesCodec::new_with_max_length(MAX_FRAME));
        let client;
        let server;
        if host {
            let hello: Value = serde_json::from_str(&socket.next().await.context("对端已断开")??)?;
            ensure!(
                hello["kind"] == "hello" && hello["version"] == 1,
                "控制协议不兼容"
            );
            client = hello["nonce"].as_str().context("缺少随机数")?.to_owned();
            server = nonce();
            socket
                .send(serde_json::json!({"kind":"challenge", "nonce":server}).to_string())
                .await?;
        } else {
            client = nonce();
            socket
                .send(serde_json::json!({"kind":"hello","version":1,"nonce":client}).to_string())
                .await?;
            let challenge: Value =
                serde_json::from_str(&socket.next().await.context("对端已断开")??)?;
            ensure!(challenge["kind"] == "challenge", "握手格式无效");
            server = challenge["nonce"]
                .as_str()
                .context("缺少随机数")?
                .to_owned();
        }
        let mut wire = Self {
            socket,
            codec: Codec::new(&key, &client, &server, host)?,
        };
        if !host {
            wire.send(&serde_json::json!({"kind":"auth"})).await?;
        }
        let auth = wire.receive().await?;
        ensure!(
            auth["kind"] == if host { "auth" } else { "accepted" },
            "配对认证失败"
        );
        if host {
            wire.send(&serde_json::json!({"kind":"accepted"})).await?;
        }
        Ok(wire)
    }
    pub async fn send(&mut self, value: &Value) -> Result<()> {
        self.socket.send(self.codec.seal(value)?).await?;
        Ok(())
    }
    pub async fn receive(&mut self) -> Result<Value> {
        match self.socket.next().await {
            Some(line) => self.codec.open(&line?),
            None => bail!("对端已断开"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typescript_interoperability_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/control-protocol.json"))
                .unwrap();
        let key: [u8; 32] = hex::decode(fixture["key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let c = fixture["clientNonce"].as_str().unwrap();
        let s = fixture["serverNonce"].as_str().unwrap();
        let mut sender = Codec::new(&key, c, s, true).unwrap();
        let mut receiver = Codec::new(&key, c, s, false).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&sender.seal(&fixture["payload"]).unwrap()).unwrap(),
            fixture["rustFrame"]
        );
        assert_eq!(
            receiver.open(&fixture["frame"].to_string()).unwrap(),
            fixture["payload"]
        );
    }
    #[test]
    fn detects_replay_and_wrong_key() {
        let c = "01".repeat(32);
        let s = "02".repeat(32);
        let mut a = Codec::new(&[7; 32], &c, &s, true).unwrap();
        let mut b = Codec::new(&[7; 32], &c, &s, false).unwrap();
        let frame = a.seal(&serde_json::json!({"kind":"ping"})).unwrap();
        assert_eq!(b.open(&frame).unwrap()["kind"], "ping");
        assert!(b.open(&frame).is_err());
        let mut wrong = Codec::new(&[8; 32], &c, &s, false).unwrap();
        assert!(wrong.open(&frame).is_err());
    }
    #[test]
    fn pairing_is_canonical() {
        let code = new_pairing();
        assert!(pairing_key(&code).is_ok());
        assert!(pairing_key("SB1-bad").is_err());
    }
    #[tokio::test]
    async fn actual_tcp_authentication_and_messages() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut w = Wire::handshake(s, [3; 32], true).await.unwrap();
            let value = w.receive().await.unwrap();
            w.send(&value).await.unwrap();
        });
        let mut w = Wire::handshake(TcpStream::connect(addr).await.unwrap(), [3; 32], false)
            .await
            .unwrap();
        w.send(&serde_json::json!({"kind":"test"})).await.unwrap();
        assert_eq!(w.receive().await.unwrap()["kind"], "test");
        server.await.unwrap();
    }
}
