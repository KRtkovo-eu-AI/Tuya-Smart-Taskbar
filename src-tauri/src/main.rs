#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{
  image::Image, menu::MenuEvent, tray::TrayIconBuilder, AppHandle, Manager, RunEvent, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{Mutex, RwLock};

use tuya_smart_taskbar::{
  commands,
  config::{set_auto_launch, ConfigManager},
  tray::{self, MenuItemRegistry},
  tuya::{
    create_shared_client, initialize_client, light_operation_active, light_percent_to_value,
    mark_light_operation, SharedTuyaClient, TuyaCommand, TuyaDeviceStatus, TuyaValue,
    LIGHT_BRIGHTNESS_CODES, LIGHT_SWITCH_CODES, LIGHT_TEMPERATURE_CODES,
  },
  update::{self, create_update_state, SharedUpdateState},
};

static RUNNING: AtomicBool = AtomicBool::new(false);
static MENU_INTERACTION_TIME: AtomicI64 = AtomicI64::new(0);
static UPDATE_CHECK_COUNTER: AtomicU64 = AtomicU64::new(0);

type DeviceStatusCache = Arc<RwLock<HashMap<String, Vec<TuyaDeviceStatus>>>>;
type MenuUpdateLock = Arc<Mutex<()>>;

const ICON_BYTES: &[u8] = include_bytes!("../icons/icon.ico");
const LOADING_ICON_BYTES: &[u8] = include_bytes!("../icons/loading.ico");
const UPDATE_ICON_BYTES: &[u8] = include_bytes!("../icons/update.ico");
const UPDATE_CHECK_INTERVAL: u64 = 360;

async fn update_tray_menu(
  app: &AppHandle,
  is_auto_refresh: bool,
  status_cache: &DeviceStatusCache,
  menu_lock: &MenuUpdateLock,
  update_state: &SharedUpdateState,
  menu_registry: &MenuItemRegistry,
) {
  if is_auto_refresh {
    if light_operation_active() {
      tracing::debug!("Skipping auto-refresh: light operation in progress");
      return;
    }
    let last_interaction = MENU_INTERACTION_TIME.load(Ordering::SeqCst);
    let now = chrono::Utc::now().timestamp_millis();
    if now - last_interaction < 2000 {
      tracing::debug!("Skipping auto-refresh: recent menu interaction");
      return;
    }
  }

  let _guard = if is_auto_refresh {
    match tokio::time::timeout(Duration::from_millis(100), menu_lock.lock()).await {
      Ok(guard) => guard,
      Err(_) => {
        tracing::debug!("Skipping auto-refresh: menu update in progress");
        return;
      }
    }
  } else {
    menu_lock.lock().await
  };

  let config_manager = app.state::<ConfigManager>();
  let client = app.state::<SharedTuyaClient>();

  if !is_auto_refresh {
    if let Some(tray) = app.tray_by_id("main") {
      if let Ok(icon) = Image::from_bytes(LOADING_ICON_BYTES) {
        let _ = tray.set_icon(Some(icon));
      }
    }
  }

  if !config_manager.is_configured() {
    let menu = tray::build_unconfigured_menu(app, update_state).await;
    if let Ok(menu) = menu {
      if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_menu(Some(menu));
      }
    }
    {
      let mut registry = menu_registry.write().await;
      registry.clear();
      let mut cache = status_cache.write().await;
      cache.clear();
    }
    if !is_auto_refresh {
      restore_tray_icon(app, update_state).await;
    }
    return;
  }

  match tray::fetch_device_statuses(&client, &config_manager).await {
    Ok((devices, new_statuses)) => {
      let old_cache = status_cache.read().await.clone();

      if is_auto_refresh
        && !old_cache.is_empty()
        && !tray::is_structural_change(&old_cache, &new_statuses)
      {
        if old_cache != new_statuses {
          let registry = menu_registry.read().await;
          let updated = tray::update_menu_items_in_place(&registry, &old_cache, &new_statuses);
          tracing::debug!("In-place update: {} items changed", updated);
        }
        let mut cache = status_cache.write().await;
        *cache = new_statuses;
      } else {
        match tray::build_device_menu(app, &devices, &new_statuses, update_state).await {
          Ok((menu, new_registry_entries)) => {
            if let Some(tray) = app.tray_by_id("main") {
              let _ = tray.set_menu(Some(menu));
            }
            {
              let mut registry = menu_registry.write().await;
              *registry = new_registry_entries;
            }
            let mut cache = status_cache.write().await;
            *cache = new_statuses;
          }
          Err(e) => {
            tracing::error!("Error building device menu: {}", e);
            if let Ok(menu) = tray::build_error_menu(app, update_state).await {
              if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(Some(menu));
              }
            }
            let mut registry = menu_registry.write().await;
            registry.clear();
          }
        }
      }
    }
    Err(e) => {
      tracing::error!("Error fetching device statuses: {}", e);
      if !is_auto_refresh {
        if let Ok(menu) = tray::build_error_menu(app, update_state).await {
          if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
          }
        }
        let mut registry = menu_registry.write().await;
        registry.clear();
      }
    }
  }

  if !is_auto_refresh {
    restore_tray_icon(app, update_state).await;
  }
}

