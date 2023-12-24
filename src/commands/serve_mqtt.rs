use crate::api_types::{
    HomeAutomationPostBackData, HomeAutomationRecordType, HomeAutomationService, ShadePosition,
    ShadeUpdateMotion, UserData,
};
use crate::hub::Hub;
use mosquitto_rs::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};

const SECONDARY_SUFFIX: &str = "_2";

// <https://www.home-assistant.io/integrations/cover.mqtt/>

#[derive(clap::Parser, Debug)]
pub struct ServeMqttCommand {
    /// The mqtt broker hostname or address
    #[arg(long)]
    host: String,

    /// The mqtt broker port
    #[arg(long, default_value = "1883")]
    port: u16,

    /// The username to authenticate against the broker
    #[arg(long)]
    username: Option<String>,
    /// The password to authenticate against the broker
    #[arg(long)]
    password: Option<String>,

    #[arg(long)]
    bind_address: Option<String>,

    #[arg(long, default_value = "homeassistant")]
    discovery_prefix: String,
}

#[derive(Debug)]
enum ServerEvent {
    MqttMessage(Message),
    HomeAutomationData(Vec<HomeAutomationPostBackData>),
}

struct MqttMsg {
    pub topic: String,
    pub payload: Vec<u8>,
}

impl MqttMsg {
    pub fn new<T: Into<String>, P: Into<Vec<u8>>>(topic: T, payload: P) -> Self {
        Self {
            topic: topic.into(),
            payload: payload.into(),
        }
    }
}

struct HassRegistration<'a> {
    pub client: &'a mut Client,
    pub updates: Vec<MqttMsg>,
}

impl<'a> HassRegistration<'a> {
    pub async fn config<T: AsRef<str>, P: AsRef<[u8]>>(
        &mut self,
        topic: T,
        payload: P,
    ) -> anyhow::Result<()> {
        self.client
            .publish(topic.as_ref(), payload.as_ref(), QoS::AtMostOnce, false)
            .await?;
        Ok(())
    }

    pub fn update<T: Into<String>, P: Into<Vec<u8>>>(&mut self, topic: T, payload: P) {
        self.updates.push(MqttMsg::new(topic, payload));
    }

    pub async fn apply_updates(&mut self) -> anyhow::Result<()> {
        for msg in &self.updates {
            self.client
                .publish(&msg.topic, &msg.payload, QoS::AtMostOnce, false)
                .await?;
        }
        Ok(())
    }
}

impl ServeMqttCommand {
    async fn register_hub(
        &self,
        user_data: &UserData,
        reg: &mut HassRegistration<'_>,
    ) -> anyhow::Result<()> {
        let serial = &user_data.serial_number;
        let data = serde_json::json!({
            "name": "IP Address",
            "unique_id": format!("{serial}-hub-ip"),
            "state_topic": format!("pv2mqtt/sensor/{serial}-hub-ip/state"),
            "availability_topic": format!("pv2mqtt/sensor/{serial}-hub-ip/availability"),
            "device": {
                "identifiers": [
                    format!("pv2mqtt-{serial}"),
                    user_data.serial_number,
                    user_data.mac_address,
                ],
                "connections": [
                    ["mac", user_data.mac_address],
                ],
                "name": format!("{} PowerView Hub: {}", user_data.brand, user_data.hub_name.to_string()),
                "manufacturer": "Wez Furlong",
                "model": "pv2mqtt",
            },
            "entity_category": "diagnostic",
            "origin": {
                "name": "pv2mqtt",
                "sw": "0.0",
                "url": "https://github.com/wez/pview",
            },
        });

        reg.config(
            format!("{}/sensor/{serial}-hub-ip/config", self.discovery_prefix),
            serde_json::to_string(&data)?,
        )
        .await?;

        reg.update(
            format!("pv2mqtt/sensor/{serial}-hub-ip/availability"),
            "online",
        );

        reg.update(
            format!("pv2mqtt/sensor/{serial}-hub-ip/state"),
            user_data.ip.clone(),
        );

        Ok(())
    }

