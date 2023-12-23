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
    pub battery_strength: i32,
    pub firmware: Option<ShadeFirmware>,
    pub capabilities: ShadeCapabilities,
    pub battery_kind: ShadeBatteryKind,
    pub smart_power_supply: SmartPowerSupply,
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
}

#[derive(Serialize_repr, Deserialize_repr, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ShadePosition {
    pub pos_kind_1: PositionKind,
    pub pos_kind_2: Option<PositionKind>,
    pub position_1: u16,
    pub position_2: Option<u16>,
}

#[derive(Serialize_repr, Deserialize_repr, Debug)]
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
