# shuohua icon assets

This directory owns the project logo and app icon assets.

## Design

The mark means "speech becomes typed input": five voice waveform bars lead into
a text insertion cursor. The style is a dark macOS app icon with a Gruvbox-like
palette: warm orange/yellow waveform, aqua cursor, and a quiet warm-charcoal
background.

`shuohua-icon.svg` is the canonical source asset. It uses simple vector shapes,
controlled gradients, and deterministic glow effects.

## Rendering

Generate derived assets from the SVG with the local render script:

```bash
assets/icon/render.sh
```

The script writes:

- `shuohua-icon-1024.png`: canonical raster export for places that need PNG
- `macos/shuohua.icns`: icon for future `.app` bundles

The script uses the first available SVG renderer:

1. `rsvg-convert`
2. `resvg`
3. ImageMagick `magick`

On macOS, `iconutil` is used to build the `.icns` file.

## Maintenance

Do not edit generated PNG or ICNS files by hand. Update `shuohua-icon.svg`, run
`assets/icon/render.sh`, and review the generated `1024` PNG and `.icns`.

The GitHub README uses `shuohua-icon.svg` directly, so it stays sharp at any
display size and does not need a separate README PNG. Keep one SVG source, one
large PNG fallback, and one macOS `.icns` bundle asset; avoid maintaining
multiple hand-sized PNG copies unless a target platform explicitly requires
them.

Keep icon-specific tooling in this directory so the logo pipeline stays local to
the assets it maintains.

## macOS privacy settings

The current release is a command-line binary, so macOS privacy panes may not show
this custom icon for microphone or accessibility permissions. To make Finder,
Launchpad, and privacy settings consistently use the project icon, package
`shuo` as a real `.app` bundle and use `macos/shuohua.icns` from this directory
as the bundle icon.
