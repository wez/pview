use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_repr::*;
use std::convert::AsRef;

// <https://github.com/jlaur/hdpowerview-doc/>
// <https://github.com/openhab/openhab-addons/files/7583705/PowerView-Hub-REST-API-v2.pdf>
// <https://github.com/openhab/openhab-addons/issues/11533>

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RoomResponse {
    pub room_data: Vec<RoomData>,
    pub room_ids: Vec<u32>,
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct Base64Name(String);

impl std::fmt::Display for Base64Name {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(fmt)
    }
}

impl Base64Name {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for Base64Name {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

impl AsRef<str> for Base64Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Base64Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        let decoded_bytes = data_encoding::BASE64
            .decode(text.as_bytes())
            .map_err(|e| D::Error::custom(format!("{e:#}")))?;
        Ok(Base64Name(
            String::from_utf8(decoded_bytes).map_err(|e| D::Error::custom(format!("{e:#}")))?,
        ))
    }
}

impl Serialize for Base64Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = data_encoding::BASE64.encode(&self.0.as_bytes());
        encoded.serialize(serializer)
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RoomData {
    pub color_id: i32,
    pub icon_id: i32,
    pub id: i32,
    pub name: Base64Name,
    pub order: i32,
    #[serde(rename = "type")]
    pub room_type: RoomType,
}