async fn restore_tray_icon(app: &AppHandle, update_state: &SharedUpdateState) {
  let (has_update, latest_version) = {
    let guard = update_state.read().await;
    (guard.update_available, guard.latest_version.clone())
  };
  if let Some(tray) = app.tray_by_id("main") {
    let icon_bytes = if has_update {
      UPDATE_ICON_BYTES
    } else {
      ICON_BYTES
    };
    if let Ok(icon) = Image::from_bytes(icon_bytes) {
      let _ = tray.set_icon(Some(icon));
    }
    let tooltip = if has_update {
      if let Some(ref version) = latest_version {
        format!("Tuya Smart Taskbar - Update Available (v{})", version)
      } else {
        "Tuya Smart Taskbar - Update Available".to_string()
      }
    } else {
      "Tuya Smart Taskbar".to_string()
    };
    let _ = tray.set_tooltip(Some(&tooltip));
  }
}

async fn check_and_notify_update(
  app: &AppHandle,
  update_state: &SharedUpdateState,
  status_cache: Option<&DeviceStatusCache>,
  menu_lock: Option<&MenuUpdateLock>,
  menu_registry: Option<&MenuItemRegistry>,
) -> bool {
  tracing::debug!("Checking for updates...");

  if let Some(update_info) = update::check_for_update(app).await {
    tracing::debug!(
      "Update check result: current={}, latest={}, available={}",
      update_info.current_version,
      update_info.latest_version,
      update_info.available
    );

    let (is_new, should_notify) = update::update_state(update_state, &update_info).await;

    if is_new {
      tracing::info!(
        "Update available: {} -> {}",
        update_info.current_version,
        update_info.latest_version
      );

      if let Some(tray) = app.tray_by_id("main") {
        if let Ok(icon) = Image::from_bytes(UPDATE_ICON_BYTES) {
          let _ = tray.set_icon(Some(icon));
        }
        let tooltip = format!(
          "Tuya Smart Taskbar - Update Available (v{})",
          update_info.latest_version
        );
        let _ = tray.set_tooltip(Some(&tooltip));
      }

      if let (Some(cache), Some(lock), Some(reg)) = (status_cache, menu_lock, menu_registry) {
        tracing::debug!("Rebuilding menu to show update indicator");
        update_tray_menu(app, false, cache, lock, update_state, reg).await;
      }
    }

    if should_notify {
      tracing::info!("Sending update notification");
      match app
        .notification()
        .builder()
        .title("Tuya Smart Taskbar Update Available")
        .body(format!(
          "Version {} is now available. Click the tray icon to update.",
          update_info.latest_version
        ))
        .show()
      {
        Ok(_) => tracing::debug!("Notification sent successfully"),
        Err(e) => tracing::error!("Failed to send notification: {}", e),
      }
    }

    return is_new;
  } else {
    tracing::debug!("Update check failed or returned no info");
  }

  false
}

