use anyhow::Context;
use reqwest::Method;
use std::collections::BTreeMap;
use std::net::IpAddr;
use tabout::{Alignment, Column};

mod api_types;
mod discovery;
mod http_helpers;

use crate::api_types::*;

#[derive(Debug)]
struct Hub {
    addr: IpAddr,
}

impl Hub {
    fn url(&self, extra: &str) -> String {
        format!("http://{}/{extra}", self.addr)
    }

    pub async fn list_rooms(&self) -> anyhow::Result<Vec<RoomData>> {
        let mut resp: RoomResponse =
            http_helpers::get_request_with_json_response(self.url("api/rooms")).await?;
        resp.room_data.sort_by(|a, b| a.order.cmp(&b.order));
        Ok(resp.room_data)
    }

    pub async fn list_shades(
        &self,
        group_id: Option<i32>,
        room_id: Option<i32>,
    ) -> anyhow::Result<Vec<ShadeData>> {
        let params = match (group_id, room_id) {
            (Some(g), Some(r)) => format!("?groupId={g}&roomId={r}"),
            (Some(g), None) => format!("?groupId={g}"),
            (None, Some(r)) => format!("?roomId={r}"),
            (None, None) => String::new(),
        };
        let url = self.url(&format!("api/shades{params}"));

        let mut resp: ShadesResponse = http_helpers::get_request_with_json_response(url).await?;
        resp.shade_data.sort_by(|a, b| a.order.cmp(&b.order));

        Ok(resp.shade_data)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Hello, world!");

    let addr = discovery::resolve_hub().await?;
    let hub = Hub { addr };
    println!("Hub: {addr:?}");

    let rooms: BTreeMap<String, RoomData> = hub
        .list_rooms()
        .await?
        .into_iter()
        .map(|room| (room.name.to_string(), room))
        .collect();
    // println!("Rooms: {rooms:#?}");

    let shades = hub.list_shades(None, None).await?;
    // println!("Shades: {shades:#?}");

    let mut shades_by_room = BTreeMap::new();
    for shade in shades {
        let room = shades_by_room
            .entry(shade.room_id.unwrap_or(0))
            .or_insert_with(|| vec![]);
        room.push(shade);
    }
    for shades_in_room in shades_by_room.values_mut() {
        shades_in_room.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let columns = &[
        Column {
            name: "Room".to_string(),
            alignment: Alignment::Left,
        },
        Column {
            name: "Shade".to_string(),
            alignment: Alignment::Left,
        },
        Column {
            name: "Position".to_string(),
            alignment: Alignment::Left,
        },
        Column {
            name: "Secondary".to_string(),
            alignment: Alignment::Left,
        },
    ];
    let mut rows = vec![];
    for (room_name, room_data) in &rooms {
        if let Some(shades) = shades_by_room.get(&room_data.id) {
            for shade in shades {

                let (pos1, pos2) = match &shade.positions {
                    Some(p) => {
                        (p.position_1.to_string(),
                         p.position_2.map(|s| s.to_string()).unwrap_or_else(String::new))
                    }
                    None => (String::new(), String::new())
                };

                rows.push(vec![
                    room_name.to_string(),
                    shade
                        .name
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                        pos1,
                        pos2,
                ]);
            }
        }
    }
    println!("{}", tabout::tabulate_output_as_string(columns, &rows)?);

    Ok(())
}
