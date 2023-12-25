use crate::api_types::UserData;
use crate::hub::Hub;
use anyhow::Context;
use std::net::IpAddr;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;
use wez_mdns::{QueryParameters, RecordKind};

pub const POWERVIEW_SERVICE: &str = "_powerview._tcp.local";

fn ip_from_response(response: wez_mdns::Response) -> anyhow::Result<IpAddr> {
    let mut ipv4 = None;
    let mut ipv6 = None;

    for record in &response.additional {
        match record.kind {
            RecordKind::A(v4) => {
                ipv4.replace(v4);
            }
            RecordKind::AAAA(v6) => {
                ipv6.replace(v6);
            }
            _ => {}
        }
    }

    if let Some(v4) = ipv4 {
        Ok(v4.into())
    } else if let Some(v6) = ipv6 {
        Ok(v6.into())
    } else {
        anyhow::bail!("failed to resolve a v4 or v6 address for the hub. {response:?}");
    }
}

/// Discover a hub on the local network
pub async fn resolve_hub(timeout: Duration) -> anyhow::Result<IpAddr> {
    let params = QueryParameters {
        timeout_after: Some(timeout),
        ..QueryParameters::SERVICE_LOOKUP
    };

    let response = wez_mdns::resolve_one(POWERVIEW_SERVICE, params)
        .await
        .context("MDNS discovery")?;

    ip_from_response(response)
}

#[derive(Clone, Debug)]
pub struct ResolvedHub {
    pub hub: Hub,
    pub user_data: Option<UserData>,
}

impl ResolvedHub {
    async fn new(addr: IpAddr) -> Self {
        let hub = Hub::with_addr(addr);
        Self::with_hub(hub).await
    }

    pub async fn with_hub(hub: Hub) -> Self {
        let user_data = hub.get_user_data().await.ok();
        ResolvedHub { hub, user_data }
    }
}

impl std::ops::Deref for ResolvedHub {
    type Target = Hub;
    fn deref(&self) -> &Hub {
        &self.hub
    }
}

pub async fn resolve_hub_with_serial(
    timeout: Option<Duration>,
    serial: &str,
) -> anyhow::Result<Hub> {
    let mut rx = resolve_hubs(timeout).await?;
    while let Some(hub) = rx.recv().await {
        if let Some(user_data) = &hub.user_data {
            if user_data.serial_number == serial {
                return Ok(hub.hub);
            }
        }
    }
    anyhow::bail!("No hub found with serial {serial}");
}

pub async fn resolve_hubs(timeout: Option<Duration>) -> anyhow::Result<Receiver<ResolvedHub>> {
    let params = QueryParameters {
        timeout_after: timeout,
        ..QueryParameters::DISCOVERY
    };

    let disco_rx = wez_mdns::resolve(POWERVIEW_SERVICE, params)
        .await
        .context("MDNS discovery")?;
    let (tx, rx) = tokio::sync::mpsc::channel(8);

    tokio::spawn(async move {
        while let Ok(response) = disco_rx.recv().await {
            match ip_from_response(response) {
                Ok(addr) => {
                    let resolved = ResolvedHub::new(addr).await;
                    if let Err(err) = tx.send(resolved).await {
                        log::error!("resolve_hubs: tx.send error: {err:#?}");
                        break;
                    }
                }
                Err(err) => {
                    log::error!("{err:#?}");
                }
            }
        }
        log::warn!("fell out of disco loop");
    });

    Ok(rx)
}
