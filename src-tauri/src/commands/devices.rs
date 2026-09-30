use tauri::State;

use crate::config::ConfigManager;
use crate::error::{AppError, CommandResult, SerializableError};
use crate::tuya::{
  light_percent_to_value, mark_light_operation, SharedTuyaClient, TuyaCommand, TuyaDevice,
  TuyaDeviceStatus, TuyaValue, LIGHT_BRIGHTNESS_CODES, LIGHT_SWITCH_CODES, LIGHT_TEMPERATURE_CODES,
};

#[tauri::command]
pub async fn fetch_devices(
  client: State<'_, SharedTuyaClient>,
  config: State<'_, ConfigManager>,
) -> CommandResult<Vec<TuyaDevice>> {
  let guard = client.read().await;
  let tuya_client = guard
    .as_ref()
    .ok_or_else(|| SerializableError::from(AppError::NotConfigured))?;

  let user_id = config.get_user_id().ok_or_else(|| {
    SerializableError::from(AppError::Config("User ID not configured".to_string()))
  })?;

  tuya_client
    .fetch_devices(&user_id)
    .await
    .map_err(SerializableError::from)
}

#[tauri::command]
pub async fn fetch_device_status(
  device_id: String,
  client: State<'_, SharedTuyaClient>,
) -> CommandResult<Vec<TuyaDeviceStatus>> {
  let guard = client.read().await;
  let tuya_client = guard
    .as_ref()
    .ok_or_else(|| SerializableError::from(AppError::NotConfigured))?;

  tuya_client
    .fetch_device_status(&device_id)
    .await
    .map_err(SerializableError::from)
}

#[tauri::command]
pub async fn send_device_command(
  device_id: String,
  code: String,
  value: serde_json::Value,
  client: State<'_, SharedTuyaClient>,
) -> CommandResult<bool> {
  let guard = client.read().await;
  let tuya_client = guard
    .as_ref()
    .ok_or_else(|| SerializableError::from(AppError::NotConfigured))?;

  let tuya_value = match value {
    serde_json::Value::Bool(b) => TuyaValue::Boolean(b),
    serde_json::Value::String(s) => TuyaValue::String(s),
    serde_json::Value::Number(n) => {
      if let Some(i) = n.as_i64() {
        TuyaValue::Integer(i)
      } else if let Some(f) = n.as_f64() {
        TuyaValue::Float(f)
      } else {
        return Err(SerializableError {
          error_type: "parse".to_string(),
          message: "Invalid number value".to_string(),
          code: None,
        });
      }
    }
    _ => {
      return Err(SerializableError {
        error_type: "parse".to_string(),
        message: "Unsupported value type".to_string(),
        code: None,
      });
    }
  };

  tuya_client
    .send_device_command(&device_id, &code, tuya_value)
    .await
    .map_err(SerializableError::from)
}

#[tauri::command]
pub async fn toggle_device_state(
  device_id: String,
  code: String,
  current_value: bool,
  client: State<'_, SharedTuyaClient>,
) -> CommandResult<bool> {
  let guard = client.read().await;
  let tuya_client = guard
    .as_ref()
    .ok_or_else(|| SerializableError::from(AppError::NotConfigured))?;

  tuya_client
    .toggle_device_state(&device_id, &code, current_value)
    .await
    .map_err(SerializableError::from)
}

#[tauri::command]
pub async fn set_light_settings(
  device_id: String,
  brightness: i32,
  temperature: i32,
  power: bool,
  client: State<'_, SharedTuyaClient>,
) -> CommandResult<bool> {
  mark_light_operation();
  let guard = client.read().await;
  let tuya_client = guard
    .as_ref()
    .ok_or_else(|| SerializableError::from(AppError::NotConfigured))?;
  let statuses = tuya_client
    .fetch_device_status(&device_id)
    .await
    .map_err(SerializableError::from)?;
  let brightness_status = statuses
    .iter()
    .find(|s| LIGHT_BRIGHTNESS_CODES.contains(&s.code.as_str()))
    .ok_or_else(|| SerializableError {
      error_type: "unsupported".to_string(),
      message: "Brightness is not supported by this device".to_string(),
      code: None,
    })?;
  let temperature_status = statuses
    .iter()
    .find(|s| LIGHT_TEMPERATURE_CODES.contains(&s.code.as_str()))
    .ok_or_else(|| SerializableError {
      error_type: "unsupported".to_string(),
      message: "White temperature is not supported by this device".to_string(),
      code: None,
    })?;
  let settings = vec![
    TuyaCommand {
      code: brightness_status.code.clone(),
      value: light_percent_to_value(&brightness_status.code, brightness.clamp(1, 100)),
    },
    TuyaCommand {
      code: temperature_status.code.clone(),
      value: light_percent_to_value(&temperature_status.code, temperature.clamp(1, 100)),
    },
  ];
  let expected_brightness = settings[0].value.clone();
  let expected_temperature = settings[1].value.clone();
  tuya_client
    .send_device_commands(&device_id, settings)
    .await
    .map_err(SerializableError::from)?;

  for _ in 0..15 {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let current = tuya_client
      .fetch_device_status(&device_id)
      .await
      .map_err(SerializableError::from)?;
    let brightness_ready = current
      .iter()
      .find(|s| LIGHT_BRIGHTNESS_CODES.contains(&s.code.as_str()))
      .map(|s| s.value == expected_brightness)
      .unwrap_or(false);
    let temperature_ready = current
      .iter()
      .find(|s| LIGHT_TEMPERATURE_CODES.contains(&s.code.as_str()))
      .map(|s| s.value == expected_temperature)
      .unwrap_or(false);
    if brightness_ready && temperature_ready {
      break;
    }
  }

  let switch = statuses
    .iter()
    .find(|s| LIGHT_SWITCH_CODES.contains(&s.code.as_str()));
  if let Some(switch) = switch {
    if switch.value.as_bool().unwrap_or(false) != power {
      tuya_client
        .send_device_command(&device_id, &switch.code, TuyaValue::Boolean(power))
        .await
        .map_err(SerializableError::from)?;
    }
  }
  Ok(true)
}
