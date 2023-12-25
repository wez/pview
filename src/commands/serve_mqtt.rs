use crate::api_types::{
    HomeAutomationPostBackData, HomeAutomationRecordType, HomeAutomationService,
    ShadeCapabilityFlags, ShadePosition, ShadeUpdateMotion, UserData,
};
use crate::discovery::ResolvedHub;
use crate::hub::Hub;
use crate::opt_env_var;
use anyhow::Context;
use mosquitto_rs::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};

const SECONDARY_SUFFIX: &str = "_middle";
const MODEL: &str = "pv2mqtt";

// <https://www.home-assistant.io/integrations/cover.mqtt/>

/// Launch the pv2mqtt bridge, adding your hub to Home Assistant
#[derive(clap::Parser, Debug)]
pub struct ServeMqttCommand {
    /// The mqtt broker hostname or address.
    /// You may also set this via the PV_MQTT_HOST environment variable.
    #[arg(long)]
    host: Option<String>,

    /// The mqtt broker port
    /// You may also set this via the PV_MQTT_PORT environment variable.
    /// If unspecified, uses 1883
    #[arg(long)]
    port: Option<u16>,

    /// The username to authenticate against the broker
    /// You may also set this via the PV_MQTT_USER environment variable.
    #[arg(long)]
    username: Option<String>,
    /// The password to authenticate against the broker
    /// You may also set this via the PV_MQTT_PASSWORD environment variable.
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
    PeriodicStateUpdate,
    HubDiscovered(ResolvedHub),
}

#[derive(Debug)]
enum RegEntry {
    Delay,
    Msg { topic: String, payload: String },
}

impl RegEntry {
    pub fn msg<T: Into<String>, P: Into<String>>(topic: T, payload: P) -> Self {
        Self::Msg {
            topic: topic.into(),
            payload: payload.into(),
        }
    }
}

struct HassRegistration {
    configs: Vec<RegEntry>,
    updates: Vec<RegEntry>,
}

impl HassRegistration {
    pub fn new() -> Self {
        Self {
            configs: vec![],
            updates: vec![
                // Delay between registering configs and advising hass
                // of the states, so that hass has had enough time
                // to subscribe to the correct topics
                RegEntry::Delay,
            ],
        }
    }

    pub fn config<T: Into<String>, P: Into<String>>(&mut self, topic: T, payload: P) {
        self.configs.push(RegEntry::msg(topic, payload));
    }

    pub fn update<T: Into<String>, P: Into<String>>(&mut self, topic: T, payload: P) {
        self.updates.push(RegEntry::msg(topic, payload));
    }

