use crate::model::{Config, Pin};
use anyhow::{ensure, Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::net::{lookup_host, TcpStream};

pub const SERVICE: &str = "_deskflow-ddc._tcp.local.";
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
}
pub struct Discovery {
    daemon: ServiceDaemon,
    registered: Option<String>,
}
impl Discovery {
    pub fn new() -> Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            registered: None,
        })
    }
    pub fn advertise(&mut self, id: &str, config: &Config) -> Result<()> {
        self.stop_advertising();
        if config.role != "host" {
            return Ok(());
        }
        // UUID-derived hostname avoids collisions after renaming a computer.
        let host = format!("ddc-{id}.local.");
        let props = [
            ("id", id),
            ("name", config.device_name.as_str()),
            ("protocol", "1"),
        ];
        let addresses = local_addresses();
        ensure!(!addresses.is_empty(), "没有可发布的局域网地址");
        let info = ServiceInfo::new(
            SERVICE,
            &format!("ddc-{id}"),
            &host,
            addresses.as_slice(),
            config.control_port,
            &props[..],
        )?
        .enable_addr_auto();
        self.registered = Some(info.get_fullname().into());
        self.daemon.register(info)?;
        Ok(())
    }
    pub fn stop_advertising(&mut self) {
        if let Some(name) = self.registered.take() {
            let _ = self.daemon.unregister(&name);
        }
    }
    pub async fn browse(&self, wait: Duration) -> Result<Vec<Device>> {
        self.browse_until(wait, std::future::pending())
            .await
            .map(|(devices, _)| devices)
    }
    pub async fn browse_until(
        &self,
        wait: Duration,
        interrupt: impl std::future::Future<Output = ()>,
    ) -> Result<(Vec<Device>, bool)> {
        let receiver = self.daemon.browse(SERVICE)?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut found = BTreeMap::new();
        tokio::pin!(interrupt);
        let interrupted = loop {
            let event = tokio::select! {
                biased;
                _ = &mut interrupt => break true,
                event = tokio::time::timeout_at(deadline, receiver.recv_async()) => {
                    let Ok(Ok(event)) = event else { break false; };
                    event
                }
            };
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if info.get_property_val_str("protocol") != Some("1") {
                        continue;
                    }
                    let Some(id) = info.get_property_val_str("id") else {
                        continue;
                    };
                    if uuid::Uuid::parse_str(id).is_err() {
                        continue;
                    }
                    let device = Device {
                        id: id.into(),
                        name: info
                            .get_property_val_str("name")
                            .unwrap_or("附近的电脑")
                            .chars()
                            .take(48)
                            .collect(),
                        hostname: info.get_hostname().trim_end_matches('.').into(),
                        addresses: info
                            .get_addresses()
                            .iter()
                            .copied()
                            .filter(usable)
                            .collect(),
                        port: info.get_port(),
                    };
                    found.insert(info.get_fullname().to_owned(), device);
                }
                ServiceEvent::ServiceRemoved(_, name) => {
                    found.remove(&name);
                }
                _ => {}
            }
        };
        let _ = self.daemon.stop_browse(SERVICE);
        Ok((found.into_values().collect(), interrupted))
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop_advertising();
        let _ = self.daemon.shutdown();
    }
}
fn usable(ip: &IpAddr) -> bool {
    !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_loopback()
        && match ip {
            IpAddr::V6(v) => !v.is_unicast_link_local(),
            _ => true,
        }
}
pub fn local_addresses() -> Vec<IpAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .filter(usable)
        .collect()
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub address: SocketAddr,
    pub source: &'static str,
}
fn append(items: &mut Vec<Candidate>, address: SocketAddr, source: &'static str) {
    if !items.iter().any(|c| c.address == address) {
        items.push(Candidate { address, source })
    }
}
/// Discovery is only an address hint; the caller authenticates every candidate
/// before caching it or starting Deskflow. Cache is never a device identity.
pub fn immediate_candidates(
    config: &Config,
    pin: Option<&Pin>,
    cached: Option<IpAddr>,
    discovered: &[Device],
) -> Vec<Candidate> {
    let mut items = vec![];
    if pin.is_some() {
        if let Some(ip) = cached {
            append(
                &mut items,
                SocketAddr::new(ip, config.control_port),
                "cache",
            );
        }
    }
    if let Ok(ip) = config.peer_address.parse::<IpAddr>() {
        append(
            &mut items,
            SocketAddr::new(ip, config.control_port),
            "hostname",
        );
    }
    if let Some(pin) = pin {
        for device in discovered.iter().filter(|d| d.id == pin.id.to_string()) {
            for ip in &device.addresses {
                append(&mut items, SocketAddr::new(*ip, device.port), "discovery");
            }
        }
    }
    for ip in &config.fallback_ips {
        append(
            &mut items,
            SocketAddr::new(*ip, config.control_port),
            "fallback",
        );
    }
    items
}

