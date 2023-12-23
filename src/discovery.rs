use std::net::IpAddr;
use wez_mdns::RecordKind;

/// Discover a hub on the local network
pub async fn resolve_hub() -> anyhow::Result<IpAddr> {
    let response = wez_mdns::resolve_one(
        "_powerview._tcp.local",
        wez_mdns::QueryParameters::SERVICE_LOOKUP,
    )
    .await?;

    let mut ipv4 = None;
    let mut ipv6 = None;

    for record in response.additional {
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
        anyhow::bail!("failed to resolve a v4 or v6 address for the hub");
    }
}
