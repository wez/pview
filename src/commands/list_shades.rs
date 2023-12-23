use std::collections::BTreeMap;
use tabout::{Alignment, Column};

#[derive(clap::Parser, Debug)]
pub struct ListShadesCommand {
    /// Only return shades in the specified room
    #[clap(long)]
    room: Option<String>,
}

impl ListShadesCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
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
                alignment: Alignment::Right,
            },
        ];
        let mut rows = vec![];
        for room_data in &rooms {
            if let Some(shades) = shades_by_room.get(&room_data.id) {
                for shade in shades {
                    if let Some(pos) = shade.positions.as_ref() {
                        rows.push(vec![
                            room_data.name.to_string(),
                            shade.name().to_string(),
                            pos.describe_pos1(),
                        ]);

                        if pos.pos_kind_2.is_some() {
                            rows.push(vec![
                                room_data.name.to_string(),
                                shade.secondary_name(),
                                pos.describe_pos2(),
                            ]);
                        }
                    }
                }
            }
        }
        println!("{}", tabout::tabulate_output_as_string(columns, &rows)?);
        Ok(())
    }
}
