# App Data Ownership

## Goal

CLI, daemon, GUI, and packaged desktop app entries must share one product data model. Packaging may add
app-private storage, but it must not create a second copy of user config, history, retained audio, or logs.

This document defines the long-term path ownership model for macOS, Windows, and Linux. Platform-specific
documents may refine exact APIs and validation commands, but they should not contradict this ownership model.

## Ownership Model

Shuohua uses three path classes:

| Class | Owner | Contents | Shared by CLI/daemon/GUI? |
|---|---|---|---|
| Install root | installer/package | executable, app bundle, resources | No user data. |
| Product data root | shuohua product | config, profiles, ASR/post configs, history, retained audio, logs, traces, daemon state | Yes. |
| App-private data | packaged GUI/runtime | window state, WebView cache, onboarding state, tray UI preference, package runtime cache | No, unless explicitly exported. |

Rules:

- Product data is the durable user contract. CLI, daemon, TUI, GUI, and tray must resolve the same product data
  roots by default.
- Packaged app data is not the default source of product data because CLI and daemon must keep working outside
  the package container.
- Do not split one product data kind across two roots. If history lives in product local state, every entry
  reads and writes that same history root.
- Install roots never store mutable user data.
- Cache/temp data can be app-private only when losing it does not affect product behavior.
- If a future store/sandbox package cannot access the product data root directly, provide an explicit
  import/export or migration flow. Do not silently fork product state.

## macOS Baseline

Current macOS CLI behavior may continue to use terminal-friendly config paths such as `~/.config/shuohua`.
Future `.app` packaging should share the same product data instead of starting over inside the app bundle or a
separate GUI-only root.

Recommended long-term stance:

- Config remains terminal-friendly by default. Keeping `~/.config/shuohua` is acceptable for the product
  contract because many CLI users expect it and the current macOS version already uses that style.
- App-private cache/state may use macOS app conventions such as `~/Library/Caches/Shuohua` or an app container
  when the data only belongs to GUI/runtime presentation.
- If a future sandboxed macOS package needs a container, treat it as a packaging mode with explicit migration or
  import/export, not as an implicit new default for product config/history.

## Windows Baseline

Windows should follow per-user known-folder conventions for product data:

| Product data | Root |
|---|---|
| Config, profiles, ASR/post configs | `%APPDATA%\Shuohua\` |
| State, history, retained audio, logs, traces, cache | `%LOCALAPPDATA%\Shuohua\` |

Windows packaged app data remains app-private by default:

- MSIX/package-local data can store GUI runtime state, WebView cache, onboarding state, and package runtime
  cache.
- CLI/daemon shared config and history must not move into package-private data by default.
- If package identity or store policy later restricts direct access, add an explicit migration/import/export
  flow before changing the root.

## Linux Baseline

Linux keeps the XDG split:

| Product data | Root |
|---|---|
| Config, profiles, ASR/post configs | `$XDG_CONFIG_HOME/shuohua` or `~/.config/shuohua` |
| State/history/audio/logs/traces | `$XDG_STATE_HOME/shuohua` or `~/.local/state/shuohua` |
| Rebuildable cache | `$XDG_CACHE_HOME/shuohua` or `~/.cache/shuohua` |

Flatpak/Snap/AppImage packaging must follow the same ownership rule: package-private storage is not a second
product data truth source unless the package sandbox forces an explicit import/export model.

## Implementation Boundary

Path resolution should converge behind a small product path facade before Windows runtime validation:

```text
AppPaths
  config_root
  state_root
  history_dir
  audio_dir
  logs_dir
  traces_dir
  cache_dir
  app_private_dir (optional, GUI/package only)
```

Business modules should request paths through this facade. They should not directly read `HOME`, XDG variables,
`%APPDATA%`, `%LOCALAPPDATA%`, package container paths, or app bundle paths.

Migration rules:

- First prefer existing roots when they already contain user config/history and remain valid for the current
  packaging mode.
- Never auto-copy large or sensitive data such as retained audio without an explicit migration decision.
- Show path decisions in doctor/TUI/GUI diagnostics so users can see which roots are active.
- Any future root migration must document source, destination, rollback, duplicate handling, and user-visible
  prompts before implementation.
