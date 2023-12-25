use crate::api_types::*;
use crate::discovery::resolve_hub;
use crate::http_helpers::{get_request_with_json_response, request_with_json_response};
use anyhow::Context;
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct Hub {
    addr: IpAddr,
}

impl Hub {
    fn url(&self, extra: &str) -> String {
        format!("http://{}/{extra}", self.addr)
    }

    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    pub async fn list_rooms(&self) -> anyhow::Result<Vec<RoomData>> {
        let mut resp: RoomResponse = get_request_with_json_response(self.url("api/rooms")).await?;
        resp.room_data
            .sort_by_key(|item| (item.order, item.name.to_string()));
        Ok(resp.room_data)
    }

    pub async fn list_scenes(&self) -> anyhow::Result<Vec<Scene>> {
        let mut resp: ScenesResponse =
            get_request_with_json_response(self.url("api/scenes")).await?;
        resp.scene_data
            .sort_by_key(|item| (item.order, item.name.clone()));

        Ok(resp.scene_data)
    }

    pub async fn list_scene_members(&self) -> anyhow::Result<HashMap<i32, Vec<SceneMember>>> {
        let resp: SceneMembersResponse =
            get_request_with_json_response(self.url("api/scenemembers")).await?;

        let mut by_scene = HashMap::new();
        for member in resp.scene_member_data {
            by_scene
                .entry(member.scene_id)
                .or_insert_with(|| vec![])
                .push(member);
        }

        Ok(by_scene)
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

    pub fn with_addr(addr: IpAddr) -> Self {
        Self { addr }
    }

    pub async fn discover(timeout: Duration) -> anyhow::Result<Self> {
        let addr = resolve_hub(timeout).await.context(
            "Failed to discover the PowerView Hub. \
             Ensure that pview is running on the same network as the Hub!",
        )?;
        Ok(Self::with_addr(addr))
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

    pub async fn change_shade_position(
        &self,
        shade_id: i32,
        position: ShadePosition,
    ) -> anyhow::Result<ShadeData> {
        let url = self.url(&format!("api/shades/{shade_id}"));

        #[derive(Deserialize, Debug)]
        struct Response {
            shade: ShadeData,
        }

        let response: Response = request_with_json_response(
            Method::PUT,
            url,
            &json!({
                "shade": {
                    "positions": position
                }
            }),
        )
        .await?;
        Ok(response.shade)
    }

    pub async fn move_shade(
        &self,
        shade_id: i32,
        motion: ShadeUpdateMotion,
    ) -> anyhow::Result<ShadeData> {
        let url = self.url(&format!("api/shades/{shade_id}"));

        #[derive(Deserialize, Debug)]
        struct Response {
            shade: ShadeData,
        }

        let response: Response = request_with_json_response(
            Method::PUT,
            url,
            &json!({
                "shade": {
                    "motion": motion
                }
            }),
        )
        .await?;
        Ok(response.shade)
    }

    /// Returns the list of affected shades
    pub async fn activate_scene(&self, scene_id: i32) -> anyhow::Result<Vec<i32>> {
        let url = self.url(&format!("api/scenes?sceneId={scene_id}"));

        #[derive(Deserialize, Debug)]
        #[serde(rename_all = "camelCase")]
        struct Response {
            shade_ids: Vec<i32>,
        }
        let response: Response = get_request_with_json_response(url).await?;

        Ok(response.shade_ids)
    }

    pub async fn scene_by_name(&self, name: &str) -> anyhow::Result<Scene> {
        let scenes = self.list_scenes().await?;
        for s in scenes {
            if s.name.eq_ignore_ascii_case(name) {
                return Ok(s);
            }
        }
        anyhow::bail!("No scene with name matching '{name}' was found");
    }

    pub async fn shade_by_id(&self, shade_id: i32) -> anyhow::Result<ShadeData> {
        let url = self.url(&format!("api/shades/{shade_id}"));

        #[derive(Deserialize, Debug)]
        #[serde(rename_all = "camelCase")]
        struct Response {
            shade: ShadeData,
        }
        let response: Response = get_request_with_json_response(url).await?;

        Ok(response.shade)
    }

    pub async fn shade_by_name(&self, name: &str) -> anyhow::Result<ResolvedShadeData> {
        let shades = self.list_shades(None, None).await?;
        for shade in shades {
            if shade.name().eq_ignore_ascii_case(name) {
                return Ok(ResolvedShadeData::Primary(shade));
            }
            if shade.secondary_name().as_str().eq_ignore_ascii_case(name) {
                return Ok(ResolvedShadeData::Secondary(shade));
            }
            if shade.id.to_string() == name {
                return Ok(ResolvedShadeData::Primary(shade));
            }
        }
        anyhow::bail!(
            "No shade with name, secondary name or id matching provided '{name}' was found"
        );
    }

    pub async fn get_user_data(&self) -> anyhow::Result<UserData> {
        let resp: UserDataResponse =
            get_request_with_json_response(self.url("api/userdata")).await?;
        Ok(resp.user_data)
    }

    /// Figure out the local address that is determined
    /// by the kernel for communication with the hub
    pub async fn suggest_bind_address(&self) -> anyhow::Result<IpAddr> {
        let stream = TcpStream::connect((self.addr, 80)).await?;
        Ok(stream.local_addr()?.ip())
    }

    pub async fn enable_home_automation_hook(&self, postback_url: &str) -> anyhow::Result<()> {
        let url = self.url("api/homeautomation");

        let _res: serde_json::Value = request_with_json_response(
            Method::PUT,
            url,
            &json!({
                "homeautomation": {
                    "enabled": true,
                    "postBackUrl": postback_url
                }
            }),
        )
        .await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum ResolvedShadeData {
    Primary(ShadeData),
    Secondary(ShadeData),
}

impl ResolvedShadeData {
    pub fn is_primary(&self) -> bool {
        matches!(self, Self::Primary(_))
    }
}

impl std::ops::Deref for ResolvedShadeData {
    type Target = ShadeData;
    fn deref(&self) -> &ShadeData {
        match self {
            Self::Primary(a) | Self::Secondary(a) => a,
        }
    }
}
