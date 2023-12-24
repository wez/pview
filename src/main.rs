use clap::Parser;
use tokio::sync::Mutex;

mod api_types;
mod commands;
mod discovery;
mod http_helpers;
mod hub;

use crate::hub::*;

#[derive(Parser, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: SubCommand,

    #[clap(skip)]
    hub: Mutex<Option<Hub>>,
}

#[derive(Parser, Debug)]
pub enum SubCommand {
    ListScenes(commands::list_scenes::ListScenesCommand),
    ListShades(commands::list_shades::ListShadesCommand),
    InspectShade(commands::inspect_shade::InspectShadeCommand),
    MoveShade(commands::move_shade::MoveShadeCommand),
    ActivateScene(commands::activate_scene::ActivateSceneCommand),
    ServeMqtt(commands::serve_mqtt::ServeMqttCommand),
    HubInfo(commands::hub_info::HubInfoCommand),
}

impl SubCommand {
    pub async fn run(&self, args: &Args) -> anyhow::Result<()> {
        match self {
            Self::ListScenes(cmd) => cmd.run(args).await,
            Self::ListShades(cmd) => cmd.run(args).await,
            Self::InspectShade(cmd) => cmd.run(args).await,
            Self::MoveShade(cmd) => cmd.run(args).await,
            Self::ActivateScene(cmd) => cmd.run(args).await,
            Self::ServeMqtt(cmd) => cmd.run(args).await,
            Self::HubInfo(cmd) => cmd.run(args).await,
        }
    }
}

impl Args {
    pub async fn run(&self) -> anyhow::Result<()> {
        self.cmd.run(self).await
    }

    pub async fn hub(&self) -> anyhow::Result<Hub> {
        let mut lock = self.hub.lock().await;
        match lock.as_ref() {
            Some(hub) => Ok(hub.clone()),
            None => {
                let hub = Hub::discover().await?;
                lock.replace(hub.clone());
                Ok(hub)
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loading environment overrides from {path:?}");
    }
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let args = Args::parse();
    args.run().await
}
