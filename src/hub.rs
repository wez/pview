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

        let mut resp: ShadesResponse = get_request_with_json_response(url).await?;
        resp.shade_data.sort_by(|a, b| a.order.cmp(&b.order));

        Ok(resp.shade_data)
    }

    pub async fn discover() -> anyhow::Result<Self> {
        let addr = resolve_hub().await?;
        Ok(Hub { addr })
    }
}
