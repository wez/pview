use std::collections::HashMap;
use tabout::{Alignment, Column};

/// List scenes and their associated shades
#[derive(clap::Parser, Debug)]
pub struct ListScenesCommand {
    /// Only return shades in the specified room
    #[clap(long)]
    room: Option<String>,
}

impl ListScenesCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;
        let mut scenes = hub.list_scenes().await?;

        if let Some(room) = &self.room {
            let room = hub.room_by_name(room).await?;
            scenes.retain(|scene| scene.room_id == room.id);
        }

        let shade_by_id: HashMap<_, _> = hub
            .list_shades(None, None)
            .await?
            .into_iter()
            .map(|shade| (shade.id, shade))
            .collect();

        let mut members_by_scene = hub.list_scene_members().await?;

        let columns = &[
            Column {
                name: "SCENE/SHADES".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "POSITION".to_string(),
                alignment: Alignment::Right,
            },
        ];
        let mut rows = vec![];

        for scene in scenes {
            rows.push(vec![scene.name.to_string()]);
            if let Some(members) = members_by_scene.get_mut(&scene.id) {
                members.sort_by_key(|m| {
                    let shade = &shade_by_id[&m.shade_id];
                    (shade.order, shade.name())
                });

                for m in members {
                    let shade = &shade_by_id[&m.shade_id];
                    rows.push(vec![
                        format!("    {}", shade.name()),
                        m.positions.describe(),
                    ]);
                }
            }
            rows.push(vec![]);
        }
        println!("{}", tabout::tabulate_output_as_string(columns, &rows)?);

        Ok(())
    }
}
