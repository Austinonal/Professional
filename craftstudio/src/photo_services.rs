//! PhotoCraft's platform services: open, save, export and file dialogs. A trimmed copy of
//! `photocraft/apps/photocraft/src/services.rs` (same photocraft-io and atomic-write calls).
//! shortcut: no clipboard images, crash recovery or Linux portal save dialog; add them from
//! upstream `services.rs` when CraftStudio ships beyond a preview.

use std::path::{Path, PathBuf};

use photocraft_doc::Document;
use photocraft_ui_egui::{
    ExportSettings, FileDialogAnswer, FileDialogReply, FileDialogRequest, Services,
    jobs_ui::SaveCtl,
};

pub fn native() -> Services {
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], depth: usize| {
            guard(|| {
                photocraft_io::import_with_svg_group_depth(name, bytes, depth)
                    .map(|r| (r.document, r.warnings))
                    .map_err(|e| e.to_string())
            })
        })),
        export: Some(Box::new(
            |doc: &Document, path: &str, settings: &ExportSettings| {
                guard(|| {
                    photocraft_io::export(doc, path, &settings.io_options())
                        .map(|r| (r.bytes, r.warnings))
                        .map_err(|e| e.to_string())
                })
            },
        )),
        save_file: Some(std::sync::Arc::new(save_file)),
        file_dialog: Some(Box::new(show_file_dialog)),
        write: Some(Box::new(|path: &str, bytes: &[u8]| {
            photocraft_format::atomic_write(Path::new(path), bytes).map_err(|e| e.to_string())
        })),
        file_stamp: Some(Box::new(|path: &str| {
            let meta = std::fs::metadata(path).ok()?;
            Some(photocraft_ui_egui::FileStamp {
                len: meta.len(),
                modified: meta.modified().ok(),
            })
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec())
                .map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default())
                .map_err(|e| e.to_string())
        })),
        load_prefs: Some(Box::new(|| std::fs::read_to_string(prefs_file()?).ok())),
        save_prefs: Some(Box::new(|text: &str| {
            let file = prefs_file().ok_or("no config directory")?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            photocraft_format::atomic_write(&file, text.as_bytes()).map_err(|e| e.to_string())
        })),
        ..Default::default()
    }
}

/// A decoder or encoder panic on a hostile file becomes an error, not a crash (as upstream).
fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("the file could not be processed (internal error)".into()))
}

fn prefs_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("craftstudio").join("photo-preferences.json"))
}

fn save_file(
    doc: &Document,
    path: &str,
    settings: &ExportSettings,
    ctl: &SaveCtl,
) -> Result<Vec<String>, String> {
    ctl.progress(0.0, "Encoding");
    let cancelled = || ctl.check().is_err();
    let progress = |f: f32| ctl.progress(f * 0.85, "");
    let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
    let r = guard(|| {
        photocraft_io::export_with(doc, path, &settings.io_options(), &interrupt)
            .map_err(|e| e.to_string())
    })?;
    ctl.commit()?;
    ctl.progress(0.85, "Writing");
    photocraft_format::atomic_write(Path::new(path), &r.bytes).map_err(|e| e.to_string())?;
    ctl.progress(1.0, "");
    Ok(r.warnings)
}

/// Open/Save dialogs on a helper thread so the window keeps drawing while one is up.
fn show_file_dialog(
    request: FileDialogRequest,
    _parent: Option<&eframe::Frame>,
    reply: FileDialogReply,
) {
    let spawned = std::thread::Builder::new()
        .name("file dialog".into())
        .spawn(move || {
            let d = rfd::FileDialog::new();
            let s = |p: PathBuf| p.to_string_lossy().into_owned();
            let answer = match request {
                FileDialogRequest::Open {
                    multiple,
                    initial_dir,
                    extensions,
                } => {
                    let mut d = initial_dir.map_or(d.clone(), |dir| d.set_directory(dir));
                    if let Some(exts) = extensions {
                        d = d.add_filter("Supported Files", &exts);
                    }
                    if multiple {
                        d.pick_files().map(|f| f.into_iter().map(s).collect())
                    } else {
                        d.pick_file().map(|f| vec![s(f)])
                    }
                    .map(FileDialogAnswer::Paths)
                }
                FileDialogRequest::Folder { initial_dir } => initial_dir
                    .map_or(d.clone(), |dir| d.set_directory(dir))
                    .pick_folder()
                    .map(|f| FileDialogAnswer::FolderTo(s(f))),
                FileDialogRequest::Save { suggested } => {
                    let p = Path::new(&suggested);
                    let mut d = d;
                    if let Some(name) = p.file_name() {
                        d = d.set_file_name(name.to_string_lossy());
                    }
                    if let Some(dir) = p.parent().filter(|p| p.is_dir()) {
                        d = d.set_directory(dir);
                    }
                    d.save_file().map(|f| FileDialogAnswer::SaveTo(s(f)))
                }
            };
            reply.send(answer);
        });
    // Without the thread the reply is dropped unanswered, which the app takes as Cancel.
    if let Err(e) = spawned {
        log::error!("couldn't show the file dialog: {e}");
    }
}
