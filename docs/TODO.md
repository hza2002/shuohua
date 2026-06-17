# TODO

## Configuration

- Move overlay visual parameters into a theme system:
  `config.toml` should select `overlay.theme`, while `theme/*.toml` owns glass,
  tint, radius, blur, and related visual parameters.
- Keep current VAD advanced parameters during development. Before release,
  validate defaults with real recordings and hide low-level tuning fields from
  the default user config.
- Add `doctor` reporting for retained audio: total size, file count, oldest and
  newest recording.
- Add TUI history actions for retained audio: play, reveal, delete one item, and
  clean by age or size.
- Add a TUI or overlay-assisted route workflow for assigning the current app
  bundle id to a profile.