    async fn register_scenes(
        &self,
        user_data: &UserData,
        hub: &Hub,
        reg: &mut HassRegistration<'_>,
    ) -> anyhow::Result<()> {
        let scenes = hub.list_scenes().await?;
        let room_by_id: HashMap<_, _> = hub
            .list_rooms()
            .await?
            .into_iter()
            .map(|room| (room.id, room.name))
            .collect();

        for scene in scenes {
            let scene_id = scene.id;
            let scene_name = scene.name.to_string();

            let area = room_by_id
                .get(&scene.room_id)
                .map(|name| serde_json::json!(name.as_str()))
                .unwrap_or(serde_json::Value::Null);

            if !scene_name.contains("Study") {
                continue;
            }

            let unique_id = format!("{}-scene-{scene_id}", user_data.serial_number);

            let data = serde_json::json!({
                "name": serde_json::Value::Null,
                "unique_id": unique_id,
                "availability_topic": format!("pv2mqtt/scene/{scene_id}/availability"),
                "command_topic": format!("pv2mqtt/scene/{scene_id}/set"),
                "payload_on": "ON",
                "device": {
                    "suggested_area": area,
                    "identifiers": [
                        unique_id,
                    ],
                    "via_device": format!("pv2mqtt-{}", user_data.serial_number),
                    "name": scene_name,
                    "manufacturer": "Wez Furlong",
                    "model": "pv2mqtt",
                },
            });

            // Tell hass about this shade
            reg.config(
                format!("{}/scene/{unique_id}/config", self.discovery_prefix),
                serde_json::to_string(&data)?,
            )
            .await?;

            reg.update(format!("pv2mqtt/scene/{scene_id}/availability"), "online");
        }

        Ok(())
    }

    async fn register_shades(
        &self,
        user_data: &UserData,
        hub: &Hub,
        reg: &mut HassRegistration<'_>,
    ) -> anyhow::Result<()> {
        let shades = hub.list_shades(None, None).await?;
        let room_by_id: HashMap<_, _> = hub
            .list_rooms()
            .await?
            .into_iter()
            .map(|room| (room.id, room.name))
            .collect();

        for shade in &shades {
            if shade.name() != "Study Sheer" {
                continue;
            }
            let unique_id = format!("{}-{}", user_data.serial_number, shade.id);

            let position = match shade.positions.clone() {
                Some(p) => p,
                None => continue,
            };

            let mut shades = vec![(
                shade.id.to_string(),
                shade.name().to_string(),
                position.pos1_percent(),
            )];
            if let Some(pos2) = position.pos2_percent() {
                shades.push((
                    format!("{}{SECONDARY_SUFFIX}", shade.id),
                    shade.secondary_name(),
                    pos2,
                ));
            }

            for (shade_id, shade_name, pos) in shades {
                let area = shade
                    .room_id
                    .and_then(|room_id| {
                        room_by_id
                            .get(&room_id)
                            .map(|name| serde_json::json!(name.as_str()))
                    })
                    .unwrap_or(serde_json::Value::Null);

                let data = serde_json::json!({
                    "name": serde_json::Value::Null,
                    "device_class": "shade",
                    "unique_id": unique_id,
                    "state_topic": format!("pv2mqtt/shade/{shade_id}/state"),
                    "position_topic": format!("pv2mqtt/shade/{shade_id}/position"),
                    "availability_topic": format!("pv2mqtt/shade/{shade_id}/availability"),
                    "set_position_topic": format!("pv2mqtt/shade/{shade_id}/set_position"),
                    "command_topic": format!("pv2mqtt/shade/{shade_id}/command"),
                    "device": {
                        "suggested_area": area,
                        "identifiers": [
                            unique_id
                        ],
                        "via_device": format!("pv2mqtt-{}", user_data.serial_number),
                        "name": shade_name,
                        "manufacturer": "Hunter Douglas",
                        "model": "pv2mqtt",
                        "sw_version": shade.firmware.as_ref().map(|vers| {
                            format!("{}.{}.{}", vers.revision, vers.sub_revision, vers.build)
                        }).unwrap_or_else(|| "unknown".to_string()),
                    },
                    "origin": {
                        "name": "pv2mqtt",
                        "sw": "0.0",
                        "url": "https://github.com/wez/pview",
                    },
                    "platform": "mqtt",
                });

                // Tell hass about this shade
                reg.config(
                    format!("{}/cover/{shade_id}/config", self.discovery_prefix),
                    serde_json::to_string(&data)?,
                )
                .await?;

                reg.update(format!("pv2mqtt/shade/{shade_id}/availability"), "online");

                reg.update(
                    format!("pv2mqtt/shade/{shade_id}/position"),
                    format!("{pos}"),
                );
                let state = if pos == 0 { "closed" } else { "open" };
                reg.update(format!("pv2mqtt/shade/{shade_id}/state"), state);
            }
        }

        Ok(())
    }

    async fn advise_of_state_label(
        &self,
        client: &mut Client,
        shade_id: &str,
        state: &str,
    ) -> anyhow::Result<()> {
        client
            .publish(
                &format!("pv2mqtt/shade/{shade_id}/state"),
                &state.as_bytes(),
                QoS::AtMostOnce,
                false,
            )
            .await?;
        Ok(())
    }

    async fn handle_position(
        &self,
        client: &mut Client,
        shade_id: &str,
        position: u8,
    ) -> anyhow::Result<()> {
        client
            .publish(
                &format!("pv2mqtt/shade/{shade_id}/position"),
                &format!("{position}").as_bytes(),
                QoS::AtMostOnce,
                false,
            )
            .await?;

        Ok(())
    }

