use crate::api_types::{
    HomeAutomationPostBackData, HomeAutomationRecordType, HomeAutomationService,
    ShadeCapabilityFlags, ShadePosition, ShadeUpdateMotion, UserData,
};
use crate::discovery::ResolvedHub;
use crate::hub::Hub;
use crate::mqtt_helper::{parse_deser, MqttRouter};
use crate::opt_env_var;
use anyhow::Context;
use arc_swap::ArcSwap;
use axum::extract::Path;
use mosquitto_rs::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    HomeAutomationData {
        serial: String,
        data: Vec<HomeAutomationPostBackData>,
    },
    PeriodicStateUpdate,
    HubDiscovered(ResolvedHub),
}

#[derive(Debug)]
enum RegEntry {
    Delay(Duration),
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
    deletes: Vec<RegEntry>,
    configs: Vec<RegEntry>,
    updates: Vec<RegEntry>,
}

impl HassRegistration {
    pub fn new() -> Self {
        Self {
            deletes: vec![],
            configs: vec![],
            updates: vec![
                // Delay between registering configs and advising hass
                // of the states, so that hass has had enough time
                // to subscribe to the correct topics
                RegEntry::Delay(Duration::from_millis(500)),
            ],
        }
    }

    pub fn delete<T: Into<String>>(&mut self, topic: T) {
        if self.deletes.is_empty() {
            self.deletes.push(RegEntry::Delay(Duration::from_secs(4)));
        }
        self.deletes.push(RegEntry::msg(topic, ""));
    }

    pub fn config<T: Into<String>, P: Into<String>>(&mut self, topic: T, payload: P) {
        self.configs.push(RegEntry::msg(topic, payload));
    }

    pub fn update<T: Into<String>, P: Into<String>>(&mut self, topic: T, payload: P) {
        self.updates.push(RegEntry::msg(topic, payload));
    }

    pub async fn apply_updates(mut self, state: &Arc<Pv2MqttState>) -> anyhow::Result<()> {
        if !state.first_run.load(Ordering::SeqCst) {
            self.deletes.clear();
        }
        for queue in [self.deletes, self.configs, self.updates] {
            for entry in queue {
                match entry {
                    RegEntry::Delay(duration) => {
                        tokio::time::sleep(duration).await;
                    }
                    RegEntry::Msg { topic, payload } => {
                        state
                            .client
                            .publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false)
                            .await?;
                    }
                }
            }
        }
        state.first_run.store(false, Ordering::SeqCst);
        Ok(())
    }
}

struct DiagnosticEntity {
    name: String,
    unique_id: String,
    value: String,
}

