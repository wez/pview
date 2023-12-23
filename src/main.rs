use anyhow::Context;
use clap::Parser;
use reqwest::Method;
use std::collections::BTreeMap;
use tabout::{Alignment, Column};
use tokio::sync::Mutex;

mod api_types;
mod discovery;
mod http_helpers;
mod hub;

use crate::api_types::*;
use crate::hub::*;

#[derive(Parser, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: SubCommand,

    #[clap(skip)]
    hub: Mutex<Option<Hub>>,
}

#[derive(Parser, Debug)]
pub struct ListShadesCommand {
    /// Only return shades in the specified room
    #[clap(long)]
    room: Option<String>,
}

impl ListShadesCommand {
    pub async fn run(&self, args: &Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;

        let opt_room_id = match &self.room {
            Some(name) => Some(hub.room_by_name(name).await?.id),
            None => None,
        };

        let rooms = hub.list_rooms().await?;

        let shades = hub.list_shades(None, opt_room_id).await?;

        let mut shades_by_room = BTreeMap::new();
        for shade in shades {
            let room = shades_by_room
                .entry(shade.room_id.unwrap_or(0))
                .or_insert_with(|| vec![]);
            room.push(shade);
        }

        let columns = &[
            Column {
                name: "ROOM".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "SHADE".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "POSITION".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "SECONDARY".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "POWER".to_string(),
                alignment: Alignment::Left,
            },
        ];
        let mut rows = vec![];
        for room_data in &rooms {
            if let Some(shades) = shades_by_room.get(&room_data.id) {
                for shade in shades {
                    let (pos1, pos2) = match &shade.positions {
                        Some(p) => (
                            p.position_1.to_string(),
                            p.position_2
                                .map(|s| s.to_string())
                                .unwrap_or_else(String::new),
                        ),
                        None => (String::new(), String::new()),
                    };

                    rows.push(vec![
                        room_data.name.to_string(),
                        shade
                            .name
                            .as_ref()
                            .map(|s| s.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        pos1,
                        pos2,
                        format!("{:?}", shade.battery_kind),
                    ]);
                }
            }
        }
        println!("{}", tabout::tabulate_output_as_string(columns, &rows)?);
        Ok(())
    }
}

#[derive(Parser, Debug)]
pub struct InspectShadeCommand {
    /// The name or id of the shade to inspect.
    /// Names will be compared ignoring case.
    name: String,
}

impl InspectShadeCommand {
    pub async fn run(&self, args: &Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;

        let shade = hub.shade_by_name(&self.name).await?;

        println!("{shade:#?}");
        Ok(())
    }
}

#[derive(Parser, Debug)]
pub enum SubCommand {
    ListShades(ListShadesCommand),
    InspectShade(InspectShadeCommand),
}

impl SubCommand {
    pub async fn run(&self, args: &Args) -> anyhow::Result<()> {
        match self {
            Self::ListShades(cmd) => cmd.run(args).await,
            Self::InspectShade(cmd) => cmd.run(args).await,
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
    let args = Args::parse();
    args.run().await
}
