# CraftStudio

Six apps in one window: LightCraft (photo library and raw developer), PhotoCraft (image editing),
VectorCraft (illustration), FigCraft (interface design and prototyping), EffectCraft (motion
graphics and visual effects) and FilmCraft (video). A Home screen with plain-language starting
points and an always-visible switch: **Home · Library · Photo · Vector · Design · Motion · Video**.
Every app keeps its work when you switch tabs.

Each tab is the full upstream app (its own `ui-egui` crate), so every tool, panel and command is
there. Each app's platform hooks (file dialogs, clipboard, printing, settings, libraries, crash
recovery and auto-save, FilmCraft's sound, hardware decoders and GPU export) are its own modules,
compiled as they are or copied unchanged.

Where the apps meet:

- Library ▸ Photo ▸ Edit In ▸ **Edit in PhotoCraft** opens the copy in the Photo tab.
- Vector ▸ Links ▸ **Edit Original** on a placed PNG/JPEG/TIFF/PSD opens it in the Photo tab.
- Files given on the command line open in the right tab (drawings → Vector, `.figcraft` →
  Design, `.ecproj` → Motion, images → Photo, media → Video).
- Quitting with unsaved work in a hidden tab says which tabs and lets you go to them first.

## Build

Expects the upstream repos next to this folder:

    git clone https://github.com/storytold/lightcraft ../lightcraft
    git clone https://github.com/storytold/photocraft ../photocraft
    git clone https://github.com/storytold/vectorcraft ../vectorcraft
    git clone https://github.com/storytold/figcraft ../figcraft
    git clone https://github.com/storytold/effectcraft ../effectcraft
    git clone https://github.com/storytold/filmcraft ../filmcraft
    cargo build --release      # target/release/craftstudio, about 350 MB, one file

Linux only for now (the macOS and Windows hooks aren't wired in); FilmCraft's sound needs ALSA
(`libasound2-dev` to build). Format with
`rustfmt` on `src/*.rs`, not `cargo fmt`, which would follow the `#[path]` modules into the
upstream repos.

Screenshots are in `shots/`.