    pub async fn apply_updates(self, client: &mut Client) -> anyhow::Result<()> {
        for queue in [self.configs, self.updates] {
            for entry in queue {
                match entry {
                    RegEntry::Delay => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    RegEntry::Msg { topic, payload } => {
                        client
                            .publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false)
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl ServeMqttCommand {
    async fn register_hub(
        &self,
        user_data: &UserData,
        reg: &mut HassRegistration,
    ) -> anyhow::Result<()> {
        let serial = &user_data.serial_number;
        let data = serde_json::json!({
            "name": "IP Address",
            "unique_id": format!("{serial}-hub-ip"),
            "state_topic": format!("{MODEL}/sensor/{serial}-hub-ip/state"),
            "availability_topic": format!("{MODEL}/sensor/{serial}-hub-ip/availability"),
            "device": {
                "identifiers": [
                    format!("{MODEL}-{serial}"),
                    user_data.serial_number,
                    user_data.mac_address,
                ],
                "connections": [
                    ["mac", user_data.mac_address],
                ],
                "name": format!("{} PowerView Hub: {}", user_data.brand, user_data.hub_name.to_string()),
                "manufacturer": "Wez Furlong",
                "model": MODEL,
            },
            "entity_category": "diagnostic",
            "origin": {
                "name": MODEL,
                "sw": "0.0",
                "url": "https://github.com/wez/pview",
            },
        });

        reg.config(
            format!("{}/sensor/{serial}-hub-ip/config", self.discovery_prefix),
            serde_json::to_string(&data)?,
        );

        reg.update(
            format!("{MODEL}/sensor/{serial}-hub-ip/availability"),
            "online",
        );

        reg.update(
            format!("{MODEL}/sensor/{serial}-hub-ip/state"),
            user_data.ip.clone(),
        );

        Ok(())
    }

    async fn register_scenes(
        &self,
        user_data: &UserData,
        hub: &Hub,
        reg: &mut HassRegistration,
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

            let unique_id = format!("{}-scene-{scene_id}", user_data.serial_number);

            let data = serde_json::json!({
                "name": serde_json::Value::Null,
                "unique_id": unique_id,
                "availability_topic": format!("{MODEL}/scene/{scene_id}/availability"),
                "command_topic": format!("{MODEL}/scene/{scene_id}/set"),
                "payload_on": "ON",
                "device": {
                    "suggested_area": area,
                    "identifiers": [
                        unique_id,
                    ],
                    "via_device": format!("{MODEL}-{}", user_data.serial_number),
                    "name": scene_name,
                    "manufacturer": "Wez Furlong",
                    "model": MODEL,
                },
            });

            // Tell hass about this shade
            reg.config(
                format!("{}/scene/{unique_id}/config", self.discovery_prefix),
                serde_json::to_string(&data)?,
            );

            reg.update(format!("{MODEL}/scene/{scene_id}/availability"), "online");
        }

        Ok(())
    }

    async fn register_shades(
        &self,
        user_data: &UserData,
        hub: &Hub,
        reg: &mut HassRegistration,
    ) -> anyhow::Result<()> {
        let shades = hub.list_shades(None, None).await?;
        let room_by_id: HashMap<_, _> = hub
            .list_rooms()
            .await?
            .into_iter()
            .map(|room| (room.id, room.name))
            .collect();

        for shade in &shades {
            let position = match shade.positions.clone() {
                Some(p) => p,
                None => continue,
            };

            let mut shades = vec![(
                shade.id.to_string(),
                serde_json::Value::Null,
                Some(position.pos1_percent()),
            )];

            // The shade data doesn't always include the second rail
            // position, so we must use the capabilities to decide if
            // it should actually be there
            if shade
                .capabilities
                .flags()
                .contains(ShadeCapabilityFlags::SECONDARY_RAIL)
            {
                shades.push((
                    format!("{}{SECONDARY_SUFFIX}", shade.id),
                    serde_json::json!("Middle Rail"),
                    position.pos2_percent(),
                ));
            }

            let device_id = format!("{}-{}", user_data.serial_number, shade.id);

            for (shade_id, shade_name, pos) in shades {
                let area = shade
                    .room_id
                    .and_then(|room_id| {
                        room_by_id
                            .get(&room_id)
                            .map(|name| serde_json::json!(name.as_str()))
                    })
                    .unwrap_or(serde_json::Value::Null);
                let unique_id = format!("{}-{shade_id}", user_data.serial_number);

                let data = serde_json::json!({
                    "name": shade_name ,
                    "device_class": "shade",
                    "unique_id": unique_id,
                    "state_topic": format!("{MODEL}/shade/{shade_id}/state"),
                    "position_topic": format!("{MODEL}/shade/{shade_id}/position"),
                    "availability_topic": format!("{MODEL}/shade/{shade_id}/availability"),
                    "set_position_topic": format!("{MODEL}/shade/{shade_id}/set_position"),
                    "command_topic": format!("{MODEL}/shade/{shade_id}/command"),
                    "device": {
                        "suggested_area": area,
                        "identifiers": [
                            device_id
                        ],
                        "via_device": format!("{MODEL}-{}", user_data.serial_number),
                        "name": shade.name(),
                        "manufacturer": "Hunter Douglas",
                        "model": MODEL,
                        "sw_version": shade.firmware.as_ref().map(|vers| {
                            format!("{}.{}.{}", vers.revision, vers.sub_revision, vers.build)
                        }).unwrap_or_else(|| "unknown".to_string()),
                    },
                    "origin": {
                        "name": MODEL,
                        "sw": "0.0",
                        "url": "https://github.com/wez/pview",
                    },
                });

                // Tell hass about this shade
                reg.config(
                    format!("{}/cover/{shade_id}/config", self.discovery_prefix),
                    serde_json::to_string(&data)?,
                );

                reg.update(format!("{MODEL}/shade/{shade_id}/availability"), "online");

                // We may not know the position; this can happen when the shade is
                // partially out of sync, for example, for a top-down-bottom-up
                // shade, I've seen the primary position reported, but the secondary
                // is blank
                if let Some(pos) = pos {
                    reg.update(
                        format!("{MODEL}/shade/{shade_id}/position"),
                        format!("{pos}"),
                    );
                    let state = if pos == 0 { "closed" } else { "open" };
                    reg.update(format!("{MODEL}/shade/{shade_id}/state"), state);
                }
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
                &format!("{MODEL}/shade/{shade_id}/state"),
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
                &format!("{MODEL}/shade/{shade_id}/position"),
                &format!("{position}").as_bytes(),
                QoS::AtMostOnce,
                false,
            )
            .await?;

        Ok(())
    }

    async fn setup_http_server(&self, tx: Sender<ServerEvent>) -> anyhow::Result<u16> {
        // Figure out our local ip when talking to the hub
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
                log::debug!("postback: {data:?}");
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

        let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0)).await?;
        let addr = listener.local_addr()?;
        log::info!("http server addr is {addr:?}");
        tokio::spawn(async {
            if let Err(err) = axum::serve(listener, app).await {
                log::error!("http server stopped: {err:#}");
            }
        });
        Ok(addr.port())
    }

    async fn register_with_hass(&self, hub: &Hub, client: &mut Client) -> anyhow::Result<()> {
        let user_data = hub.get_user_data().await?;
        let mut reg = HassRegistration::new();

        self.register_hub(&user_data, &mut reg).await?;
        self.register_shades(&user_data, &hub, &mut reg).await?;
        self.register_scenes(&user_data, &hub, &mut reg).await?;
        reg.apply_updates(client).await?;
        Ok(())
    }

    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let mqtt_host = match &self.host {
            Some(h) => h.to_string(),
            None => std::env::var("PV_MQTT_HOST").context(
                "specify the mqtt host either via the --host \
                 option or the PV_MQTT_HOST environment variable",
            )?,
        };

        let mqtt_port: u16 = match self.port {
            Some(p) => p,
            None => opt_env_var("PV_MQTT_PORT")?.unwrap_or(1883),
        };

        let mqtt_username: Option<String> = match self.username.clone() {
            Some(u) => Some(u),
            None => opt_env_var("PV_MQTT_USER")?,
        };
        let mqtt_password: Option<String> = match self.password.clone() {
            Some(u) => Some(u),
            None => opt_env_var("PV_MQTT_PASSWORD")?,
        };

        let (tx, rx) = tokio::sync::mpsc::channel(32);

        let hub = args.hub().await?;
        let hub = ResolvedHub::with_hub(hub).await;

        let http_port = self.setup_http_server(tx.clone()).await?;
        self.update_homeautomation_hook(&hub, http_port).await?;

        let mut client = Client::with_auto_id()?;

        client.set_username_and_password(mqtt_username.as_deref(), mqtt_password.as_deref())?;
        client
            .connect(
                &mqtt_host,
                mqtt_port.into(),
                Duration::from_secs(10),
                self.bind_address.as_deref(),
            )
            .await
            .with_context(|| format!("connecting to mqtt broker {mqtt_host}:{mqtt_port}"))?;
        let subscriber = client.subscriber().expect("to own the subscriber");

        client
            .subscribe(
                &format!("{}/status", self.discovery_prefix),
                QoS::AtMostOnce,
            )
            .await?;
        client
            .subscribe(&format!("{MODEL}/shade/+/state"), QoS::AtMostOnce)
            .await?;
        client
            .subscribe(&format!("{MODEL}/shade/+/position"), QoS::AtMostOnce)
            .await?;
        client
            .subscribe(&format!("{MODEL}/shade/+/set_position"), QoS::AtMostOnce)
            .await?;
        client
            .subscribe(&format!("{MODEL}/shade/+/availability"), QoS::AtMostOnce)
            .await?;
        client
            .subscribe(&format!("{MODEL}/shade/+/command"), QoS::AtMostOnce)
            .await?;
        client
            .subscribe(&format!("{MODEL}/scene/+/set"), QoS::AtMostOnce)
            .await?;

        self.register_with_hass(&hub, &mut client).await?;

        {
            let tx = tx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    if let Err(err) = tx.send(ServerEvent::PeriodicStateUpdate).await {
                        log::error!("{err:#?}");
                        break;
                    }
                }
            });
        }