    async fn setup_http_server(&self, hub: &Hub, tx: Sender<ServerEvent>) -> anyhow::Result<()> {
        // Figure out our local ip when talking to the hub
        let hub_bind_addr = hub.suggest_bind_address().await?;

        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::{IntoResponse, Response};
        use axum::routing::post;
        use axum::Router;
        use base64::engine::Engine;

        fn generic<T: ToString + std::fmt::Display>(err: T) -> Response {
            log::error!("err: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
        }

        /// The hook data is sent with `Content-Type: application/x-www-form-urlencoded`
        /// but the data is most often actually base64 encoded json, so we just have
        /// to ignore the content type and extract from the data ourselves.
        async fn pv_postback(
            State(tx): State<Sender<ServerEvent>>,
            body: String,
        ) -> Result<Response, Response> {
            #[derive(Deserialize, Debug)]
            #[serde(rename_all = "camelCase")]
            #[serde(deny_unknown_fields)]
            pub struct ConfigUpdate {
                pub config_num: i64,
            }

            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&body) {
                let data: Vec<HomeAutomationPostBackData> =
                    serde_json::from_slice(&decoded).map_err(generic)?;
                tx.send(ServerEvent::HomeAutomationData(data))
                    .await
                    .map_err(generic)?;
            } else if let Ok(config) = serde_urlencoded::from_str::<ConfigUpdate>(&body) {
                log::debug!(
                    "** A shade failed post-move verification. New configuration {config:?}"
                );
            } else {
                log::error!("** Not sure what to do with {body}");
            }
            Ok((StatusCode::OK, "").into_response())
        }

        let app = Router::new()
            .route("/pv-postback", post(pv_postback))
            .with_state(tx);

        let listener = tokio::net::TcpListener::bind((hub_bind_addr, 0)).await?;
        let addr = listener.local_addr()?;
        log::info!("http server addr is {addr:?}");
        hub.enable_home_automation_hook(&format!("{addr}/pv-postback"))
            .await?;
        tokio::spawn(async { axum::serve(listener, app).await });
        Ok(())
    }

    async fn register_with_hass(&self, hub: &Hub, client: &mut Client) -> anyhow::Result<()> {
        let user_data = hub.get_user_data().await?;
        let mut reg = HassRegistration {
            client,
            updates: vec![],
        };

        self.register_hub(&user_data, &mut reg).await?;
        self.register_shades(&user_data, &hub, &mut reg).await?;
        self.register_scenes(&user_data, &hub, &mut reg).await?;
        // Give home assistant some time to process the configuration
        // messages before attempting to notify of availability and
        // other updates
        tokio::time::sleep(Duration::from_millis(500)).await;
        reg.apply_updates().await?;
        Ok(())
    }

    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        let hub = args.hub().await?;

        self.setup_http_server(&hub, tx.clone()).await?;

        let mut client = Client::with_auto_id()?;

        client.set_username_and_password(self.username.as_deref(), self.password.as_deref())?;
        client
            .connect(
                &self.host,
                self.port.into(),
                Duration::from_secs(10),
                self.bind_address.as_deref(),
            )
            .await?;
        let subscriber = client.subscriber().expect("to own the subscriber");

        client
            .subscribe(
                &format!("{}/status", self.discovery_prefix),
                QoS::AtMostOnce,
            )
            .await?;
        client
            .subscribe("pv2mqtt/shade/+/state", QoS::AtMostOnce)
            .await?;
        client
            .subscribe("pv2mqtt/shade/+/position", QoS::AtMostOnce)
            .await?;
        client
            .subscribe("pv2mqtt/shade/+/set_position", QoS::AtMostOnce)
            .await?;
        client
            .subscribe("pv2mqtt/shade/+/availability", QoS::AtMostOnce)
            .await?;
        client
            .subscribe("pv2mqtt/shade/+/command", QoS::AtMostOnce)
            .await?;
        client
            .subscribe("pv2mqtt/scene/+/set", QoS::AtMostOnce)
            .await?;

        self.register_with_hass(&hub, &mut client).await?;

        tokio::spawn(async move {
            while let Ok(msg) = subscriber.recv().await {
                if let Err(err) = tx.send(ServerEvent::MqttMessage(msg)).await {
                    log::error!("{err:#?}");
                    break;
                }
            }
        });

