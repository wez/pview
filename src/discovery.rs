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
pub async fn resolve_hub() -> anyhow::Result<IpAddr> {
    let response = wez_mdns::resolve_one(POWERVIEW_SERVICE, QueryParameters::SERVICE_LOOKUP)
        .await
        .context("MDNS discovery")?;

    ip_from_response(response)
}

#[derive(Debug)]
pub struct ResolvedHub {
    pub hub: Hub,
    pub user_data: Option<UserData>,
}

pub async fn resolve_hubs(timeout: Option<Duration>) -> anyhow::Result<Receiver<ResolvedHub>> {
    let mut params = QueryParameters::SERVICE_LOOKUP.clone();
    params.timeout_after = timeout;

    let disco_rx = wez_mdns::resolve(POWERVIEW_SERVICE, params)
        .await
        .context("MDNS discovery")?;
    let (tx, rx) = tokio::sync::mpsc::channel(8);

    tokio::spawn(async move {
        while let Ok(response) = disco_rx.recv().await {
            match ip_from_response(response) {
                Ok(addr) => {
                    let hub = Hub::with_addr(addr);
                    let user_data = hub.get_user_data().await.ok();
                    if let Err(err) = tx.send(ResolvedHub { hub, user_data }).await {
                        log::error!("resolve_hubs: tx.send error: {err:#?}");
                        break;
                    }
                }
                Err(err) => {
                    log::error!("{err:#?}");
                }
            }
        }
    });

    Ok(rx)
}