        if !args.hub_ip_was_specified_by_user() {
            let tx = tx.clone();
            let serial = args.hub_serial()?;
            let mut disco = crate::discovery::resolve_hubs(None).await?;
            tokio::spawn(async move {
                while let Some(resolved_hub) = disco.recv().await {
                    log::trace!("disco resolved: {resolved_hub:?}");
                    if let Some(user_data) = &resolved_hub.user_data {
                        if let Some(serial) = &serial {
                            if *serial != user_data.serial_number {
                                continue;
                            }
                        }

                        if let Err(err) = tx.send(ServerEvent::HubDiscovered(resolved_hub)).await {
                            log::error!("discovery: send to main thread: {err:#}");
                            break;
                        }
                    }
                }
                log::warn!("fell out of disco loop");
            });
        }

        tokio::spawn(async move {
            while let Ok(msg) = subscriber.recv().await {
                if let Err(err) = tx.send(ServerEvent::MqttMessage(msg)).await {
                    log::error!("{err:#?}");
                    break;
                }
            }
        });

        self.serve(hub, client, rx, http_port).await;
        Ok(())
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

                            log::info!(
                                "Set {shade_id} {} position to {position} ({shade_pos:?})",
                                shade.name()
                            );
                            hub.change_shade_position(actual_shade_id, shade_pos.clone())
                                .await?;
                        }
                        "command" => {
                            log::info!("OPEN {shade_id} {}", shade.name());
                            match payload.as_ref() {
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
                            }
                        }
                        _ => {}
                    }
                }
                "scene" => {
                    let scene_id = target_id.parse()?;
                    log::info!("SCENE {scene_id}");
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

    async fn update_homeautomation_hook(&self, hub: &Hub, http_port: u16) -> anyhow::Result<()> {
        let addr = hub.suggest_bind_address().await?;
        hub.enable_home_automation_hook(&format!("{addr}:{http_port}/pv-postback"))
            .await?;
        Ok(())
    }

    async fn handle_discovery(
        &self,
        new_hub: ResolvedHub,
        hub: &mut ResolvedHub,
        client: &mut Client,
        http_port: u16,
    ) -> anyhow::Result<()> {
        let changed = match (&new_hub.user_data, &hub.user_data) {
            (Some(n), Some(e)) => n.ip != e.ip || n.hub_name != e.hub_name,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if !changed {
            return Ok(());
        }
        log::info!("Hub ip and/or name changed");

        *hub = new_hub;
        self.update_homeautomation_hook(hub, http_port).await?;
        self.register_with_hass(hub, client).await?;
        Ok(())
    }

    async fn serve(
        &self,
        mut hub: ResolvedHub,
        mut client: Client,
        mut rx: Receiver<ServerEvent>,
        http_port: u16,
    ) {
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

                ServerEvent::HubDiscovered(resolved_hub) => {
                    if let Err(err) = self
                        .handle_discovery(resolved_hub, &mut hub, &mut client, http_port)
                        .await
                    {
                        log::error!("While updating hass state: {err:#?}");
                    }
                }

                ServerEvent::PeriodicStateUpdate => {
                    if let Err(err) = self.register_with_hass(&mut hub, &mut client).await {
                        log::error!("While updating hass state: {err:#?}");
                    }
                }
            }
        }
    }
}
