//! Bounded binary AEAD frames on a dedicated TCP connection. No file bytes
//! enter the control channel, WebView or diagnostic logs.
use super::{
    metrics::{self, Metrics},
    scheduling::Network,
};
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{ensure, Context, Result};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{de::DeserializeOwned, Serialize};
use sha2::Sha256;
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
const IDLE: Duration = Duration::from_secs(120);
pub fn connection_error(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        if cause.is::<tokio::time::error::Elapsed>() {
            return true;
        }
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotConnected
            )
        })
    })
}
async fn bounded<T>(io: impl std::future::Future<Output = std::io::Result<T>>) -> Result<T> {
    tokio::time::timeout(IDLE, io)
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "文件通道读写超时（120 秒）")
        })?
        .map_err(Into::into)
}
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub const CHUNK: usize = 256 * 1024;
// Stay within the existing frame maximum without a timer/filesystem handoff
// for every 32 KiB. A bounded send queue and latency pacing protect input.
pub const SEND_CHUNK: usize = 128 * 1024;
const MAX: usize = CHUNK + 4096;
pub struct Wire {
    socket: TcpStream,
    tx: [u8; 36],
    rx: [u8; 36],
    encrypt: Aes256Gcm,
    decrypt: Aes256Gcm,
    buffer: Vec<u8>,
    pub metrics: Arc<Metrics>,
    network: Arc<Network>,
    sent: u64,
    received: u64,
}
impl Wire {
    pub async fn handshake(mut socket: TcpStream, secret: &[u8; 32], server: bool) -> Result<Self> {
        socket.set_nodelay(true)?;
        // A 32 KiB send buffer can starve a Windows/Wi-Fi connection between
        // acknowledgements. Keep a bounded 256 KiB queue and use measured
        // sustained congestion to pace bulk traffic. Receive auto-tuning stays on.
        socket2::SockRef::from(&socket).set_send_buffer_size(256 * 1024)?;
        let mut local = [0; 32];
        rand::thread_rng().fill_bytes(&mut local);
        socket.write_all(&local).await?;
        let mut remote = [0; 32];
        socket.read_exact(&mut remote).await?;
        let (client, host) = if server {
            (remote, local)
        } else {
            (local, remote)
        };
        let mut salt = Vec::from(client);
        salt.extend(host);
        let hk = Hkdf::<Sha256>::new(Some(&salt), secret);
        let mut s2c = [0; 36];
        let mut c2s = [0; 36];
        hk.expand(b"deskflow-ddc/files/v1/s2c", &mut s2c)
            .map_err(|_| anyhow::anyhow!("文件密钥派生失败"))?;
        hk.expand(b"deskflow-ddc/files/v1/c2s", &mut c2s)
            .map_err(|_| anyhow::anyhow!("文件密钥派生失败"))?;
        let (tx, rx) = if server { (s2c, c2s) } else { (c2s, s2c) };
        let mut wire = Self {
            socket,
            encrypt: Aes256Gcm::new_from_slice(&tx[..32]).unwrap(),
            decrypt: Aes256Gcm::new_from_slice(&rx[..32]).unwrap(),
            buffer: Vec::with_capacity(MAX + 20),
            metrics: Arc::new(Metrics::default()),
            network: Arc::new(Network::default()),
            tx,
            rx,
            sent: 0,
            received: 0,
        };
        wire.json(&"deskflow-ddc-files-v1").await?;
        ensure!(
            wire.read_json::<String>().await? == "deskflow-ddc-files-v1",
            "文件连接认证失败"
        );
        Ok(wire)
    }
    pub fn configure(&mut self, network: Arc<Network>, metrics: Arc<Metrics>) {
        self.network = network;
        self.metrics = metrics;
    }
    fn iv(key: &[u8; 36], seq: u64) -> [u8; 12] {
        let mut iv = [0; 12];
        iv[..4].copy_from_slice(&key[32..]);
        iv[4..].copy_from_slice(&seq.to_be_bytes());
        iv
    }
    pub async fn send(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= MAX, "文件帧过大");
        let paced = Instant::now();
        self.network.pace(bytes.len()).await;
        metrics::elapsed(&self.metrics.pacing, paced);
        let crypto = Instant::now();
        self.buffer.clear();
        self.buffer.extend_from_slice(&[0; 4]);
        self.buffer.extend_from_slice(bytes);
        let tag = self
            .encrypt
            .encrypt_in_place_detached(
                Nonce::from_slice(&Self::iv(&self.tx, self.sent)),
                b"",
                &mut self.buffer[4..],
            )
            .map_err(|_| anyhow::anyhow!("文件加密失败"))?;
        self.buffer.extend_from_slice(&tag);
        let length = (self.buffer.len() - 4) as u32;
        self.buffer[..4].copy_from_slice(&length.to_be_bytes());
        metrics::elapsed(&self.metrics.encrypt, crypto);
        self.sent = self.sent.checked_add(1).context("文件帧序号耗尽")?;
        let io = Instant::now();
        let result = bounded(self.socket.write_all(&self.buffer)).await;
        metrics::elapsed(&self.metrics.network_write, io);
        result?;
        self.network.sent(bytes.len());
        self.metrics
            .sent
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        tokio::task::yield_now().await;
        Ok(())
    }
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        loop {
            let io = Instant::now();
            let result: Result<Vec<u8>> = async {
                let len = bounded(self.socket.read_u32()).await? as usize;
                ensure!((16..=MAX + 16).contains(&len), "文件帧长度无效");
                let mut bytes = vec![0; len];
                bounded(self.socket.read_exact(&mut bytes)).await?;
                Ok(bytes)
            }
            .await;
            metrics::elapsed(&self.metrics.network_read, io);
            let mut plain = result?;
            let crypto = Instant::now();
            self.decrypt
                .decrypt_in_place(
                    Nonce::from_slice(&Self::iv(&self.rx, self.received)),
                    b"",
                    &mut plain,
                )
                .map_err(|_| anyhow::anyhow!("文件认证或完整性校验失败"))?;
            metrics::elapsed(&self.metrics.decrypt, crypto);
            self.metrics
                .received
                .fetch_add(plain.len() as u64, Ordering::Relaxed);
            tokio::task::yield_now().await;
            self.received = self.received.checked_add(1).context("文件帧序号耗尽")?;
            // Empty authenticated frames are keepalives while hashing a large prefix.
            if !plain.is_empty() {
                return Ok(plain);
            }
        }
    }
    pub async fn json<T: Serialize>(&mut self, v: &T) -> Result<()> {
        self.send(&serde_json::to_vec(v)?).await
    }
    pub async fn read_json<T: DeserializeOwned>(&mut self) -> Result<T> {
        Ok(serde_json::from_slice(&self.receive().await?)?)
    }
}
