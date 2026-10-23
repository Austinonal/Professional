//! FilmCraft's platform services, copied unchanged from `filmcraft/apps/filmcraft/src/main.rs`
//! (the app's hooks, clipboard media, open/reveal, the GPU export renderer, hardware decoders,
//! Linux file dialogs), so every hook the FilmCraft app wires up works here too. Re-copy it when
//! upstream changes.
#![allow(dead_code)]

use super::{appearance, audio, file_filters, window_raise};
use chrono::TimeZone;
use filmcraft_ui_egui::FilmcraftApp;

/// The app's sound output and file, clipboard, link and system hooks.
pub(super) fn hooks(app: &mut FilmcraftApp) {
    app.audio = Some(Box::new(audio::CpalOut::new()));
    app.hooks.pick_files = Some(Box::new(|exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("Media"), &file_filters::extensions(exts))
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }));
    // Link Media ▸ Locate…, Attach Proxies, Reconnect Full Resolution: one path, not imported.
    app.hooks.pick_file_for_relink = Some(Box::new(|exts: &[&str], _hint| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("Media"), &file_filters::extensions(exts))
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_save = Some(Box::new(|name: &str| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("FilmCraft Project"), &file_filters::extensions(&["fcproj"]))
            .set_file_name(name)
            .save_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_save_as = Some(Box::new(|filter: &str, exts: &[&str], name: &str| {
        // a full path (Quick Export, Export mode) opens the dialog in its folder
        let path = std::path::Path::new(name);
        let mut dialog = rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t(filter), &file_filters::extensions(exts))
            .set_file_name(path.file_name().map_or(name.into(), |n| n.to_string_lossy()));
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_folder = Some(Box::new(|| rfd::FileDialog::new().pick_folder().map(|p| p.to_string_lossy().to_string())));
    app.hooks.pick_folder_at =
        Some(Box::new(|dir: &str| rfd::FileDialog::new().set_directory(dir).pick_folder().map(|p| p.to_string_lossy().into_owned())));
    // Linux: the XDG portal call blocks until the user chooses, which would stop the frame
    // loop (GNOME offers to force-quit the window after about five seconds, #260). Show the
    // dialog on its own thread instead; the app keeps running and acts on the answer.
    #[cfg(target_os = "linux")]
    {
        app.hooks.run_dialog = Some(Box::new(run_dialog_off_thread));
    }
    app.hooks.open_path = Some(Box::new(open_path));
    // Help menu, About and Discord links (#642): eframe is built without its `links`
    // feature, so `ctx.open_url` does nothing on the desktop. Detached, so a launcher
    // that waits for the browser never freezes the UI.
    app.hooks.open_url = Some(Box::new(|url: &str| open::that_detached(url).map_err(|e| e.to_string())));
    app.hooks.clipboard_media = Some(Box::new(clipboard_media));
    // Settings ▸ General ▸ Interface Language ▸ System Language (#218).
    app.hooks.cursor_screen_position = Some(Box::new(filmcraft_platform::cursor::cursor_screen_position));
    app.hooks.system_languages = Some(Box::new(|| sys_locale::get_locales().collect()));
    // Settings ▸ Appearance ▸ Appearance Mode ▸ Sync with system on Linux desktops whose
    // compositor reports no theme to winit (no polling: see appearance.rs).
    app.hooks.system_theme = appearance::service();
    app.hooks.raise_without_focus = Some(Box::new(|| {
        window_raise::raise_without_focus();
    }));
    app.hooks.pick_open_file = Some(Box::new(|filter: &str, exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t(filter), &file_filters::extensions(exts))
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_open_project = Some(Box::new(|| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("FilmCraft Project"), &file_filters::extensions(&["fcproj"]))
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
}

/// Files copied in the file manager, else an image, on the system clipboard (Paste, #611).
fn clipboard_media() -> Option<filmcraft_ui_egui::ClipboardMedia> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Some(filmcraft_ui_egui::ClipboardMedia::Files(files.iter().map(|p| p.to_string_lossy().into_owned()).collect()));
    }
    let image = clipboard.get_image().ok()?;
    Some(filmcraft_ui_egui::ClipboardMedia::Image { width: image.width, height: image.height, rgba: image.bytes.into_owned() })
}

/// Open a file in its default application, or reveal it in the file manager (Edit Original,
/// Reveal Log Files).
fn open_path(path: &str, reveal: bool) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        if reveal {
            c.arg(format!("/select,{path}"));
        } else {
            c.arg(path);
        }
        c
    } else {
        // Linux file managers have no select-file flag: open the parent folder instead.
        let mut c = std::process::Command::new("xdg-open");
        let p = std::path::Path::new(path);
        c.arg(if reveal { p.parent().unwrap_or(p) } else { p });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("can't open {path}: {e}"))
}

/// The GPU export frame renderer (filmcraft-gpu's off-screen compositor behind filmcraft-export's
/// frame-renderer hook): exports with GPU rendering Auto composite on the GPU and fall back to the
/// CPU reference renderer wherever it cannot.
struct GpuFrameRenderer(filmcraft_gpu::ExportRenderer);

impl filmcraft_export::FrameRenderer for GpuFrameRenderer {
    fn render(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<filmcraft_render::Image> {
        self.0.render(project, seq, t, opts, sources)
    }
}

pub(super) fn register_gpu_frame_renderer() {
    filmcraft_export::register_frame_renderer(|| {
        filmcraft_gpu::ExportRenderer::new().map(|r| Box::new(GpuFrameRenderer(r)) as Box<dyn filmcraft_export::FrameRenderer>)
    });
}

/// Put the OS hardware video decoders in front of our own. Registered in a statement of its own:
/// a log macro does not evaluate its arguments while no logger takes its level, which left the
/// hardware decoders out of every run.
pub(super) fn register_hardware_decoders() -> filmcraft_platform::Availability {
    let hardware = filmcraft_platform::register();
    log::info!("hardware decoding: {hardware:?}");
    hardware
}

/// Local UTC offset at a unix time (auto-save file names and recovery times use local time).
pub(super) fn local_offset(unix: i64) -> i32 {
    chrono::Local.timestamp_opt(unix, 0).single().map(|d| d.offset().local_minus_utc()).unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn run_dialog_off_thread(dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> std::sync::mpsc::Receiver<Vec<String>> {
    use filmcraft_ui_egui::native_dialogs::FileDialog;
    let t = |s: &str| filmcraft_ui_egui::i18n::t(s).to_string();
    let media = t("Media");
    let project = t("FilmCraft Project");
    let project_exts = project_patterns();
    let dialog = prepare_dialog(dialog);
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        let shown = std::panic::catch_unwind(move || {
            let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
            match dialog {
                FileDialog::OpenFiles { exts } => {
                    rfd::FileDialog::new().add_filter(&media, &exts).pick_files().unwrap_or_default().into_iter().map(path).collect()
                }
                FileDialog::OpenFile { filter, exts } => rfd::FileDialog::new().add_filter(&filter, &exts).pick_file().map(path).into_iter().collect(),
                FileDialog::Relink { exts, .. } => rfd::FileDialog::new().add_filter(&media, &exts).pick_file().map(path).into_iter().collect(),
                FileDialog::OpenProject => rfd::FileDialog::new().add_filter(&project, &project_exts).pick_file().map(path).into_iter().collect(),
                FileDialog::SaveProject { name } => {
                    rfd::FileDialog::new().add_filter(&project, &project_exts).set_file_name(name).save_file().map(path).into_iter().collect()
                }
                FileDialog::SaveAs { filter, exts, name } => {
                    rfd::FileDialog::new().add_filter(&filter, &exts).set_file_name(name).save_file().map(path).into_iter().collect()
                }
                FileDialog::Folder { at } => {
                    let d = rfd::FileDialog::new();
                    let d = match at {
                        Some(dir) if !dir.is_empty() => d.set_directory(dir),
                        _ => d,
                    };
                    d.pick_folder().map(path).into_iter().collect::<Vec<_>>()
                }
            }
        });
        match shown {
            Ok(paths) => {
                let _ = tx.send(paths);
            }
            Err(_) => log::warn!("the file dialog thread panicked; taken as cancelled"),
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not start the file dialog thread: {e}");
    }
    rx
}

/// A dialog as the Linux dialog thread shows it: filter names translated (here, on the UI thread:
/// the interface language is per thread) and every extension list turned into the
/// case-insensitive patterns the synchronous pickers use, so `CLIP.MP4` stays visible (#262).
#[cfg(target_os = "linux")]
fn prepare_dialog(dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> filmcraft_ui_egui::native_dialogs::FileDialog {
    use filmcraft_ui_egui::native_dialogs::FileDialog;
    let t = |s: &str| filmcraft_ui_egui::i18n::t(s).to_string();
    let patterns = |exts: Vec<String>| file_filters::extensions(&exts.iter().map(String::as_str).collect::<Vec<_>>());
    match dialog {
        FileDialog::OpenFiles { exts } => FileDialog::OpenFiles { exts: patterns(exts) },
        FileDialog::OpenFile { filter, exts } => FileDialog::OpenFile { filter: t(&filter), exts: patterns(exts) },
        FileDialog::SaveAs { filter, exts, name } => FileDialog::SaveAs { filter: t(&filter), exts: patterns(exts), name },
        FileDialog::Relink { exts, hint } => FileDialog::Relink { exts: patterns(exts), hint },
        other => other,
    }
}

/// The project dialogs' filter patterns (`EDIT.FCPROJ` included).
#[cfg(target_os = "linux")]
fn project_patterns() -> Vec<String> {
    file_filters::extensions(&["fcproj"])
}