pub async fn candidates(
    config: &Config,
    pin: Option<&Pin>,
    cached: Option<IpAddr>,
    discovered: &[Device],
) -> Result<Vec<Candidate>> {
    let mut items = immediate_candidates(config, pin, cached, discovered);
    if !config.peer_address.is_empty() {
        if let Ok(Ok(addresses)) = tokio::time::timeout(
            Duration::from_secs(2),
            lookup_host((config.peer_address.as_str(), config.control_port)),
        )
        .await
        {
            for address in addresses {
                append(&mut items, address, "hostname")
            }
        }
    }
    ensure!(
        !items.is_empty(),
        "主机名解析和设备发现未找到地址，请填写备用 IP"
    );
    Ok(items)
}
pub async fn connect(candidate: &Candidate) -> Result<TcpStream> {
    tokio::time::timeout(
        Duration::from_secs(2),
        TcpStream::connect(candidate.address),
    )
    .await
    .context("连接地址超时")?
    .context("无法连接提供端")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn browse_interrupt_stops_subscription_and_allows_next_browse() {
        let discovery = Discovery::new().unwrap();
        let (devices, interrupted) = tokio::time::timeout(
            Duration::from_millis(500),
            discovery.browse_until(Duration::from_secs(60), async {}),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(interrupted && devices.is_empty());
        discovery.browse(Duration::ZERO).await.unwrap();
    }
    #[tokio::test]
    async fn verified_address_is_tried_before_dns_and_new_discovery_without_skipping_authentication(
    ) {
        let config = Config {
            peer_address: "slow-or-unavailable.local".into(),
            fallback_ips: vec!["192.168.1.8".parse().unwrap()],
            ..Config::default()
        };
        let pin = Pin {
            id: uuid::Uuid::new_v4(),
            fingerprint: "a".repeat(64),
        };
        let device = Device {
            id: pin.id.to_string(),
            name: "Provider".into(),
            hostname: "provider.local".into(),
            addresses: vec!["192.168.1.9".parse().unwrap()],
            port: config.control_port,
        };
        let cached = Some("192.168.1.8".parse().unwrap());
        let all = immediate_candidates(&config, Some(&pin), cached, &[device]);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].source, "cache");
        assert_eq!(all[1].source, "discovery");
        // Cache without a paired identity is not promoted to a reconnect hint.
        assert_eq!(
            immediate_candidates(&config, None, cached, &[])[0].source,
            "fallback"
        );
        // An unrelated machine answering on the cached IP still fails the
        // existing authenticated handshake. No plaintext/identity bypass.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let remote = tokio::spawn(async move {
            crate::protocol::Wire::handshake(listener.accept().await.unwrap().0, [3; 32], true)
                .await
        });
        let local = crate::protocol::Wire::handshake(
            TcpStream::connect(address).await.unwrap(),
            [4; 32],
            false,
        )
        .await;
        assert!(local.is_err());
        assert!(remote.await.unwrap().is_err());
    }
    #[tokio::test]
    async fn hostname_failure_uses_ip_and_deduplicates_cache() {
        let c = Config {
            peer_address: "".into(),
            fallback_ips: vec!["192.168.1.8".parse().unwrap()],
            ..Config::default()
        };
        let all = candidates(&c, None, Some("192.168.1.8".parse().unwrap()), &[])
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, "fallback");
    }
    #[tokio::test]
    async fn discovered_name_does_not_override_pinned_identity() {
        let pin = Pin {
            id: uuid::Uuid::new_v4(),
            fingerprint: "a".repeat(64),
        };
        let d = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Same name".into(),
            hostname: "same.local".into(),
            addresses: vec!["192.168.1.3".parse().unwrap()],
            port: 1234,
        };
        assert!(candidates(&Config::default(), Some(&pin), None, &[d])
            .await
            .is_err());
    }
}
