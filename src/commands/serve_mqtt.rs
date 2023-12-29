use crate::api_types::{
    HomeAutomationPostBackData, HomeAutomationRecordType, HomeAutomationService, ShadeBatteryKind,
    ShadeCapabilityFlags, ShadeData, ShadePosition, ShadeUpdateMotion, UserData,
};
use crate::discovery::ResolvedHub;
use crate::hass_helper::*;
use crate::hub::Hub;
use crate::mqtt_helper::*;
use crate::opt_env_var;
use crate::version_info::pview_version;
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
const WEZ: &str = "Wez Furlong";
const HUNTER_DOUGLAS: &str = "Hunter Douglas";
const BATTERY_LABEL: &str = "Battery";
const RECHARGEABLE_LABEL: &str = "Rechargeable Battery";
const HARD_WIRED_LABEL: &str = "Hard Wired";

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
            updates: vec![],
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
        let is_first_run = state.first_run.load(Ordering::SeqCst);

        if is_first_run {
            if !self.configs.is_empty() && !self.updates.is_empty() {
                // Delay between registering configs and advising hass
                // of the states, so that hass has had enough time
                // to subscribe to the correct topics
                let delay = self.configs.len() as u64 * 30;
                log::info!(
                    "there are {} configs, and {} updates. delay ms = {delay}",
                    self.configs.len(),
                    self.updates.len()
                );
                self.updates
                    .insert(0, RegEntry::Delay(Duration::from_millis(delay)));
            }
        } else {
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

    let config = SensorConfig {
        base: EntityConfig {
            name: Some(diagnostic.name),
            availability_topic: format!("{MODEL}/sensor/{unique_id}/availability"),
            device: Device {
                identifiers: vec![
                    format!("{MODEL}-{serial}"),
                    user_data.serial_number.to_string(),
                    user_data.mac_address.to_string(),
                ],
                connections: vec![("mac".to_string(), user_data.mac_address.to_string())],
                name: format!(
                    "{} PowerView Hub: {}",
                    user_data.brand,
                    user_data.hub_name.to_string()
                ),
                manufacturer: WEZ.to_string(),
                model: MODEL.to_string(),
                sw_version: Some(pview_version().to_string()),
                suggested_area: None,
                via_device: None,
            },
            device_class: None,
            origin: Origin::default(),
            unique_id: unique_id.to_string(),
            entity_category: Some("diagnostic".to_string()),
            icon: None,
        },
        state_topic: format!("{MODEL}/sensor/{unique_id}/state"),
        unit_of_measurement: None,
    };

    reg.config(
        format!("{}/sensor/{unique_id}/config", state.discovery_prefix),
        serde_json::to_string(&config)?,
    );

    reg.update(config.base.availability_topic, "online");

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

        let mut shades = vec![(shade.id.to_string(), None, Some(position.pos1_percent()))];

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
                Some("Middle Rail".to_string()),
                position.pos2_percent(),
            ));
        }

        let area = shade
            .room_id
            .and_then(|room_id| room_by_id.get(&room_id).map(|name| name.to_string()));

        let device_id = format!("{serial}-{}", shade.id);
        let device = Device {
            suggested_area: area,
            identifiers: vec![device_id.clone()],
            via_device: Some(format!("{MODEL}-{serial}")),
            name: shade.name().to_string(),
            manufacturer: HUNTER_DOUGLAS.to_string(),
            model: MODEL.to_string(),
            connections: vec![],
            sw_version: shade
                .firmware
                .as_ref()
                .map(|vers| format!("{}.{}.{}", vers.revision, vers.sub_revision, vers.build)),
        };

        for (shade_id, shade_name, pos) in shades {
            let unique_id = format!("{serial}-{shade_id}");

            let config = CoverConfig {
                base: EntityConfig {
                    unique_id,
                    name: shade_name,
                    availability_topic: format!("{MODEL}/shade/{serial}/{shade_id}/availability"),
                    device_class: Some("shade".to_string()),
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: None,
                    icon: None,
                },
                command_topic: format!("{MODEL}/shade/{serial}/{shade_id}/command"),
                position_topic: format!("{MODEL}/shade/{serial}/{shade_id}/position"),
                set_position_topic: format!("{MODEL}/shade/{serial}/{shade_id}/set_position"),
                state_topic: format!("{MODEL}/shade/{serial}/{shade_id}/state"),
            };

            // Delete legacy version of this shade, for those upgrading.
            // TODO: remove this, or find some way to keep track of what
            // version of things are already present in hass
            reg.delete(format!(
                "{}/cover/{shade_id}/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/cover/{serial}-{shade_id}/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&config)?,
            );

            reg.update(config.base.availability_topic, "online");

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

        {
            let jog = ButtonConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-jog"),
                    name: Some("Jog".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/jog/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: None,
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                payload_press: Some("JOG".to_string()),
            };

            reg.delete(format!(
                "{}/button/{device_id}-jog/config",
                state.discovery_prefix
            ));

            reg.config(
                format!("{}/button/{device_id}-jog/config", state.discovery_prefix),
                serde_json::to_string(&jog)?,
            );

            reg.update(jog.base.availability_topic, "online");
        }

        {
            let calibrate = ButtonConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-calibrate"),
                    name: Some("Calibrate".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/calibrate/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: Some("mdi:swap-vertical-circle-outline".to_string()),
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                payload_press: Some("CALIBRATE".to_string()),
            };
            reg.delete(format!(
                "{}/button/{device_id}-calibrate/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/button/{device_id}-calibrate/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&calibrate)?,
            );

            reg.update(calibrate.base.availability_topic, "online");
        }

        {
            let heart = ButtonConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-heart"),
                    name: Some("Move to Favorite Position".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/heart/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: Some("mdi:heart".to_string()),
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                payload_press: Some("HEART".to_string()),
            };
            reg.delete(format!(
                "{}/button/{device_id}-heart/config",
                state.discovery_prefix
            ));

            reg.config(
                format!("{}/button/{device_id}-heart/config", state.discovery_prefix),
                serde_json::to_string(&heart)?,
            );

            reg.update(heart.base.availability_topic, "online");
        }

        {
            let battery = SensorConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-battery"),
                    name: Some("Battery".to_string()),
                    availability_topic: state.battery_availability_topic(&shade),
                    device_class: Some("battery".to_string()),
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: None,
                },
                state_topic: state.battery_state_topic(&shade),
                unit_of_measurement: Some("%".to_string()),
            };
            reg.delete(format!(
                "{}/sensor/{device_id}-battery/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/sensor/{device_id}-battery/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&battery)?,
            );

            if let Some(pct) = shade.battery_percent() {
                reg.update(battery.base.availability_topic, "online");
                reg.update(battery.state_topic, format!("{pct}"));
            } else {
                reg.update(battery.base.availability_topic, "offline");
            }
        }
        {
            let refresh_battery = ButtonConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-rebattery"),
                    name: Some("Refresh Battery Status".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/rebattery/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: None,
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                payload_press: Some("UPDATE_BATTERY".to_string()),
            };

            reg.delete(format!(
                "{}/button/{device_id}-rebattery/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/button/{device_id}-rebattery/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&refresh_battery)?,
            );

            reg.update(refresh_battery.base.availability_topic, "online");
        }

        {
            let signal = SensorConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-signal"),
                    name: Some("Signal Strength".to_string()),
                    availability_topic: format!(
                        "{MODEL}/sensor/{serial}/{}/signal/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: Some("mdi:signal".to_string()),
                },
                state_topic: format!("{MODEL}/sensor/{device_id}-signal/state"),
                unit_of_measurement: Some("%".to_string()),
            };
            reg.delete(format!(
                "{}/sensor/{device_id}-signal/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/sensor/{device_id}-signal/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&signal)?,
            );

            if let Some(pct) = shade.signal_strength_percent() {
                reg.update(signal.base.availability_topic, "online");
                reg.update(signal.state_topic, format!("{pct}"));
            } else {
                reg.update(signal.base.availability_topic, "offline");
            }
        }

        {
            let refresh_position = ButtonConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-refresh"),
                    name: Some("Refresh Position".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/refresh/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: None,
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                payload_press: Some("REFRESH_POS".to_string()),
            };

            reg.delete(format!(
                "{}/button/{device_id}-refresh/config",
                state.discovery_prefix
            ));

            reg.config(
                format!(
                    "{}/button/{device_id}-refresh/config",
                    state.discovery_prefix
                ),
                serde_json::to_string(&refresh_position)?,
            );

            reg.update(refresh_position.base.availability_topic, "online");
        }

        {
            let power_source = SelectConfig {
                base: EntityConfig {
                    unique_id: format!("{device_id}-psu"),
                    name: Some("Power Source".to_string()),
                    availability_topic: format!(
                        "{MODEL}/shade/{serial}/{}/psu/availability",
                        shade.id
                    ),
                    device_class: None,
                    origin: Origin::default(),
                    device: device.clone(),
                    entity_category: Some("diagnostic".to_string()),
                    icon: Some("mdi:power-plug-outline".to_string()),
                },
                command_topic: format!("{MODEL}/shade/{serial}/{}/command", shade.id),
                state_topic: state.battery_kind_state_topic(&shade),
                options: vec![
                    HARD_WIRED_LABEL.to_string(),
                    BATTERY_LABEL.to_string(),
                    RECHARGEABLE_LABEL.to_string(),
                ],
            };
            reg.delete(format!(
                "{}/select/{device_id}-psu/config",
                state.discovery_prefix
            ));

            reg.config(
                format!("{}/select/{device_id}-psu/config", state.discovery_prefix),
                serde_json::to_string(&power_source)?,
            );

            reg.update(power_source.base.availability_topic, "online");
            reg.update(
                power_source.state_topic,
                battery_kind_to_state(shade.battery_kind).to_string(),
            );
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

        let suggested_area = room_by_id.get(&scene.room_id).map(|name| name.to_string());

        let unique_id = format!("{serial}-scene-{scene_id}");

        let config = SceneConfig {
            base: EntityConfig {
                device: Device {
                    suggested_area,
                    identifiers: vec![unique_id.clone()],
                    via_device: Some(format!("{MODEL}-{serial}")),
                    name: scene_name,
                    manufacturer: HUNTER_DOUGLAS.to_string(),
                    model: MODEL.to_string(),
                    connections: vec![],
                    sw_version: None,
                },
                availability_topic: format!("{MODEL}/scene/{serial}/{scene_id}/availability"),
                device_class: None,
                name: None,
                origin: Origin::default(),
                unique_id: unique_id.clone(),
                entity_category: None,
                icon: None,
            },
            command_topic: format!("{MODEL}/scene/{serial}/{scene_id}/set"),
            payload_on: "ON".to_string(),
        };

        // Delete legacy scene
        reg.delete(format!(
            "{}/scene/{unique_id}/config",
            state.discovery_prefix
        ));

        // Tell hass about this scene
        reg.config(
            format!("{}/scene/{unique_id}/config", state.discovery_prefix),
            serde_json::to_string(&config)?,
        );

        reg.update(config.base.availability_topic, "online");
    }

    Ok(())
}

async fn register_with_hass(state: &Arc<Pv2MqttState>) -> anyhow::Result<()> {
    let mut reg = HassRegistration::new();

    register_hub(&state.hub.load().user_data, state, &mut reg)
        .await
        .context("register_hub")?;
    register_shades(state, &mut reg)
        .await
        .context("register_shades")?;
    register_scenes(state, &mut reg)
        .await
        .context("register_scenes")?;
    reg.apply_updates(state).await.context("apply_updates")?;
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

fn battery_kind_to_state(kind: ShadeBatteryKind) -> &'static str {
    match kind {
        ShadeBatteryKind::HardWiredPowerSupply => HARD_WIRED_LABEL,
        ShadeBatteryKind::BatteryWand => BATTERY_LABEL,
        ShadeBatteryKind::RechargeableBattery => RECHARGEABLE_LABEL,
    }
}

async fn advise_hass_of_battery_kind(
    state: &Arc<Pv2MqttState>,
    shade: &ShadeData,
) -> anyhow::Result<()> {
    let state_topic = state.battery_kind_state_topic(shade);

    state
        .client
        .publish(
            state_topic,
            battery_kind_to_state(shade.battery_kind),
            QoS::AtMostOnce,
            false,
        )
        .await?;

    Ok(())
}

async fn advise_hass_of_battery_level(
    state: &Arc<Pv2MqttState>,
    shade: &ShadeData,
) -> anyhow::Result<()> {
    let availability_topic = state.battery_availability_topic(shade);
    let state_topic = state.battery_state_topic(shade);

    if let Some(pct) = shade.battery_percent() {
        state
            .client
            .publish(state_topic, format!("{pct}"), QoS::AtMostOnce, false)
            .await?;
        state
            .client
            .publish(availability_topic, "online", QoS::AtMostOnce, false)
            .await?;
    } else {
        state
            .client
            .publish(availability_topic, "offline", QoS::AtMostOnce, false)
            .await?;
    }

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
                self.update_homeautomation_hook(state)
                    .await
                    .context("update_homeautomation_hook")?;
                register_with_hass(&state)
                    .await
                    .context("register_with_hass")?;
                Ok(())
            }
            None => {
                // Hub isn't responding. Do something to update an entity
                // in hass so that this is visible
                state.responding.store(false, Ordering::SeqCst);
                advise_hass_of_unresponsive(state)
                    .await
                    .context("advise_hass_of_unresponsive")?;
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
        log::info!(
            "Version {}. Waiting for mqtt and pv messages",
            pview_version()
        );
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
                        log::error!("During handle_discovery: {err:#?}");
                    }
                }

                ServerEvent::PeriodicStateUpdate => {
                    if let Err(err) = register_with_hass(&state).await {
                        log::error!("During register_with_hass: {err:#?}");

                        // Look for a request error; it isn't the root cause but rather
                        // the penultimate cause, so we have to walk the chain to find it.
                        for cause in err.chain() {
                            if let Some(http_err) = cause.downcast_ref::<reqwest::Error>() {
                                if http_err.is_connect() {
                                    if let Err(err) = advise_hass_of_unresponsive(&state).await {
                                        log::error!(
                                            "While advising hass of unresponsive hub: {err:#}"
                                        );
                                    }
                                }
                                break;
                            }
                        }
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
    Params(SerialAndScene { serial, scene_id }): Params<SerialAndScene>,
    Topic(topic): Topic,
    State(state): State<Arc<Pv2MqttState>>,
) -> anyhow::Result<()> {
    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
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
    params: Params<SerialAndShade>,
    Topic(topic): Topic,
    State(state): State<Arc<Pv2MqttState>>,
    Payload(position): Payload<u8>,
) -> anyhow::Result<()> {
    let Params(SerialAndShade {
        serial,
        shade_id: ShadeIdAddr {
            shade_id,
            is_secondary,
        },
    }) = params;

    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
            actual_serial = state.serial
        );
        return Ok(());
    }

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
    params: Params<SerialAndShade>,
    Topic(topic): Topic,
    State(state): State<Arc<Pv2MqttState>>,
    Payload(command): Payload<String>,
) -> anyhow::Result<()> {
    let Params(SerialAndShade {
        serial,
        shade_id: ShadeIdAddr {
            shade_id,
            is_secondary: _,
        },
    }) = params;

    if serial != state.serial {
        log::warn!(
            "ignoring {topic} which is intended for \
                    serial={serial}, while we are serial {actual_serial}",
            actual_serial = state.serial
        );
        return Ok(());
    }

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
        "JOG" => {
            hub.hub.move_shade(shade_id, ShadeUpdateMotion::Jog).await?;
        }
        "CALIBRATE" => {
            hub.hub
                .move_shade(shade_id, ShadeUpdateMotion::Calibrate)
                .await?;
        }
        "HEART" => {
            hub.hub
                .move_shade(shade_id, ShadeUpdateMotion::Heart)
                .await?;
        }
        "UPDATE_BATTERY" => {
            let shade = hub.hub.shade_update_battery_level(shade_id).await?;
            advise_hass_of_battery_level(&state, &shade).await?;
        }
        "REFRESH_POS" => {
            let shade = hub.hub.shade_refresh_position(shade_id).await?;
            log::info!("shade: {shade:?}");
            // TODO: position update
        }
        BATTERY_LABEL => {
            let shade = hub
                .hub
                .change_battery_kind(shade_id, ShadeBatteryKind::BatteryWand)
                .await?;
            advise_hass_of_battery_kind(&state, &shade).await?;
        }
        RECHARGEABLE_LABEL => {
            let shade = hub
                .hub
                .change_battery_kind(shade_id, ShadeBatteryKind::RechargeableBattery)
                .await?;
            advise_hass_of_battery_kind(&state, &shade).await?;
        }
        HARD_WIRED_LABEL => {
            let shade = hub
                .hub
                .change_battery_kind(shade_id, ShadeBatteryKind::HardWiredPowerSupply)
                .await?;
            advise_hass_of_battery_kind(&state, &shade).await?;
        }
        _ => {
            log::warn!("Command {command} has no handler");
        }
    }

    Ok(())
}

async fn mqtt_homeassitant_status(
    Payload(status): Payload<String>,
    State(state): State<Arc<Pv2MqttState>>,
) -> anyhow::Result<()> {
    log::info!("Home Assistant status changed: {status}",);
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

impl Pv2MqttState {
    pub fn battery_availability_topic(&self, shade: &ShadeData) -> String {
        format!(
            "{MODEL}/sensor/{}/{}/battery/availability",
            self.serial, shade.id
        )
    }

    pub fn battery_state_topic(&self, shade: &ShadeData) -> String {
        format!("{MODEL}/sensor/{}-{}-battery/state", self.serial, shade.id)
    }

    pub fn battery_kind_state_topic(&self, shade: &ShadeData) -> String {
        format!("{MODEL}/select/{}/{}/psu/state", self.serial, shade.id)
    }
}
