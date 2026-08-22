# Tuya Smart Taskbar

System tray app for controlling Tuya smart home devices. Built with Tauri v2 (Rust backend) + vanilla HTML/CSS/JS frontend.

## Commands

```bash
pnpm install          # Install dependencies (Tauri CLI)
pnpm dev              # Development mode with hot reload
pnpm build            # Production build (output: src-tauri/target/release/)
```

```bash
pnpm lint             # Biome lint check (frontend/JSON)
pnpm format           # Biome auto-format
pnpm validate         # Full check: biome + cargo fmt + clippy
pnpm lint:rust        # Cargo clippy only
```

Linux setup: `scripts/setup-xubuntu.sh` installs system deps, Node.js, pnpm, Rust.

## Architecture

```
frontend/pages/         # HTML pages served by Tauri (no build step)
  config.html           # Configuration form (credentials, region)
  about.html            # About/version/update checker
src-tauri/src/          # Rust backend
  main.rs               # Entry point, tray icon, menu events, auto-refresh loop (10s)
  lib.rs                # Module declarations
  error.rs              # Custom error types (AppError -> serializable for frontend)
  config/manager.rs     # JSON config I/O, auto-launch, region definitions
  tuya/client.rs        # HTTP client: device fetch, status poll, command send (3-retry backoff)
  tuya/auth.rs          # HMAC-SHA256 request signing
  tuya/token.rs         # Token acquire/refresh (5-min-before-expiry, rate-limited)
  tuya/types.rs         # TuyaDevice, TuyaValue, TuyaDeviceStatus, TokenState
  tray/menu.rs          # Dynamic menu generation from device capabilities
  commands/config.rs    # Tauri IPC: save_config, get_config, is_configured, get_regions
  commands/devices.rs   # Tauri IPC: fetch_devices, fetch_device_status, send/toggle
  commands/app.rs       # Tauri IPC: get_version, check_for_update, open_external
  update.rs             # Background update checks (hourly, GitHub manifest)
scripts/
  setup-xubuntu.sh      # Linux dependency installer
```

## Key Patterns

- **Shared state**: `Arc<RwLock<T>>` for TuyaClient, ConfigManager, DeviceStatusCache, UpdateState. `Mutex` for MenuUpdateLock.
- **Frontend IPC**: `window.__TAURI__.core.invoke("command_name", { args })` - no JS framework.
- **Menu updates**: Two-path auto-refresh — status-only changes update `CheckMenuItem` states in-place via `set_checked()` (menu stays open); structural changes (device online/offline) trigger full rebuild. Uses `MenuItemRegistry` (`Arc<RwLock<HashMap>>`) to hold item references.
- **Menu item IDs**: Booleans use `toggle:{deviceId}:{code}` (handler reads current state from cache). Enums use `set:{deviceId}:{code}:{value}`.
- **Auto-refresh**: 10s interval, skips if menu was interacted within 2s (prevents UI flicker), 100ms lock timeout to avoid blocking.
- **Token management**: Auto-refresh 5 min before expiry; 5 consecutive failures trigger 60s cooldown.
- **HTTP retry**: Exponential backoff (500ms -> 1s -> 2s), 3 retries, 30s request timeout.
- **API code 1010**: Means token invalid - triggers re-authentication.

## Config

Runtime config stored at `{platform_config_dir}/Tuya Smart Taskbar/config.json`:

- Windows: `%LOCALAPPDATA%/Tuya Smart Taskbar/config.json`
- Linux: `~/.config/Tuya Smart Taskbar/config.json` (via `directories` crate)

Fields: `baseUrl`, `accessKey`, `secretKey`, `userId`, `runOnStartup`

## Platform Notes

- **Windows-specific**: `windows` crate dependency for `Win32_System_Threading` (mutex for single instance). Icon format: `.ico`.
- **Linux-specific**: Requires GTK3, WebKit2GTK, Ayatana AppIndicator. Icon format: `.png`. Auto-launch not yet implemented.
- **Bundle targets**: Windows (MSI, NSIS), Linux (AppImage, Deb, RPM) - configured in `src-tauri/tauri.conf.json`.

## Code Style

- Rust: 2021 edition, async/await with Tokio, `thiserror` for error types, `tracing` for logging.
- Frontend: Vanilla HTML with inline `<style>` and `<script>`. Dark theme via `prefers-color-scheme`. No build tooling.
- Release profile: `panic = "abort"`, LTO enabled, optimized for size (`opt-level = "s"`). `strip = false` due to AppImage bundling issue (tauri-apps/tauri#14796).
- Linting: [Biome](https://biomejs.dev/) for frontend (2 spaces, single quotes). Clippy for Rust (`-D warnings`). Formatting: 2 spaces for both Rust (`rustfmt.toml`) and frontend/JSON (`biome.json`).
- CI: GitHub Actions — `ci.yml` (lint + test on push) and `release.yml` (manual dispatch, builds Windows + Linux, creates GitHub Release).

## Gotchas

- `src-tauri/gen/schemas/` files are **auto-generated** by Tauri - don't edit manually.
- The `image-ico` Tauri feature is required for Windows tray icon embedding.
- CSP in `tauri.conf.json` allows Google Fonts and `unsafe-inline` styles - intentional for the config/about pages.
- Tests exist only in `tuya/auth.rs` (signature validation). No integration test suite.
- `strip = false` in release profile is intentional — `strip = true` breaks AppImage bundling on Linux (tauri-apps/tauri#14796). Do not re-enable.
- AppImage builds fail on GitHub Actions CI — release workflow builds deb/rpm only. AppImage can be built locally on Linux.
