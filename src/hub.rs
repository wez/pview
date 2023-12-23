use crate::api_types::*;
use crate::discovery::resolve_hub;
use crate::http_helpers::get_request_with_json_response;
use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct Hub {
    addr: IpAddr,
}

impl Hub {
    fn url(&self, extra: &str) -> String {
        format!("http://{}/{extra}", self.addr)
    }

    pub async fn list_rooms(&self) -> anyhow::Result<Vec<RoomData>> {
        let mut resp: RoomResponse = get_request_with_json_response(self.url("api/rooms")).await?;
        resp.room_data
            .sort_by_key(|item| (item.order, item.name.to_string()));
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

        let mut resp: ShadesResponse = get_request_with_json_response(url).await?;
        resp.shade_data
            .sort_by_key(|item| (item.order, item.name.clone()));

        Ok(resp.shade_data)
    }

    pub async fn discover() -> anyhow::Result<Self> {
        let addr = resolve_hub().await?;
        Ok(Hub { addr })
    }

    pub async fn room_by_name(&self, name: &str) -> anyhow::Result<RoomData> {
        let rooms = self.list_rooms().await?;
        for room in rooms {
            if room.name.eq_ignore_ascii_case(name) {
                return Ok(room);
            }
            if room.id.to_string() == name {
                return Ok(room);
            }
        }
        anyhow::bail!("No room with name or id matching provided '{name}' was found");
    }

    pub async fn shade_by_name(&self, name: &str) -> anyhow::Result<ShadeData> {
        let shades = self.list_shades(None, None).await?;
        for shade in shades {
            if let Some(shade_name) = &shade.name {
                if shade_name.as_str().eq_ignore_ascii_case(name) {
                    return Ok(shade);
                }
            }
            if let Some(shade_name) = &shade.secondary_name {
                if shade_name.as_str().eq_ignore_ascii_case(name) {
                    return Ok(shade);
                }
            }
            if shade.id.to_string() == name {
                return Ok(shade);
            }
        }
        anyhow::bail!(
            "No shade with name, secondary name or id matching provided '{name}' was found"
        );
    }
}