fn handle_menu_event(
  app: &AppHandle,
  event: MenuEvent,
  status_cache: DeviceStatusCache,
  menu_lock: MenuUpdateLock,
  update_state: SharedUpdateState,
  menu_registry: MenuItemRegistry,
) {
  MENU_INTERACTION_TIME.store(chrono::Utc::now().timestamp_millis(), Ordering::SeqCst);

  let id = event.id().as_ref();
  tracing::debug!("Menu event: {}", id);

  match id {
    "open_config" => {
      open_config_window(app);
    }
    "open_about" => {
      open_about_window(app);
    }
    _ if id.starts_with("custom_light:") => {
      if let Some(rest) = id.strip_prefix("custom_light:") {
        let mut parts = rest.splitn(2, ':');
        if let (Some(device_id), Some(encoded_name)) = (parts.next(), parts.next()) {
          let device_name = urlencoding::decode(encoded_name)
            .map(|name| name.into_owned())
            .unwrap_or_else(|_| "Custom lighting".to_string());
          open_light_window(app, device_id, &device_name);
        }
      }
    }
    "open_update" => {
      let _ = open::that(update::get_download_url());
    }
    "refresh" => {
      let app_handle = app.clone();
      let cache = status_cache.clone();
      let lock = menu_lock.clone();
      let update_st = update_state.clone();
      let registry = menu_registry.clone();
      tauri::async_runtime::spawn(async move {
        update_tray_menu(&app_handle, false, &cache, &lock, &update_st, &registry).await;
      });
    }
    "quit" => {
      RUNNING.store(false, Ordering::Release);
      app.exit(0);
    }
    _ if id.starts_with("toggle:") => {
      if let Some((device_id, code, _)) = tray::parse_command_id(id) {
        let app_handle = app.clone();
        let cache = status_cache.clone();
        let registry = menu_registry.clone();

        tauri::async_runtime::spawn(async move {
          let current = {
            let cache_guard = cache.read().await;
            cache_guard
              .get(&device_id)
              .and_then(|statuses| {
                statuses
                  .iter()
                  .find(|s| s.code == code)
                  .and_then(|s| s.value.as_bool())
              })
              .unwrap_or(false)
          };

          let result = {
            let client = app_handle.state::<SharedTuyaClient>();
            let guard = client.read().await;
            if let Some(tuya_client) = guard.as_ref() {
              Some(
                tuya_client
                  .toggle_device_state(&device_id, &code, current)
                  .await,
              )
            } else {
              None
            }
          };

          match result {
            Some(Ok(_)) => {
              let new_value = TuyaValue::Boolean(!current);
              tracing::info!("Toggled {}:{} (was {})", device_id, code, current);
              let reg = registry.read().await;
              let mut cache_guard = cache.write().await;
              tray::update_single_device_status_in_place(
                &reg,
                &mut cache_guard,
                &device_id,
                &code,
                &new_value,
              );
            }
            Some(Err(e)) => {
              tracing::error!("Failed to toggle: {}", e);
            }
            None => {
              tracing::error!("Client not initialized");
            }
          }
        });
      }
    }
    _ if id.starts_with("set:") || id.starts_with("cmd:") => {
      if let Some((device_id, code, value_str)) = tray::parse_command_id(id) {
        if code == "scene" {
          let percentages: Vec<i32> = value_str
            .split(':')
            .filter_map(|v| v.parse().ok())
            .collect();
          if percentages.len() == 2 {
            let app_handle = app.clone();
            let cache = status_cache.clone();
            let registry = menu_registry.clone();
            mark_light_operation();
            tauri::async_runtime::spawn(async move {
              let statuses = cache
                .read()
                .await
                .get(&device_id)
                .cloned()
                .unwrap_or_default();
              let brightness = statuses
                .iter()
                .find(|s| LIGHT_BRIGHTNESS_CODES.contains(&s.code.as_str()));
              let temperature = statuses
                .iter()
                .find(|s| LIGHT_TEMPERATURE_CODES.contains(&s.code.as_str()));
              if let (Some(brightness), Some(temperature)) = (brightness, temperature) {
                let settings = vec![
                  TuyaCommand {
                    code: brightness.code.clone(),
                    value: light_percent_to_value(&brightness.code, percentages[0]),
                  },
                  TuyaCommand {
                    code: temperature.code.clone(),
                    value: light_percent_to_value(&temperature.code, percentages[1]),
                  },
                ];
                let switch = statuses
                  .iter()
                  .find(|s| LIGHT_SWITCH_CODES.contains(&s.code.as_str()));
                let client = app_handle.state::<SharedTuyaClient>();
                let guard = client.read().await;
                if let Some(tuya_client) = guard.as_ref() {
                  let result = match tuya_client
                    .send_device_commands(&device_id, settings.clone())
                    .await
                  {
                    Ok(_) => {
                      if let Some(switch) = switch {
                        if !switch.value.as_bool().unwrap_or(false) {
                          tuya_client
                            .send_device_command(&device_id, &switch.code, TuyaValue::Boolean(true))
                            .await
                        } else {
                          Ok(true)
                        }
                      } else {
                        Ok(true)
                      }
                    }
                    Err(e) => Err(e),
                  };
                  match result {
                    Ok(_) => {
                      let reg = registry.read().await;
                      let mut cache_guard = cache.write().await;
                      if let Some(switch) = switch {
                        tray::update_single_device_status_in_place(
                          &reg,
                          &mut cache_guard,
                          &device_id,
                          &switch.code,
                          &TuyaValue::Boolean(true),
                        );
                      }
                      for command in settings {
                        tray::update_single_device_status_in_place(
                          &reg,
                          &mut cache_guard,
                          &device_id,
                          &command.code,
                          &command.value,
                        );
                      }
                    }
                    Err(e) => tracing::error!("Failed to apply lighting scene: {}", e),
                  }
                }
              }
            });
          }
          return;
        }
        let value = tray::parse_value(&value_str);
        if LIGHT_BRIGHTNESS_CODES.contains(&code.as_str())
          || LIGHT_TEMPERATURE_CODES.contains(&code.as_str())
        {
          let app_handle = app.clone();
          let cache = status_cache.clone();
          let registry = menu_registry.clone();
          mark_light_operation();
          tauri::async_runtime::spawn(async move {
            let statuses = cache
              .read()
              .await
              .get(&device_id)
              .cloned()
              .unwrap_or_default();
            let mut commands = vec![TuyaCommand {
              code: code.clone(),
              value: value.clone(),
            }];
            if let Some(switch) = statuses
              .iter()
              .find(|s| LIGHT_SWITCH_CODES.contains(&s.code.as_str()))
            {
              if !switch.value.as_bool().unwrap_or(false) {
                commands.push(TuyaCommand {
                  code: switch.code.clone(),
                  value: TuyaValue::Boolean(true),
                });
              }
            }
            let client = app_handle.state::<SharedTuyaClient>();
            let guard = client.read().await;
            if let Some(tuya_client) = guard.as_ref() {
              match tuya_client
                .send_device_commands(&device_id, commands.clone())
                .await
              {
                Ok(_) => {
                  let reg = registry.read().await;
                  let mut cache_guard = cache.write().await;
                  for command in commands {
                    tray::update_single_device_status_in_place(
                      &reg,
                      &mut cache_guard,
                      &device_id,
                      &command.code,
                      &command.value,
                    );
                  }
                }
                Err(e) => tracing::error!("Failed to apply light setting: {}", e),
              }
            }
          });
          return;
        }
        let app_handle = app.clone();
        let cache = status_cache.clone();
        let registry = menu_registry.clone();

        tauri::async_runtime::spawn(async move {
          let result = {
            let client = app_handle.state::<SharedTuyaClient>();
            let guard = client.read().await;
            if let Some(tuya_client) = guard.as_ref() {
              Some(
                tuya_client
                  .send_device_command(&device_id, &code, value.clone())
                  .await,
              )
            } else {
              None
            }
          };

          match result {
            Some(Ok(_)) => {
              tracing::info!("Command sent: {}:{}", code, value_str);
              let reg = registry.read().await;
              let mut cache_guard = cache.write().await;
              tray::update_single_device_status_in_place(
                &reg,
                &mut cache_guard,
                &device_id,
                &code,
                &value,
              );
            }
            Some(Err(e)) => {
              tracing::error!("Failed to send command: {}", e);
            }
            None => {
              tracing::error!("Client not initialized");
            }
          }
        });
      }
    }
    _ => {}
  }
}