async fn register_diagnostic_entity(
    diagnostic: DiagnosticEntity,
    user_data: &UserData,
    state: &Arc<Pv2MqttState>,
    reg: &mut HassRegistration,
) -> anyhow::Result<()> {
    let serial = &user_data.serial_number;
    let unique_id = &diagnostic.unique_id;

    let data = serde_json::json!({
        "name": diagnostic.name,
        "unique_id": unique_id,
        "state_topic": format!("{MODEL}/sensor/{unique_id}/state"),
        "availability_topic": format!("{MODEL}/sensor/{unique_id}/availability"),
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
        format!("{}/sensor/{unique_id}/config", state.discovery_prefix),
        serde_json::to_string(&data)?,
    );

    reg.update(format!("{MODEL}/sensor/{unique_id}/availability"), "online");

    reg.update(
        format!("{MODEL}/sensor/{unique_id}/state"),
        diagnostic.value,
    );

    Ok(())
}

async fn register_hub(
    user_data: &UserData,
    state: &Arc<Pv2MqttState>,
    reg: &mut HassRegistration,
) -> anyhow::Result<()> {
    let serial = &user_data.serial_number;
    register_diagnostic_entity(
        DiagnosticEntity {
            name: "IP Address".to_string(),
            unique_id: format!("{serial}-hub-ip"),
            value: user_data.ip.clone(),
        },
        user_data,
        state,
        reg,
    )
    .await?;

    register_diagnostic_entity(
        DiagnosticEntity {
            name: "Status".to_string(),
            unique_id: format!("{serial}-responding"),
            value: if state.responding.load(Ordering::SeqCst) {
                "OK"
            } else {
                "UNRESPONSIVE"
            }
            .to_string(),
        },
        user_data,
        state,
        reg,
    )
    .await?;

    register_diagnostic_entity(
        DiagnosticEntity {
            name: "rfStatus".to_string(),
            unique_id: format!("{serial}-rfStatus"),
            value: user_data.rf_status.to_string(),
        },
        user_data,
        state,
        reg,
    )
    .await?;

    Ok(())
}

async fn register_shades(
    state: &Arc<Pv2MqttState>,
    reg: &mut HassRegistration,
) -> anyhow::Result<()> {
    let hub = state.hub.load();
    let shades = hub.hub.list_shades(None, None).await?;
    let room_by_id: HashMap<_, _> = hub
        .hub
        .list_rooms()
        .await?
        .into_iter()
        .map(|room| (room.id, room.name))
        .collect();

    let serial = &state.serial;

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

        let device_id = format!("{serial}-{}", shade.id);

        for (shade_id, shade_name, pos) in shades {
            let area = shade
                .room_id
                .and_then(|room_id| {
                    room_by_id
                        .get(&room_id)
                        .map(|name| serde_json::json!(name.as_str()))
                })
                .unwrap_or(serde_json::Value::Null);
            let unique_id = format!("{serial}-{shade_id}");

            let data = serde_json::json!({
                "name": shade_name ,
                "device_class": "shade",
                "unique_id": unique_id,
                "state_topic": format!("{MODEL}/shade/{serial}/{shade_id}/state"),
                "position_topic": format!("{MODEL}/shade/{serial}/{shade_id}/position"),
                "availability_topic": format!("{MODEL}/shade/{serial}/{shade_id}/availability"),
                "set_position_topic": format!("{MODEL}/shade/{serial}/{shade_id}/set_position"),
                "command_topic": format!("{MODEL}/shade/{serial}/{shade_id}/command"),
                "device": {
                    "suggested_area": area,
                    "identifiers": [
                        device_id
                    ],
                    "via_device": format!("{MODEL}-{serial}"),
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

            // Delete legacy version of this shade, for those upgrading.
            // TODO: remove this, or find some way to keep track of what
            // version of things are already present in hass
            reg.delete(format!(
                "{}/cover/{shade_id}/config",
                state.discovery_prefix
            ));

            // Tell hass about this shade
            reg.config(
                format!(
                    "{}/cover/{serial}-{shade_id}/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&data)?,
            );

            reg.update(
                format!("{MODEL}/shade/{serial}/{shade_id}/availability"),
                "online",
            );

            // We may not know the position; this can happen when the shade is
            // partially out of sync, for example, for a top-down-bottom-up
            // shade, I've seen the primary position reported, but the secondary
            // is blank
            if let Some(pos) = pos {
                reg.update(
                    format!("{MODEL}/shade/{serial}/{shade_id}/position"),
                    format!("{pos}"),
                );
                let state = if pos == 0 { "closed" } else { "open" };
                reg.update(format!("{MODEL}/shade/{serial}/{shade_id}/state"), state);
            }
        }
    }

    Ok(())
}

async fn register_scenes(
    state: &Arc<Pv2MqttState>,
    reg: &mut HassRegistration,
) -> anyhow::Result<()> {
    let hub = state.hub.load();
    let scenes = hub.hub.list_scenes().await?;
    let room_by_id: HashMap<_, _> = hub
        .hub
        .list_rooms()
        .await?
        .into_iter()
        .map(|room| (room.id, room.name))
        .collect();

    let serial = &state.serial;

    for scene in scenes {
        let scene_id = scene.id;
        let scene_name = scene.name.to_string();

        let area = room_by_id
            .get(&scene.room_id)
            .map(|name| serde_json::json!(name.as_str()))
            .unwrap_or(serde_json::Value::Null);

        let unique_id = format!("{serial}-scene-{scene_id}");

        let data = serde_json::json!({
            "name": serde_json::Value::Null,
            "unique_id": unique_id,
            "availability_topic": format!("{MODEL}/scene/{serial}/{scene_id}/availability"),
            "command_topic": format!("{MODEL}/scene/{serial}/{scene_id}/set"),
            "payload_on": "ON",
            "device": {
                "suggested_area": area,
                "identifiers": [
                    unique_id,
                ],
                "via_device": format!("{MODEL}-{serial}"),
                "name": scene_name,
                "manufacturer": "Wez Furlong",
                "model": MODEL,
            },
        });

        // Delete legacy scene
        reg.delete(format!(
            "{}/scene/{unique_id}/config",
            state.discovery_prefix
        ));

        // Tell hass about this scene
        reg.config(
            format!("{}/scene/{unique_id}/config", state.discovery_prefix),
            serde_json::to_string(&data)?,
        );

        reg.update(
            format!("{MODEL}/scene/{serial}/{scene_id}/availability"),
            "online",
        );
    }

    Ok(())
}

async fn register_with_hass(state: &Arc<Pv2MqttState>) -> anyhow::Result<()> {
    let mut reg = HassRegistration::new();

    register_hub(&state.hub.load().user_data, state, &mut reg).await?;
    register_shades(state, &mut reg).await?;
    register_scenes(state, &mut reg).await?;
    reg.apply_updates(state).await?;
    Ok(())
}

async fn advise_hass_of_unresponsive(state: &Arc<Pv2MqttState>) -> anyhow::Result<()> {
    state
        .client
        .publish(
            format!("{MODEL}/shade/{}-responding/state", state.serial),
            "UNRESPONSIVE",
            QoS::AtMostOnce,
            false,
        )
        .await?;
    Ok(())
}

async fn advise_hass_of_state_label(
    state: &Arc<Pv2MqttState>,
    shade_id: &str,
    shade_state: &str,
) -> anyhow::Result<()> {
    state
        .client
        .publish(
            &format!(
                "{MODEL}/shade/{serial}/{shade_id}/state",
                serial = state.serial
            ),
            &shade_state.as_bytes(),
            QoS::AtMostOnce,
            false,
        )
        .await?;
    Ok(())
}

async fn advise_hass_of_position(
    state: &Arc<Pv2MqttState>,
    shade_id: &str,
    position: u8,
) -> anyhow::Result<()> {
    state
        .client
        .publish(
            &format!(
                "{MODEL}/shade/{serial}/{shade_id}/position",
                serial = state.serial
            ),
            &format!("{position}").as_bytes(),
            QoS::AtMostOnce,
            false,
        )
        .await?;

    Ok(())
}

impl ServeMqttCommand {
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
            Path(serial): Path<String>,
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
                tx.send(ServerEvent::HomeAutomationData { serial, data })
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
            .route("/pv-postback/:serial", post(pv_postback))
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
        let mut hub = ResolvedHub::with_hub(hub).await;
        let user_data = hub.user_data.take().ok_or_else(|| {
            anyhow::anyhow!(
                "Unable to determine the serial number \
                    of the hub. The hub is not be responding correctly \
                    and may need to be restarted"
            )
        })?;
        let serial = &user_data.serial_number.to_string();

        let http_port = self.setup_http_server(tx.clone()).await?;

        let client = Client::with_auto_id()?;

        let state = Arc::new(Pv2MqttState {
            hub: ArcSwap::new(Arc::new(FullyResolvedHub {
                hub: hub.hub.clone(),
                user_data,
            })),
            client: client.clone(),
            serial: serial.clone(),
            http_port,
            discovery_prefix: self.discovery_prefix.clone(),
            first_run: AtomicBool::new(true),
            responding: AtomicBool::new(true),
        });

        self.update_homeautomation_hook(&state).await?;

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

        let mut router: MqttRouter<Arc<Pv2MqttState>> = MqttRouter::new(client.clone());

        router
            .route(
                format!("{}/status", self.discovery_prefix),
                mqtt_homeassitant_status,
            )
            .await?;

        router
            .route(
                format!("{MODEL}/scene/:serial/:scene_id/set"),
                mqtt_scene_activate,
            )
            .await?;

        router
            .route(
                format!("{MODEL}/shade/:serial/:shade_id/set_position"),
                mqtt_shade_set_position,
            )
            .await?;
        router
            .route(
                format!("{MODEL}/shade/:serial/:shade_id/command"),
                mqtt_shade_command,
            )
            .await?;

        register_with_hass(&state).await?;

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

        self.serve(rx, state, router).await;
        Ok(())
    }

    async fn handle_mqtt_message(
        &self,
        msg: Message,
        state: &Arc<Pv2MqttState>,
        router: &MqttRouter<Arc<Pv2MqttState>>,
    ) -> anyhow::Result<()> {
        log::debug!("msg: {msg:?}");
        router.dispatch(msg, Arc::clone(state)).await
    }

    async fn handle_pv_event(
        &self,
        state: &Arc<Pv2MqttState>,
        item: HomeAutomationPostBackData,
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
                    advise_hass_of_position(state, &shade_id, pct).await?;

                    let shade_state = if pct == 0 { "closed" } else { "open" };
                    advise_hass_of_state_label(state, &shade_id, shade_state).await?;
                }
            }
            HomeAutomationRecordType::BeginsMoving => {
                if let Some(pct) = item.current_position {
                    advise_hass_of_position(state, &shade_id, pct).await?;
                }
            }
            HomeAutomationRecordType::StartsClosing => {
                advise_hass_of_state_label(state, &shade_id, "closing").await?;
            }
            HomeAutomationRecordType::StartsOpening => {
                advise_hass_of_state_label(state, &shade_id, "opening").await?;
            }
            HomeAutomationRecordType::HasOpened | HomeAutomationRecordType::HasFullyOpened => {
                advise_hass_of_state_label(state, &shade_id, "open").await?;
            }
            HomeAutomationRecordType::HasClosed | HomeAutomationRecordType::HasFullyClosed => {
                advise_hass_of_state_label(state, &shade_id, "closed").await?;
            }
            HomeAutomationRecordType::TargetLevelChanged => {}
            HomeAutomationRecordType::LevelChanged => {}
        }
        Ok(())
    }

    async fn update_homeautomation_hook(&self, state: &Arc<Pv2MqttState>) -> anyhow::Result<()> {
        let hub = state.hub.load();

        let addr = hub.hub.suggest_bind_address().await?;
        hub.hub
            .enable_home_automation_hook(&format!(
                "{addr}:{http_port}/pv-postback/{serial}",
                http_port = state.http_port,
                serial = state.serial
            ))
            .await?;
        Ok(())
    }

    async fn handle_discovery(
        &self,
        state: &Arc<Pv2MqttState>,
        mut new_hub: ResolvedHub,
    ) -> anyhow::Result<()> {
        let hub = state.hub.load();
        match new_hub.user_data.take() {
            Some(user_data) => {
                if user_data.serial_number != state.serial {
                    // It's a different hub
                    return Ok(());
                }
                let changed = !state.responding.load(Ordering::SeqCst)
                    || user_data.ip != hub.user_data.ip
                    || user_data.hub_name != hub.user_data.hub_name;
                if !changed {
                    return Ok(());
                }

                log::info!("Hub ip, name or connectivity status changed");

                state.responding.store(true, Ordering::SeqCst);
                state.hub.store(Arc::new(FullyResolvedHub {
                    hub: hub.hub.clone(),
                    user_data,
                }));
                self.update_homeautomation_hook(state).await?;
                register_with_hass(&state).await?;
                Ok(())
            }
            None => {
                // Hub isn't responding. Do something to update an entity
                // in hass so that this is visible
                state.responding.store(false, Ordering::SeqCst);
                advise_hass_of_unresponsive(state).await?;
                Ok(())
            }
        }
    }

    async fn serve(
        &self,
        mut rx: Receiver<ServerEvent>,
        state: Arc<Pv2MqttState>,
        router: MqttRouter<Arc<Pv2MqttState>>,
    ) {
        log::info!("Waiting for mqtt and pv messages");
        while let Some(msg) = rx.recv().await {
            match msg {
                ServerEvent::MqttMessage(msg) => {
                    if let Err(err) = self.handle_mqtt_message(msg, &state, &router).await {
                        log::error!("handling mqtt message: {err:#}");
                    }
                }
                ServerEvent::HomeAutomationData { serial, mut data } => {
                    if serial != state.serial {
                        log::warn!(
                            "ignoring postback which is intended for \
                             serial={serial}, while we are serial {actual_serial}",
                            actual_serial = state.serial
                        );
                        continue;
                    }

                    // Re-order the events so that the closed/open events happen
                    // after closing/opening
                    data.sort_by(|a, b| a.record_type.cmp(&b.record_type));

                    for item in data {
                        if let Err(err) = self.handle_pv_event(&state, item).await {
                            log::error!("handling pv event: {err:#}");
                        }
                    }
                }

                ServerEvent::HubDiscovered(resolved_hub) => {
                    if let Err(err) = self.handle_discovery(&state, resolved_hub).await {
                        log::error!("While updating hass state: {err:#?}");
                    }
                }

                ServerEvent::PeriodicStateUpdate => {
                    if let Err(err) = register_with_hass(&state).await {
                        log::error!("While updating hass state: {err:#?}");
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct SerialAndScene {
    serial: String,
    #[serde(deserialize_with = "parse_deser")]
    scene_id: i32,
}

async fn mqtt_scene_activate(
    SerialAndScene { serial, scene_id }: SerialAndScene,
    msg: Message,
    state: Arc<Pv2MqttState>,
) -> anyhow::Result<()> {
    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
            topic = msg.topic,
            actual_serial = state.serial
        );
        return Ok(());
    }

    state.hub.load().hub.activate_scene(scene_id).await?;
    Ok(())
}

struct ShadeIdAddr {
    shade_id: i32,
    is_secondary: bool,
}

impl FromStr for ShadeIdAddr {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<ShadeIdAddr> {
        let (shade_id, is_secondary) = if let Some(id) = s.strip_suffix(SECONDARY_SUFFIX) {
            (id.parse::<i32>()?, true)
        } else {
            (s.parse::<i32>()?, false)
        };
        Ok(ShadeIdAddr {
            shade_id,
            is_secondary,
        })
    }
}

#[derive(Deserialize)]
struct SerialAndShade {
    serial: String,
    #[serde(deserialize_with = "parse_deser")]
    shade_id: ShadeIdAddr,
}
async fn mqtt_shade_set_position(
    params: SerialAndShade,
    msg: Message,
    state: Arc<Pv2MqttState>,
) -> anyhow::Result<()> {
    let SerialAndShade {
        serial,
        shade_id: ShadeIdAddr {
            shade_id,
            is_secondary,
        },
    } = params;

    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
            topic = msg.topic,
            actual_serial = state.serial
        );
        return Ok(());
    }

    let payload = String::from_utf8_lossy(&msg.payload);

    let position: u8 = payload.parse()?;

    let hub = state.hub.load();
    let shade = hub.hub.shade_by_id(shade_id).await?;

    let mut shade_pos = shade
        .positions
        .clone()
        .ok_or_else(|| anyhow::anyhow!("shade {shade_id} has no existing position"))?;

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
    hub.hub
        .change_shade_position(shade_id, shade_pos.clone())
        .await?;

    Ok(())
}

async fn mqtt_shade_command(
    params: SerialAndShade,
    msg: Message,
    state: Arc<Pv2MqttState>,
) -> anyhow::Result<()> {
    let SerialAndShade {
        serial,
        shade_id: ShadeIdAddr {
            shade_id,
            is_secondary: _,
        },
    } = params;

    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
            topic = msg.topic,
            actual_serial = state.serial
        );
        return Ok(());
    }

    let command = String::from_utf8_lossy(&msg.payload);
    let hub = state.hub.load();
    let shade = hub.hub.shade_by_id(shade_id).await?;

    log::info!("{command} {shade_id} {}", shade.name());
    match command.as_ref() {
        "OPEN" => {
            hub.hub.move_shade(shade_id, ShadeUpdateMotion::Up).await?;
        }
        "CLOSE" => {
            hub.hub
                .move_shade(shade_id, ShadeUpdateMotion::Down)
                .await?;
        }
        "STOP" => {
            hub.hub
                .move_shade(shade_id, ShadeUpdateMotion::Stop)
                .await?;
        }
        _ => {}
    }

    Ok(())
}

async fn mqtt_homeassitant_status(
    _: (),
    msg: Message,
    state: Arc<Pv2MqttState>,
) -> anyhow::Result<()> {
    log::info!(
        "Home Assistant status changed: {}",
        String::from_utf8_lossy(&msg.payload)
    );
    register_with_hass(&state).await
}

struct FullyResolvedHub {
    hub: Hub,
    user_data: UserData,
}

struct Pv2MqttState {
    hub: ArcSwap<FullyResolvedHub>,
    client: Client,
    serial: String,
    http_port: u16,
    discovery_prefix: String,
    first_run: AtomicBool,
    responding: AtomicBool,
}
