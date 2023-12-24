use crate::api_types::ShadeUpdateMotion;
use mosquitto_rs::*;
use std::collections::HashMap;
use std::time::Duration;

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

impl ServeMqttCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;
        let user_data = hub.get_user_data().await?;
        let shades = hub.list_shades(None, None).await?;
        let room_by_id: HashMap<_, _> = hub
            .list_rooms()
            .await?
            .into_iter()
            .map(|room| (room.id, room.name))
            .collect();

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
        client.subscribe("pv2mqtt/shade/+/state", QoS::AtMostOnce).await?;
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
                        "suggested_area": shade.room_id.and_then(|room_id| room_by_id.get(&room_id).map(|name| serde_json::json!(name.as_str()))).unwrap_or(serde_json::Value::Null),
                        "identifiers": [
                            unique_id
                        ],
                        "name": shade_name,
                        "manufacturer": "Hunter Douglas",
                        "model": "pv2mqtt",
                        "sw_version": shade.firmware.as_ref().map(|vers| {
                            format!("{}.{}.{}", vers.revision, vers.sub_revision, vers.build)
                        }).unwrap_or_else(|| "unknown".to_string()),
                    },
                    "platform": "mqtt",
                });

                // Tell hass about this shade
                client
                    .publish(
                        &format!("{}/cover/{shade_id}/config", self.discovery_prefix),
                        dbg!(serde_json::to_string(&data)?).as_bytes(),
                        QoS::AtMostOnce,
                        false,
                    )
                    .await?;

                tokio::time::sleep(Duration::from_millis(500)).await;

                client
                    .publish(
                        &format!("pv2mqtt/shade/{shade_id}/availability"),
                        b"online",
                        QoS::AtMostOnce,
                        false,
                    )
                    .await?;

                handle_position(&mut client, &shade_id, pos).await?;
            }

            break;
        }

        println!("Listening");

        async fn advise_of_state(
            client: &mut Client,
            shade_id: &str,
            position: u8,
        ) -> anyhow::Result<()> {
            let state = if position == 0 { "closed" } else { "open" };

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

            advise_of_state(client, shade_id, position).await?;
            Ok(())
        }

        while let Ok(msg) = subscriber.recv().await {
            println!("msg: {msg:?}");
            let topic: Vec<_> = msg.topic.split('/').collect();
            if let [_, device_kind, shade_id, kind] = topic.as_slice() {
                let (actual_shade_id, is_secondary) =
                    if let Some(id) = shade_id.strip_suffix(SECONDARY_SUFFIX) {
                        (id.parse::<i32>()?, true)
                    } else {
                        (shade_id.parse::<i32>()?, false)
                    };

                let shade = hub.shade_by_id(actual_shade_id).await?;

                let payload = String::from_utf8_lossy(&msg.payload);
                println!("{kind} Cover: {shade_id} 2nd={is_secondary}, payload={payload}");
                match kind.as_ref() {
                    "set_position" => {
                        let position: u8 = payload.parse()?;

                        let mut shade_pos = shade.positions.clone().ok_or_else(|| {
                            anyhow::anyhow!("shade {actual_shade_id} has no existing position")
                        })?;

                        let absolute =
                            ((u16::max_value() as u32) * (position as u32) / 100u32) as u16;

                        if is_secondary {
                            shade_pos.position_2.replace(absolute);
                        } else {
                            shade_pos.position_1 = absolute;
                        }

                        hub.change_shade_position(actual_shade_id, shade_pos.clone())
                            .await?;

                        handle_position(&mut client, shade_id, position).await?;
                    }
                    "command" => match payload.as_ref() {
                        "OPEN" => {
                            hub.move_shade(actual_shade_id, ShadeUpdateMotion::Up)
                                .await?;
                            handle_position(&mut client, shade_id, 100).await?;
                        }
                        "CLOSE" => {
                            hub.move_shade(actual_shade_id, ShadeUpdateMotion::Down)
                                .await?;
                            handle_position(&mut client, shade_id, 0).await?;
                        }
                        "STOP" => {
                            hub.move_shade(actual_shade_id, ShadeUpdateMotion::Stop)
                                .await?;
                            // TODO: report current position?
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        Ok(())
    }
}
