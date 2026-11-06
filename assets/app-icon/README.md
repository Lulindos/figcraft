# LegatIA Design app icon

The Legat.IA mark: a white "IA" on a black rounded square (the same as `public/favicon.png` in the
LegatIAdm site). It replaces the upstream chameleon in the same files, so nothing else changes.

Geometry: a 1054-unit tile (`viewBox="0 0 1054 1054"`), `rx=190`, fill `#08090A`; the two white
paths are `public/logo-ia.svg` in the site repository. Windows and Linux use the tile edge to edge;
`packaging/macos/package.sh` pads it for the macOS grid.

## Files

- `figcraft.svg`: canonical master. `figcraft-small.svg`: the same drawing (also the hicolor scalable icon).
- `figcraft-<size>.png` (16–1024): renders on the macOS grid; `packaging/macos/package.sh` builds
  the `.icns` from them.
- `figcraft.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/figcraft/build.rs`.
- `hicolor/<size>/apps/ai.storyteller.figcraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme; the 256 px one is also the runtime window icon off macOS.

## Regenerate

Replace `figcraft.svg` (and `figcraft-small.svg`), then run `cargo xtask icon`. It needs `resvg`
(`cargo install resvg`).