#[derive(Serialize_repr, Deserialize_repr, Debug)]
#[repr(i32)]
pub enum RoomType {
    Regular = 0,
    Repeater = 1,
    DefaultRoom = 2,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ShadesResponse {
    pub shade_data: Vec<ShadeData>,
    pub shade_ids: Vec<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ShadeData {
    pub battery_status: BatteryStatus,
    /// Battery level from 0-200
    pub battery_strength: i32,
    pub firmware: Option<ShadeFirmware>,
    pub capabilities: ShadeCapabilities,
    pub battery_kind: ShadeBatteryKind,
    pub smart_power_supply: SmartPowerSupply,
    /// Signal level from 0-4
    pub signal_strength: Option<i32>,
    pub motor: Option<Motor>,
    pub group_id: i32,
    pub id: i32,
    pub name: Option<Base64Name>,
    /// Display order
    pub order: Option<i32>,
    pub positions: Option<ShadePosition>,
    pub room_id: Option<i32>,
    /// The secondary name of the shade base64 encoded. Used by the Apple Home application as the
    /// secondary service name to control shades with blackout blinds or a top rail movement.
    pub secondary_name: Option<Base64Name>,
    #[serde(rename = "type")]
    pub shade_type: ShadeType,
    #[serde(default)]
    pub timed_out: bool,
}

impl ShadeData {
    pub fn name(&self) -> &str {
        self.name.as_ref().map(|s| s.as_str()).unwrap_or("unknown")
    }

    pub fn secondary_name(&self) -> String {
        if let Some(name) = &self.secondary_name {
            name.as_str().to_string()
        } else {
            format!("{} Middle Rail", self.name())
        }
    }

    pub fn battery_percent(&self) -> Option<u8> {
        if self.battery_status == BatteryStatus::Unavailable {
            None
        } else {
            Some((self.battery_strength / 2) as u8)
        }
    }

    pub fn signal_strength_percent(&self) -> Option<u8> {
        self.signal_strength
            .map(|level| ((level as u16) * 100 / 4) as u8)
    }

    pub fn pos1_percent(&self) -> Option<u8> {
        self.positions.as_ref().map(|p| p.pos1_percent())
    }

    pub fn pos2_percent(&self) -> Option<u8> {
        self.positions.as_ref().and_then(|p| p.pos2_percent())
    }
}

#[derive(Serialize_repr, Deserialize_repr, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum BatteryStatus {
    Unavailable = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    PluggedIn = 4,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ShadeFirmware {
    pub build: i32,
    pub index: Option<i32>,
    pub revision: i32,
    pub sub_revision: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ShadePosition {
    pub pos_kind_1: PositionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_kind_2: Option<PositionKind>,
    pub position_1: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_2: Option<u16>,
}

impl ShadePosition {
    pub fn describe(&self) -> String {
        if let Some(pos2) = self.position_2 {
            format!(
                "{} {}",
                self.describe_pos(self.position_1),
                self.describe_pos(pos2)
            )
        } else {
            self.describe_pos(self.position_1)
        }
    }

    pub fn pos_to_percent(pos: u16) -> u8 {
        (100u32 * pos as u32 / u16::max_value() as u32) as u8
    }

    pub fn percent_to_pos(pct: u8) -> u16 {
        ((u16::max_value() as u32) * (pct as u32) / 100u32) as u16
    }

    pub fn pos1_percent(&self) -> u8 {
        Self::pos_to_percent(self.position_1)
    }

    pub fn pos2_percent(&self) -> Option<u8> {
        self.position_2.map(Self::pos_to_percent)
    }

    pub fn describe_pos1(&self) -> String {
        self.describe_pos(self.position_1)
    }

    pub fn describe_pos2(&self) -> String {
        if let Some(pos2) = self.position_2 {
            self.describe_pos(pos2)
        } else {
            String::new()
        }
    }

    pub fn describe_pos(&self, pos: u16) -> String {
        format!("{}%", Self::pos_to_percent(pos))
    }
}

#[derive(Serialize_repr, Deserialize_repr, Debug, Clone, Copy)]
#[repr(i32)]
pub enum PositionKind {
    None = 0,
    PrimaryRail = 1,
    SecondaryRail = 2,
    VaneTilt = 3,
    Error = 4,
}

#[derive(Serialize_repr, Deserialize_repr, Debug)]
#[repr(i32)]
pub enum ShadeType {
    Roller = 1,
    Type2 = 2,
    Roman = 4,
    Type5 = 5,
    Duette = 6,
    TopDown = 7,
    DuetteTopDownBottomUp = 8,
    DuetteDuoLiteTopDownBottomUp = 9,
    Piroutte = 18,
    Silhouette = 23,
    SilhouetteDuolite = 38,
    RollerBlind = 42,
    Facette = 43,
    Twist = 44,
    PleatedTopDownBottomUp = 47,
    ACRoller = 49,
    Venetian = 51,
    VerticalSlatsLeftStack = 54,
    VerticalSlatsRightStack = 55,
    VerticalSlatsSplitStack = 56,
    Venetian62 = 62,
    VignetteDuolite = 65,
    Shutter = 66,
    CurtainLeftStack = 69,
    CurtainRightStack = 70,
    CurtainSplitStack = 71,
    DuoliteLift = 79,
}

#[derive(Serialize_repr, Deserialize_repr, Debug, Copy, Clone)]
#[repr(i32)]
pub enum ShadeCapabilities {
    BottomUp = 0,
    BottomUpTilt90 = 1,
    BottomUpTilt180 = 2,
    VerticalTilt180 = 3,
    Vertical = 4,
    TiltOnly180 = 5,
    TopDown = 6,
    TopDownBottomUp = 7,
    DualOverlapped = 8,
    DualOverlappedTilt90 = 9,
}

impl ShadeCapabilities {
    pub fn flags(self) -> ShadeCapabilityFlags {
        match self {
            Self::BottomUp => ShadeCapabilityFlags::PRIMARY_RAIL,
            Self::BottomUpTilt90 => {
                ShadeCapabilityFlags::PRIMARY_RAIL | ShadeCapabilityFlags::TILT_ON_CLOSED
            }
            Self::BottomUpTilt180 => {
                ShadeCapabilityFlags::PRIMARY_RAIL
                    | ShadeCapabilityFlags::TILT_ANYWHERE
                    | ShadeCapabilityFlags::TILT_180
            }
            Self::VerticalTilt180 => {
                ShadeCapabilityFlags::PRIMARY_RAIL
                    | ShadeCapabilityFlags::TILT_ANYWHERE
                    | ShadeCapabilityFlags::TILT_180
            }
            Self::Vertical => ShadeCapabilityFlags::PRIMARY_RAIL,
            Self::TiltOnly180 => {
                ShadeCapabilityFlags::TILT_ANYWHERE | ShadeCapabilityFlags::TILT_180
            }
            Self::TopDown => {
                ShadeCapabilityFlags::PRIMARY_RAIL | ShadeCapabilityFlags::PRIMARY_RAIL_REVERSED
            }
            Self::TopDownBottomUp => {
                ShadeCapabilityFlags::PRIMARY_RAIL | ShadeCapabilityFlags::SECONDARY_RAIL
            }
            Self::DualOverlapped => {
                ShadeCapabilityFlags::PRIMARY_RAIL
                    | ShadeCapabilityFlags::SECONDARY_RAIL
                    | ShadeCapabilityFlags::SECONDARY_RAIL_OVERLAPPED
            }
            Self::DualOverlappedTilt90 => {
                ShadeCapabilityFlags::PRIMARY_RAIL
                    | ShadeCapabilityFlags::SECONDARY_RAIL
                    | ShadeCapabilityFlags::SECONDARY_RAIL_OVERLAPPED
                    | ShadeCapabilityFlags::TILT_ON_CLOSED
            }
        }
    }
}

bitflags::bitflags! {
    pub struct ShadeCapabilityFlags : u8 {
        const PRIMARY_RAIL = 1;
        const SECONDARY_RAIL = 2;
        const TILT_ON_CLOSED = 4;
        const TILT_ANYWHERE = 8;
        const TILT_180 = 16;
        const PRIMARY_RAIL_REVERSED = 32;
        const SECONDARY_RAIL_OVERLAPPED = 64;
    }
}

#[derive(Serialize_repr, Deserialize_repr, Debug, Copy, Clone)]
#[repr(i32)]
pub enum ShadeBatteryKind {
    HardWiredPowerSupply = 1,
    BatteryWand = 2,
    RechargeableBattery = 3,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SmartPowerSupply {
    pub status: i32,
    pub id: i32,
    pub port: i32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Motor {
    pub revision: i32,
    pub sub_revision: i32,
    pub build: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, clap::ValueEnum)]
#[serde(rename_all = "camelCase")]
pub enum ShadeUpdateMotion {
    Down,
    Heart,
    Jog,
    LeftTilt,
    RightTilt,
    Stop,
    Up,
    Calibrate,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub color_id: i32,
    pub icon_id: i32,
    pub id: i32,
    pub name: Base64Name,
    pub network_number: i32,
    pub order: i32,
    pub room_id: i32,
    pub hk_assist: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ScenesResponse {
    pub scene_data: Vec<Scene>,
    pub scene_ids: Vec<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SceneMember {
    pub id: i32,
    pub scene_id: i32,
    pub shade_id: i32,
    #[serde(rename = "type")]
    pub member_type: i32,
    pub positions: ShadePosition,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SceneMembersResponse {
    pub scene_member_data: Vec<SceneMember>,
    pub scene_member_ids: Vec<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UserDataResponse {
    pub user_data: UserData,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UserData {
    pub hub_name: Base64Name,
    pub local_time_data_set: bool,
    pub enable_scheduled_events: bool,
    pub editing_enabled: bool,
    pub setup_completed: bool,
    /// network router gateway
    pub gateway: String,
    /// dns server
    pub dns: String,
    /// whether the IP is statically configured
    pub static_ip: bool,
    #[serde(rename = "_id")]
    pub id: String,
    /// The color of the light and repeaters
    pub color: Color,
    pub auto_backup: bool,
    /// Current ip address
    pub ip: String,
    pub mac_address: String,
    /// netmask
    pub mask: String,
    /// whether it is in AP mode
    pub wireless: bool,
    /// the ssid of AP mode
    pub ssid: Option<String>,
    pub firmware: FirmwareInfo,
    pub serial_number: String,
    #[serde(rename = "rfIDInt")]
    pub rf_id_int: u32,
    #[serde(rename = "rfID")]
    pub rf_id: String,
    pub rf_status: i32,
    pub times: TimeConfiguration,
    pub brand: String,
    pub rc_up: bool,
    pub remote_connect_enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FirmwareInfo {
    pub main_processor: MainProcessorFirmware,
    pub radio: MainProcessorFirmware,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MainProcessorFirmware {
    pub name: Option<String>,
    pub revision: i32,
    pub sub_revision: i32,
    pub build: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub brightness: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TimeConfiguration {
    pub timezone: String,
    pub local_sunrise_time_in_minutes: i64,
    pub local_sunset_time_in_minutes: i64,
    pub current_offset: i64,
    pub longitude: Option<f64>,
    pub latitude: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HomeAutomationPostBackData {
    pub duration_ms: Option<i64>,
    pub remaining_duration_ms: Option<i64>,
    pub initial_position: Option<u8>,
    pub service: HomeAutomationService,
    pub shade_id: i32,
    pub target_position: Option<u8>,
    pub current_position: Option<u8>,
    #[serde(rename = "type")]
    pub record_type: HomeAutomationRecordType,
    pub stopped_position: Option<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, clap::ValueEnum)]
#[serde(rename_all = "camelCase")]
pub enum HomeAutomationService {
    Primary,
    Secondary,
}

// Note that the order of the enum variants is significant!
// We want the final state items to sort after the others,
// otherwise we'll send incorrect state updates to hass.
#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, clap::ValueEnum, Ord, PartialOrd, Eq, PartialEq,
)]
#[serde(rename_all = "camelCase")]
pub enum HomeAutomationRecordType {
    StartsOpening,
    StartsClosing,
    BeginsMoving,
    TargetLevelChanged,
    LevelChanged,
    HasOpened,
    HasFullyOpened,
    HasFullyClosed,
    HasClosed,
    Stops,
}
