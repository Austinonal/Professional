# CraftStudio

LightCraft (photo library and raw developer), PhotoCraft (image editing), VectorCraft
(illustration) and FilmCraft (video) in one window. A Home screen with plain-language starting
points and an always-visible switch: **Home · Library · Photo · Vector · Video**. Every app keeps
its work when you switch tabs.

Each tab is the full upstream app (its own `ui-egui` crate), so every tool, panel and command is
there. LightCraft's and VectorCraft's platform hooks (file dialogs, clipboard, printing, settings,
libraries) are their own modules, compiled as they are or copied unchanged.

Where the apps meet:

- Library ▸ Photo ▸ Edit In ▸ **Edit in PhotoCraft** opens the copy in the Photo tab.
- Vector ▸ Links ▸ **Edit Original** on a placed PNG/JPEG/TIFF/PSD opens it in the Photo tab.
- Files given on the command line open in the right tab (drawings → Vector, images → Photo,
  media → Video).
- Quitting with unsaved work in a hidden tab says which tabs and lets you go to them first.

## Build

Expects the upstream repos next to this folder:

    git clone https://github.com/storytold/lightcraft ../lightcraft
    git clone https://github.com/storytold/photocraft ../photocraft
    git clone https://github.com/storytold/vectorcraft ../vectorcraft
    git clone https://github.com/storytold/filmcraft ../filmcraft
    cargo run

Linux only for now (VectorCraft's macOS and Windows hooks aren't wired in). Format with
`rustfmt` on `src/*.rs`, not `cargo fmt`, which would follow the `#[path]` modules into the
upstream repos.

Screenshots are in `shots/`.