        self.serve(hub, client, rx).await
    }

    async fn handle_mqtt_message(
        &self,
        msg: Message,
        hub: &Hub,
        client: &mut Client,
    ) -> anyhow::Result<()> {
        log::debug!("msg: {msg:?}");

        if msg.topic == format!("{}/status", self.discovery_prefix) {
            return self.register_with_hass(hub, client).await;
        }

        let topic: Vec<_> = msg.topic.split('/').collect();
        if let [_, device_kind, target_id, kind] = topic.as_slice() {
            match *device_kind {
                "shade" => {
                    let shade_id = target_id;
                    let (actual_shade_id, is_secondary) =
                        if let Some(id) = shade_id.strip_suffix(SECONDARY_SUFFIX) {
                            (id.parse::<i32>()?, true)
                        } else {
                            (shade_id.parse::<i32>()?, false)
                        };

                    let shade = hub.shade_by_id(actual_shade_id).await?;

                    let payload = String::from_utf8_lossy(&msg.payload);
                    log::debug!("{kind} Cover: {shade_id} 2nd={is_secondary}, payload={payload}");
                    match kind.as_ref() {
                        "set_position" => {
                            let position: u8 = payload.parse()?;

                            let mut shade_pos = shade.positions.clone().ok_or_else(|| {
                                anyhow::anyhow!("shade {actual_shade_id} has no existing position")
                            })?;

                            let absolute = ShadePosition::percent_to_pos(position);

                            if is_secondary {
                                shade_pos.position_2.replace(absolute);
                            } else {
                                shade_pos.position_1 = absolute;
                            }

                            hub.change_shade_position(actual_shade_id, shade_pos.clone())
                                .await?;
                        }
                        "command" => match payload.as_ref() {
                            "OPEN" => {
                                hub.move_shade(actual_shade_id, ShadeUpdateMotion::Up)
                                    .await?;
                            }
                            "CLOSE" => {
                                hub.move_shade(actual_shade_id, ShadeUpdateMotion::Down)
                                    .await?;
                            }
                            "STOP" => {
                                hub.move_shade(actual_shade_id, ShadeUpdateMotion::Stop)
                                    .await?;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                "scene" => {
                    let scene_id = target_id.parse()?;
                    hub.activate_scene(scene_id).await?;
                }
                _ => {
                    log::error!("device_kind {device_kind} not handled");
                }
            }
        } else {
            log::error!("topic {} not handled", msg.topic);
        }
        Ok(())
    }

    async fn handle_pv_event(
        &self,
        item: HomeAutomationPostBackData,
        client: &mut Client,
    ) -> anyhow::Result<()> {
        log::debug!("item: {item:#?}");

        let shade_id = match item.service {
            HomeAutomationService::Primary => item.shade_id.to_string(),
            HomeAutomationService::Secondary => {
                format!("{}{SECONDARY_SUFFIX}", item.shade_id)
            }
        };

        match item.record_type {
            HomeAutomationRecordType::Stops => {
                if let Some(pct) = item.stopped_position {
                    self.handle_position(client, &shade_id, pct).await?;

                    let state = if pct == 0 { "closed" } else { "open" };
                    self.advise_of_state_label(client, &shade_id, state).await?;
                }
            }
            HomeAutomationRecordType::BeginsMoving => {
                if let Some(pct) = item.current_position {
                    self.handle_position(client, &shade_id, pct).await?;
                }
            }
            HomeAutomationRecordType::StartsClosing => {
                self.advise_of_state_label(client, &shade_id, "closing")
                    .await?;
            }
            HomeAutomationRecordType::StartsOpening => {
                self.advise_of_state_label(client, &shade_id, "opening")
                    .await?;
            }
            HomeAutomationRecordType::HasOpened | HomeAutomationRecordType::HasFullyOpened => {
                self.advise_of_state_label(client, &shade_id, "open")
                    .await?;
            }
            HomeAutomationRecordType::HasClosed | HomeAutomationRecordType::HasFullyClosed => {
                self.advise_of_state_label(client, &shade_id, "closed")
                    .await?;
            }
            HomeAutomationRecordType::TargetLevelChanged => {}
            HomeAutomationRecordType::LevelChanged => {}
        }
        Ok(())
    }

    async fn serve(
        &self,
        hub: Hub,
        mut client: Client,
        mut rx: Receiver<ServerEvent>,
    ) -> anyhow::Result<()> {
        log::info!("Waiting for mqtt and pv messages");
        while let Some(msg) = rx.recv().await {
            match msg {
                ServerEvent::MqttMessage(msg) => {
                    if let Err(err) = self.handle_mqtt_message(msg, &hub, &mut client).await {
                        log::error!("handling mqtt message: {err:#}");
                    }
                }
                ServerEvent::HomeAutomationData(mut data) => {
                    // Re-order the events so that the closed/open events happen
                    // after closing/opening
                    data.sort_by(|a, b| a.record_type.cmp(&b.record_type));

                    for item in data {
                        if let Err(err) = self.handle_pv_event(item, &mut client).await {
                            log::error!("handling pv event: {err:#}");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
