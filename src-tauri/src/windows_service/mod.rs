//! Opt-in Windows receiver service. Only the official, pinned core performs
//! input injection; this module owns its privileged launch and lifetime.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub const PROTOCOL: u8 = 1;
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const LEASE_SECONDS: u64 = 10;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Launch {
    pub protocol: u8,
    pub version: String,
    pub local: uuid::Uuid,
    pub host: uuid::Uuid,
    pub address: IpAddr,
    pub port: u16,
    pub certificate: String,
    pub fingerprint: String,
    pub swap_modifiers: bool,
}
impl Launch {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.protocol == PROTOCOL && self.version == env!("CARGO_PKG_VERSION"),
            "后台服务版本不匹配，请停止共享后更新服务"
        );
        ensure!(
            self.port >= 1024 && self.local != self.host,
            "服务连接参数无效"
        );
        ensure!(
            !self.address.is_unspecified() && !self.address.is_multicast(),
            "服务目标地址无效"
        );
        ensure!(
            crate::model::valid_fingerprint(&self.fingerprint),
            "配对证书指纹无效"
        );
        ensure!(self.certificate.len() <= 16 * 1024, "引擎证书过大");
        rcgen::KeyPair::from_pem(&self.certificate)?;
        // The app stores a private key followed by a certificate in one PEM.
        // rcgen's certificate parser expects the certificate to be first.
        let cert = self
            .certificate
            .split("-----BEGIN CERTIFICATE-----")
            .nth(1)
            .context("缺少引擎证书")?
            .split_once("-----END CERTIFICATE-----")
            .context("引擎证书不完整")?
            .0;
        rcgen::CertificateParams::from_ca_cert_pem(&format!(
            "-----BEGIN CERTIFICATE-----{cert}-----END CERTIFICATE-----"
        ))?;
        Ok(())
    }
    pub fn settings(&self, root: &std::path::Path) -> Result<String> {
        self.validate()?;
        let cert = root
            .join("tls/identity.pem")
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(!cert.contains(['"', '\r', '\n', '\0']), "服务目录无效");
        let address = std::net::SocketAddr::new(self.address, self.port);
        let (ctrl, super_key) = if self.swap_modifiers {
            ("super", "ctrl")
        } else {
            ("ctrl", "super")
        };
        // No arbitrary executable, command, path, server configuration or INI
        // text is accepted from the unprivileged process.
        Ok(format!("[core]\ncomputerName=sb-{}\ncoreMode=1\nprocessMode=1\nport={}\npreventSleep=false\n[security]\ntlsEnabled=true\ncheckPeerFingerprints=true\ncertificate=\"{cert}\"\n[client]\nremoteHost={address}\n[log]\nlevel=INFO\ntoFile=false\n[computer_sb-{}]\nname=sb-{}\n[computer_sb-{}]\nname=sb-{}\nmodifierCtrl={ctrl}\nmodifierSuper={super_key}\n", self.local, self.port, self.host, self.host, self.local, self.local))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    Start { launch: Launch },
    Ping,
    Stop,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Reply {
    Started { pid: u32 },
    Pong,
    Output { line: String },
    Stopped,
    Error { message: String },
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub current: bool,
}
#[cfg(windows)]
mod client;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use client::start;
#[cfg(windows)]
pub use host::run_if_requested;
#[cfg(windows)]
pub use install::{manage, status};
#[cfg(test)]
mod tests {
    use super::*;
    fn launch() -> Launch {
        let pair = rcgen::generate_simple_self_signed(vec!["test.local".into()]).unwrap();
        Launch {
            protocol: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").into(),
            local: uuid::Uuid::new_v4(),
            host: uuid::Uuid::new_v4(),
            address: "192.168.1.2".parse().unwrap(),
            port: 24801,
            certificate: format!("{}\n{}", pair.key_pair.serialize_pem(), pair.cert.pem()),
            fingerprint: "a".repeat(64),
            swap_modifiers: false,
        }
    }
    #[test]
    fn accepts_only_typed_receiver_configuration() {
        let l = launch();
        let text = l
            .settings(std::path::Path::new("C:/Program Files/DeskflowDDC/session"))
            .unwrap();
        assert!(text.contains("remoteHost=192.168.1.2:24801"));
        assert!(text.contains("checkPeerFingerprints=true"));
        assert!(!text.contains("externalConfig"));
        let mut value = serde_json::to_value(Command::Start { launch: l }).unwrap();
        value["launch"]["executable"] = "evil.exe".into();
        assert!(serde_json::from_value::<Command>(value).is_err());
    }
    #[test]
    fn rejects_version_invalid_certificate_and_address() {
        let mut l = launch();
        l.version = "old".into();
        assert!(l.validate().is_err());
        l.version = env!("CARGO_PKG_VERSION").into();
        l.address = "0.0.0.0".parse().unwrap();
        assert!(l.validate().is_err());
        l.address = "127.0.0.1".parse().unwrap();
        l.certificate = "invalid".into();
        assert!(l.validate().is_err());
    }
    #[test]
    fn swap_and_ipv6_preserve_engine_mapping() {
        let mut l = launch();
        l.address = "::1".parse().unwrap();
        l.swap_modifiers = true;
        let text = l.settings(std::path::Path::new("service")).unwrap();
        assert!(text.contains("remoteHost=[::1]:24801"));
        assert!(text.contains("modifierCtrl=super\nmodifierSuper=ctrl"));
    }
}
