use std::sync::atomic::{AtomicI64, Ordering};

use serde::{Deserialize, Serialize};

pub const TOKEN_REFRESH_BUFFER_SECS: i64 = 300;

fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
  D: serde::Deserializer<'de>,
{
  #[derive(Deserialize)]
  #[serde(untagged)]
  enum StringOrNumber {
    String(String),
    Number(serde_json::Number),
  }

  match StringOrNumber::deserialize(deserializer)? {
    StringOrNumber::String(value) => Ok(value),
    StringOrNumber::Number(value) => Ok(value.to_string()),
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaDevice {
  pub id: String,
  pub name: String,
  pub online: bool,
  pub category: String,
  pub product_id: String,
  pub product_name: String,
  pub local_key: String,
  pub sub: bool,
  pub uuid: String,
  pub owner_id: String,
  #[serde(default)]
  pub ip: String,
  pub time_zone: String,
  pub create_time: i64,
  pub update_time: i64,
  pub active_time: i64,
  #[serde(default)]
  pub icon: String,
  #[serde(default, alias = "room_id", alias = "roomId")]
  pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuyaDeviceStatus {
  pub code: String,
  pub value: TuyaValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TuyaValue {
  Boolean(bool),
  String(String),
  Integer(i64),
  Float(f64),
}

impl TuyaValue {
  pub fn as_bool(&self) -> Option<bool> {
    match self {
      TuyaValue::Boolean(v) => Some(*v),
      TuyaValue::String(s) => match s.to_lowercase().trim() {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
        _ => None,
      },
      TuyaValue::Integer(v) => match v {
        1 => Some(true),
        0 => Some(false),
        _ => None,
      },
      _ => None,
    }
  }

  pub fn as_string(&self) -> Option<&str> {
    match self {
      TuyaValue::String(v) => Some(v.as_str()),
      _ => None,
    }
  }

  pub fn as_i64(&self) -> Option<i64> {
    match self {
      TuyaValue::Integer(v) => Some(*v),
      TuyaValue::Float(v) => Some(*v as i64),
      TuyaValue::String(s) => {
        let trimmed = s.trim();
        if let Ok(i) = trimmed.parse::<i64>() {
          Some(i)
        } else if let Ok(f) = trimmed.parse::<f64>() {
          Some(f as i64)
        } else {
          let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
          if !digits.is_empty() {
            digits.parse::<i64>().ok()
          } else {
            None
          }
        }
      }
      _ => None,
    }
  }

  #[allow(dead_code)]
  pub fn as_f64(&self) -> Option<f64> {
    match self {
      TuyaValue::Float(v) => Some(*v),
      TuyaValue::Integer(v) => Some(*v as f64),
      TuyaValue::String(s) => s.trim().parse::<f64>().ok(),
      _ => None,
    }
  }
}

impl std::fmt::Display for TuyaValue {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      TuyaValue::Boolean(v) => write!(f, "{}", v),
      TuyaValue::String(v) => write!(f, "{}", v),
      TuyaValue::Integer(v) => write!(f, "{}", v),
      TuyaValue::Float(v) => write!(f, "{}", v),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaCommand {
  pub code: String,
  pub value: TuyaValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaCommandPayload {
  pub commands: Vec<TuyaCommand>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuyaApiResponse<T> {
  pub success: bool,
  pub result: Option<T>,
  pub code: Option<i32>,
  pub msg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaHome {
  #[serde(alias = "home_id", deserialize_with = "deserialize_string_or_number")]
  pub id: String,
  #[serde(default, alias = "home_name")]
  pub name: String,
  #[serde(default, alias = "roomList", alias = "room_list")]
  pub rooms: Vec<TuyaRoom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaHomeRooms {
  #[serde(
    default,
    alias = "home_id",
    deserialize_with = "deserialize_string_or_number"
  )]
  pub home_id: String,
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub rooms: Vec<TuyaRoom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaRoom {
  #[serde(alias = "room_id", deserialize_with = "deserialize_string_or_number")]
  pub id: String,
  #[serde(default, alias = "room_name")]
  pub name: String,
  #[serde(
    default,
    alias = "deviceIds",
    alias = "device_ids",
    alias = "deviceList"
  )]
  pub device_ids: Vec<String>,
  #[serde(default)]
  pub devices: Vec<TuyaRoomDevice>,
  #[serde(skip)]
  pub home_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuyaRoomDevice {
  #[serde(alias = "device_id")]
  pub id: String,
  #[serde(default, alias = "room_id", alias = "roomId")]
  pub room_id: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
  pub access_token: String,
  pub refresh_token: String,
  pub expire_time: i64,
}

#[derive(Debug, Clone)]
pub struct TokenState {
  pub access_token: String,
  pub refresh_token: String,
  pub expires_at: i64,
}

impl TokenState {
  pub fn is_expired(&self) -> bool {
    let now = chrono::Utc::now().timestamp();
    now >= (self.expires_at - TOKEN_REFRESH_BUFFER_SECS)
  }
}

pub const AC_MODES: &[&str] = &["auto", "cold", "dry", "wind"];

pub const FAN_SPEED_LEVELS: i32 = 5;

pub const AC_FAN_SPEED_LEVELS: i32 = 4;

pub const TEMP_MIN: i32 = 16;
pub const TEMP_MAX: i32 = 30;
pub const LIGHT_BRIGHTNESS_CODES: &[&str] = &["bright_value_v2", "bright_value", "brightness"];
pub const LIGHT_TEMPERATURE_CODES: &[&str] = &["temp_value_v2", "temp_value", "temperature"];
pub const LIGHT_SWITCH_CODES: &[&str] = &["switch_led", "switch", "power", "switch_1"];
pub static LIGHT_OPERATION_UNTIL: AtomicI64 = AtomicI64::new(0);

pub fn mark_light_operation() {
  let until = chrono::Utc::now().timestamp_millis() + 5000;
  LIGHT_OPERATION_UNTIL.store(until, Ordering::SeqCst);
}

pub fn light_operation_active() -> bool {
  chrono::Utc::now().timestamp_millis() < LIGHT_OPERATION_UNTIL.load(Ordering::SeqCst)
}

pub fn light_percent_to_value(code: &str, percent: i32) -> TuyaValue {
  let scale = if code.ends_with("_v2") { 10 } else { 1 };
  TuyaValue::Integer((percent.clamp(1, 100) * scale).clamp(1, 1000) as i64)
}

pub fn parse_fan_speed(value: &TuyaValue) -> i32 {
  if let Some(s) = value.as_string() {
    match s.to_lowercase().trim() {
      "low" | "min" => return 1,
      "mid" | "med" | "medium" => return 3,
      "high" | "max" => return 5,
      _ => {}
    }
  }

  if let Some(v) = value.as_i64() {
    if (1..=5).contains(&v) {
      return v as i32;
    }
    if (6..=100).contains(&v) {
      let level = ((v as f64) / 20.0).round() as i32;
      return level.clamp(1, 5);
    }
  }

  1
}

pub fn parse_ac_fan_speed(value: &TuyaValue) -> i32 {
  if let Some(s) = value.as_string() {
    match s.to_lowercase().trim() {
      "low" | "min" | "quiet" | "mute" => return 1,
      "mid" | "med" | "medium" => return 2,
      "high" | "max" => return 3,
      "auto" => return 4,
      _ => {}
    }
  }

  if let Some(v) = value.as_i64() {
    if (1..=4).contains(&v) {
      return v as i32;
    }
    if (5..=100).contains(&v) {
      let level = ((v as f64) / 25.0).round() as i32;
      return level.clamp(1, 4);
    }
  }

  1
}

pub fn parse_temperature(value: &TuyaValue) -> i32 {
  if let Some(v) = value.as_i64() {
    if (160..=300).contains(&v) {
      return (v / 10) as i32;
    }
    if (TEMP_MIN as i64..=TEMP_MAX as i64).contains(&v) {
      return v as i32;
    }
  }
  20
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_tuya_value_as_i64() {
    assert_eq!(TuyaValue::Integer(3).as_i64(), Some(3));
    assert_eq!(TuyaValue::Float(3.0).as_i64(), Some(3));
    assert_eq!(TuyaValue::String("3".to_string()).as_i64(), Some(3));
    assert_eq!(TuyaValue::String("level_3".to_string()).as_i64(), Some(3));
    assert_eq!(TuyaValue::String("speed_3".to_string()).as_i64(), Some(3));
    assert_eq!(TuyaValue::String("60".to_string()).as_i64(), Some(60));
  }

  #[test]
  fn test_parse_fan_speed() {
    assert_eq!(parse_fan_speed(&TuyaValue::Integer(3)), 3);
    assert_eq!(parse_fan_speed(&TuyaValue::String("3".to_string())), 3);
    assert_eq!(
      parse_fan_speed(&TuyaValue::String("level_3".to_string())),
      3
    );
    assert_eq!(
      parse_fan_speed(&TuyaValue::String("speed_3".to_string())),
      3
    );
    assert_eq!(parse_fan_speed(&TuyaValue::String("mid".to_string())), 3);
    assert_eq!(parse_fan_speed(&TuyaValue::Integer(60)), 3);
    assert_eq!(parse_fan_speed(&TuyaValue::Integer(100)), 5);
    assert_eq!(parse_fan_speed(&TuyaValue::Integer(20)), 1);
  }

  #[test]
  fn test_parse_temperature() {
    assert_eq!(parse_temperature(&TuyaValue::Integer(24)), 24);
    assert_eq!(parse_temperature(&TuyaValue::String("24".to_string())), 24);
    assert_eq!(parse_temperature(&TuyaValue::Integer(240)), 24);
    assert_eq!(parse_temperature(&TuyaValue::String("240".to_string())), 24);
  }
}
