use std::time::Duration;

/// Discover and list the hubs on your network
#[derive(clap::Parser, Debug)]
pub struct ListHubsCommand {
    /// How long to wait for discovery to complete, in seconds
    #[arg(long, default_value = "15")]
    timeout: u64,
}

impl ListHubsCommand {
    pub async fn run(&self, _args: &crate::Args) -> anyhow::Result<()> {
        let mut hubs =
            crate::discovery::resolve_hubs(Some(Duration::from_secs(self.timeout))).await?;

        while let Some(hub) = hubs.recv().await {
            if let Some(user_data) = &hub.user_data {
                println!(
                    "{addr} SN={serial} MAC={mac} {name}",
                    addr = hub.hub.addr(),
                    serial = user_data.serial_number,
                    name = user_data.hub_name.to_string(),
                    mac = user_data.mac_address
                );
            } else {
                println!("{} (Not responding)", hub.hub.addr());
            }
        }

        Ok(())
    }
}