fn open_light_window(app: &AppHandle, device_id: &str, device_name: &str) {
  let label = format!("custom_light_{}", device_id);
  if let Some(window) = app.get_webview_window(&label) {
    let _ = window.show();
    let _ = window.set_focus();
    return;
  }
  let url = format!(
    "pages/light.html?device_id={}&device_name={}",
    device_id,
    urlencoding::encode(device_name),
  );
  let window = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App(url.into()))
    .title(device_name)
    .inner_size(420.0, 430.0)
    .resizable(false)
    .center()
    .visible(true)
    .build();
  if let Err(e) = window {
    tracing::error!("Failed to create lighting window: {}", e);
  }
}

fn open_config_window(app: &AppHandle) {
  if let Some(window) = app.get_webview_window("config") {
    let _ = window.show();
    let _ = window.set_focus();
  } else {
    let config_window = tauri::WebviewWindowBuilder::new(
      app,
      "config",
      tauri::WebviewUrl::App("pages/config.html".into()),
    )
    .title("Tuya Smart Taskbar - Configuration")
    .inner_size(400.0, 660.0)
    .resizable(false)
    .center()
    .visible(true)
    .build();

    if let Err(e) = config_window {
      tracing::error!("Failed to create config window: {}", e);
    }
  }
}

fn open_about_window(app: &AppHandle) {
  if let Some(window) = app.get_webview_window("about") {
    let _ = window.show();
    let _ = window.set_focus();
  } else {
    let about_window = tauri::WebviewWindowBuilder::new(
      app,
      "about",
      tauri::WebviewUrl::App("pages/about.html".into()),
    )
    .title("About Tuya Smart Taskbar")
    .inner_size(400.0, 590.0)
    .resizable(false)
    .center()
    .visible(true)
    .build();

    if let Err(e) = about_window {
      tracing::error!("Failed to create about window: {}", e);
    }
  }
}

fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()),
    )
    .init();

  tracing::info!("Starting Tuya Smart Taskbar v{}", env!("CARGO_PKG_VERSION"));

  let config_manager = ConfigManager::new();
  let shared_client = create_shared_client();

  if config_manager.is_configured() {
    let cfg = config_manager.get();
    let client_clone = shared_client.clone();

    tauri::async_runtime::block_on(async {
      initialize_client(
        &client_clone,
        cfg.access_key.clone(),
        cfg.secret_key.clone(),
        cfg.base_url.clone(),
      )
      .await;
    });

    tracing::info!("Tuya client initialized");
  }

  let cfg = config_manager.get();
  if let Err(e) = set_auto_launch(cfg.run_on_startup) {
    tracing::warn!("Failed to set auto-launch: {}", e);
  }

  let status_cache: DeviceStatusCache = Arc::new(RwLock::new(HashMap::new()));
  let menu_update_lock: MenuUpdateLock = Arc::new(Mutex::new(()));
  let update_state: SharedUpdateState = create_update_state();
  let menu_registry: MenuItemRegistry = tray::create_menu_registry();

  let app = tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
      tracing::info!("Second instance detected, focusing existing window");
      open_config_window(app);
    }))
    .manage(shared_client.clone())
    .manage(config_manager)
    .manage(status_cache.clone())
    .manage(update_state.clone())
    .invoke_handler(tauri::generate_handler![
      commands::config::save_config,
      commands::config::get_config,
      commands::config::is_configured,
      commands::config::get_regions,
      commands::devices::fetch_devices,
      commands::devices::fetch_device_status,
      commands::devices::send_device_command,
      commands::devices::set_light_settings,
      commands::devices::toggle_device_state,
      commands::app::get_version,
      commands::app::check_for_update,
      commands::app::open_external,
    ])
    .setup(move |app| {
      let icon = Image::from_bytes(ICON_BYTES).expect("Failed to load tray icon");

      let update_state_for_menu = update_state.clone();
      let initial_menu = tauri::async_runtime::block_on(async {
        tray::build_unconfigured_menu(app.handle(), &update_state_for_menu).await
      })
      .expect("Failed to create initial menu");

      let status_cache_for_event = status_cache.clone();
      let menu_lock_for_event = menu_update_lock.clone();
      let update_state_for_event = update_state.clone();
      let registry_for_event = menu_registry.clone();
      let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Tuya Smart Taskbar")
        .menu(&initial_menu)
        .on_menu_event(move |app, event| {
          handle_menu_event(
            app,
            event,
            status_cache_for_event.clone(),
            menu_lock_for_event.clone(),
            update_state_for_event.clone(),
            registry_for_event.clone(),
          );
        })
        .build(app)?;

      let app_handle = app.handle().clone();
      let cache_for_init = status_cache.clone();
      let lock_for_init = menu_update_lock.clone();
      let update_state_for_init = update_state.clone();
      let registry_for_init = menu_registry.clone();
      tauri::async_runtime::spawn(async move {
        update_tray_menu(
          &app_handle,
          false,
          &cache_for_init,
          &lock_for_init,
          &update_state_for_init,
          &registry_for_init,
        )
        .await;
      });

      let app_handle = app.handle().clone();
      let cache_for_startup = status_cache.clone();
      let lock_for_startup = menu_update_lock.clone();
      let update_state_for_startup = update_state.clone();
      let registry_for_startup = menu_registry.clone();
      tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        check_and_notify_update(
          &app_handle,
          &update_state_for_startup,
          Some(&cache_for_startup),
          Some(&lock_for_startup),
          Some(&registry_for_startup),
        )
        .await;
      });

      RUNNING.store(true, Ordering::Release);
      let app_handle = app.handle().clone();
      let cache_for_loop = status_cache.clone();
      let lock_for_loop = menu_update_lock.clone();
      let update_state_for_loop = update_state.clone();
      let registry_for_loop = menu_registry.clone();
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
          interval.tick().await;
          if !RUNNING.load(Ordering::Acquire) {
            break;
          }

          let counter = UPDATE_CHECK_COUNTER.fetch_add(1, Ordering::Relaxed);
          if counter > 0 && counter.is_multiple_of(UPDATE_CHECK_INTERVAL) {
            check_and_notify_update(
              &app_handle,
              &update_state_for_loop,
              Some(&cache_for_loop),
              Some(&lock_for_loop),
              Some(&registry_for_loop),
            )
            .await;
          }

          let result = tokio::time::timeout(
            Duration::from_secs(15),
            update_tray_menu(
              &app_handle,
              true,
              &cache_for_loop,
              &lock_for_loop,
              &update_state_for_loop,
              &registry_for_loop,
            ),
          )
          .await;

          if result.is_err() {
            tracing::warn!("Auto-refresh timed out, will retry next cycle");
          }
        }
        tracing::info!("Auto-refresh loop terminated");
      });

      tracing::info!("Application setup complete");
      Ok(())
    })
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { api, .. } = event {
        let _ = window.hide();
        api.prevent_close();
      }
    })
    .build(tauri::generate_context!())
    .expect("Error while building tauri application");

  app.run(|_app_handle, event| {
    if let RunEvent::ExitRequested { api, .. } = event {
      if RUNNING.load(Ordering::Acquire) {
        api.prevent_exit();
      }
    }
  });
}
