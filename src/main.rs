use clap::Parser;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::Mutex;

mod api_types;
mod commands;
mod discovery;
mod hass_helper;
mod http_helpers;
mod hub;
mod version_info;

use crate::hub::*;

#[derive(Parser, Debug)]
#[command(version = version_info::pview_version())]
pub struct Args {
    #[command(subcommand)]
    cmd: SubCommand,

    /// Instead of performing discovery, specify the hub ip address.
    /// You may also set this via the PV_HUB_IP environment variable.
    #[arg(long)]
    hub_ip: Option<IpAddr>,

    /// When doing discovery for hubs, match the hub with the specified
    /// serial number(s). This is useful if you have multiple hubs and
    /// want to specify an individual hub.
    /// You may also set this via the PV_HUB_SERIAL environment variable.
    #[arg(long)]
    hub_serial: Option<String>,

    #[arg(skip)]
    hub_instance: Mutex<Option<Hub>>,

    #[arg(long, default_value = "15", value_parser = parse_duration)]
    discovery_timeout: Duration,
}

fn parse_duration(arg: &str) -> Result<Duration, std::num::ParseIntError> {
    let seconds = arg.parse()?;
    Ok(Duration::from_secs(seconds))
}

#[derive(Parser, Debug)]
pub enum SubCommand {
    ListScenes(commands::list_scenes::ListScenesCommand),
    ListShades(commands::list_shades::ListShadesCommand),
    InspectShade(commands::inspect_shade::InspectShadeCommand),
    MoveShade(commands::move_shade::MoveShadeCommand),
    ActivateScene(commands::activate_scene::ActivateSceneCommand),
    ServeMqtt(commands::serve_mqtt::ServeMqttCommand),
    HubInfo(commands::hub_info::HubInfoCommand),
    ListHubs(commands::list_hubs::ListHubsCommand),
}

impl SubCommand {
    pub async fn run(&self, args: &Args) -> anyhow::Result<()> {
        match self {
            Self::ListScenes(cmd) => cmd.run(args).await,
            Self::ListShades(cmd) => cmd.run(args).await,
            Self::InspectShade(cmd) => cmd.run(args).await,
            Self::MoveShade(cmd) => cmd.run(args).await,
            Self::ActivateScene(cmd) => cmd.run(args).await,
            Self::ServeMqtt(cmd) => cmd.run(args).await,
            Self::HubInfo(cmd) => cmd.run(args).await,
            Self::ListHubs(cmd) => cmd.run(args).await,
        }
    }
}

impl Args {
    pub async fn run(&self) -> anyhow::Result<()> {
        self.cmd.run(self).await
    }

    pub fn hub_ip_was_specified_by_user(&self) -> bool {
        self.hub_ip.is_some() || std::env::var_os("PV_HUB_IP").is_some()
    }

    pub fn hub_ip(&self) -> anyhow::Result<Option<IpAddr>> {
        match self.hub_ip.clone() {
            Some(u) => Ok(Some(u)),
            None => opt_env_var("PV_HUB_IP"),
        }
    }

    pub fn hub_serial(&self) -> anyhow::Result<Option<String>> {
        match self.hub_serial.clone() {
            Some(u) => Ok(Some(u)),
            None => opt_env_var("PV_HUB_SERIAL"),
        }
    }

    pub async fn hub(&self) -> anyhow::Result<Hub> {
        let mut lock = self.hub_instance.lock().await;
        match lock.as_ref() {
            Some(hub) => Ok(hub.clone()),
            None => {
                let addr = self.hub_ip()?;

                let hub = match addr {
                    Some(addr) => Hub::with_addr(addr),
                    None => {
                        let serial = self.hub_serial()?;
                        match serial {
                            Some(serial) => {
                                crate::discovery::resolve_hub_with_serial(
                                    Some(self.discovery_timeout),
                                    &serial,
                                )
                                .await?
                            }
                            None => Hub::discover(self.discovery_timeout).await?,
                        }
                    }
                };
                lock.replace(hub.clone());
                Ok(hub)
            }
        }
    }
}

pub fn opt_env_var<T: FromStr>(name: &str) -> anyhow::Result<Option<T>>
where
    <T as FromStr>::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(p) => {
            Ok(Some(p.parse().map_err(|err| {
                anyhow::anyhow!("parsing ${name}: {err:#}")
            })?))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => anyhow::bail!("${name} is invalid: {err:#}"),
    }
}

// We use only 2 worker threads here because, by design and intent,
// we largely serialize request processing to avoid swamping the hub.
// On machines with a high core count, using the default setting would
// spawn a lot of threads that we will never use. Keeping the thread
// count small helps to reduce the resource footprint when running
// the mqtt bridge.
#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    color_backtrace::install();
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loading environment overrides from {path:?}");
    }

    let tz: chrono_tz::Tz = iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let utc_suffix = if tz == chrono_tz::UTC { "Z" } else { "" };

    env_logger::builder()
        // A bit of boilerplate here to get timestamps printed in local time.
        // <https://github.com/rust-cli/env_logger/issues/158>
        .format(move |buf, record| {
            use chrono::Utc;
            use env_logger::fmt::Color;
            use std::io::Write;

            let subtle = buf
                .style()
                .set_color(Color::Black)
                .set_intense(true)
                .clone();
            write!(buf, "{}", subtle.value("["))?;
            write!(
                buf,
                "{}{utc_suffix} ",
                Utc::now().with_timezone(&tz).format("%Y-%m-%dT%H:%M:%S")
            )?;
            write!(buf, "{:<5}", buf.default_styled_level(record.level()))?;
            if let Some(path) = record.module_path() {
                write!(buf, " {}", path)?;
            }
            write!(buf, "{}", subtle.value("]"))?;
            writeln!(buf, " {}", record.args())
        })
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let args = Args::parse();
    args.run().await
}
