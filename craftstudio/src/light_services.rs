//! LightCraft's platform services, copied unchanged from `lightcraft/apps/lightcraft/src/main.rs`
//! (`display_profile_dir`, the Windows helpers, `file_dialog` and `services`), so every dialog,
//! hook and picker the LightCraft app wires up works here too. Re-copy it when upstream changes.
#![allow(dead_code)]

#[cfg(target_os = "linux")]
use super::colord;
use super::dialog_filter::FileDialogExt as _;
use super::{appearance, editors};
use lightcraft_ui_egui::Services;

/// Where the system keeps monitor profiles, for the Choose Profile dialog to start in: the user's
/// own profiles first (Linux: colord / DisplayCAL; macOS: ColorSync), then the system's.
fn display_profile_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from);
    let user = |rel: &str| home.as_ref().map(|h| h.join(rel));
    let candidates: Vec<Option<std::path::PathBuf>> = if cfg!(target_os = "macos") {
        vec![
            user("Library/ColorSync/Profiles"),
            Some("/Library/ColorSync/Profiles".into()),
        ]
    } else if cfg!(windows) {
        let root = std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:\\Windows".into());
        vec![Some(
            root.join("System32")
                .join("spool")
                .join("drivers")
                .join("color"),
        )]
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| user(".local/share"));
        vec![
            data.map(|d| d.join("icc")),
            user(".color/icc"),
            Some("/usr/share/color/icc".into()),
        ]
    };
    candidates.into_iter().flatten().find(|d| d.is_dir())
}

/// The Windows command that opens `url` in the default browser. `explorer <url>` opened File
/// Explorer instead for links with a query or fragment, like the Map button's OpenStreetMap link
/// (issue #234). The URL protocol handler hands the URL to the browser as is; no shell runs, so
/// `?`, `&` and `#` need no escaping.
fn windows_open_url_command(url: &str) -> std::process::Command {
    let mut c = std::process::Command::new("rundll32");
    c.args(["url.dll,FileProtocolHandler", url]);
    c
}

/// Explorer's `/select,<path>` switch (issue #272). Explorer parses its own command line and only
/// selects the file when the path after the comma is quoted (`/select,"C:\My Photos\a.jpg"`),
/// spelled with backslashes and free of the verbatim prefix (`\\?\`, `\\?\UNC\`) that
/// `canonicalize` adds; given anything else it opens Documents. Windows file names can't contain
/// `"`; a path that does anyway (a hostile catalog) has them dropped, so the raw command line
/// always holds exactly this one argument.
fn explorer_select_arg(path: &str) -> String {
    let path = path.replace('/', "\\").replace('"', "");
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => path
            .strip_prefix(r"\\?\")
            .unwrap_or(path.as_str())
            .to_string(),
    };
    format!("/select,\"{path}\"")
}

/// `explorer /select,"<path>"`. Rust's own argument quoting wraps the whole switch in quotes
/// (`"/select,C:\My Photos\a.jpg"`) whenever the path has a space, which Explorer doesn't
/// recognise, so on Windows the switch goes on the command line exactly as built.
fn explorer_select_command(path: &str) -> std::process::Command {
    let mut c = std::process::Command::new("explorer");
    let arg = explorer_select_arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.raw_arg(arg);
    }
    #[cfg(not(windows))]
    {
        c.arg(arg);
    }
    c
}

/// The file dialogs' parent, set once the window exists (`services`). A dialog with a parent is
/// a child of the app's window; without one, the xdg-desktop-portal picker on Wayland is a stray
/// top-level ("Failed to associate portal window with parent window" in the portal's log): a
/// tiling compositor puts it beside the app, and a fullscreen window hides it entirely, so
/// Import Photos… looks like a hang. rfd exports the Wayland surface through xdg_foreign itself;
/// `FileDialog` is `Clone` and `Send`, so one parented template serves every picker, including
/// the one on its own thread (#191). Only set on Linux and FreeBSD: on macOS a parented dialog
/// becomes a sheet and on Windows a modal, a behaviour change this does not make.
static DIALOG_PARENT: std::sync::OnceLock<rfd::FileDialog> = std::sync::OnceLock::new();

/// A file dialog owned by the app's window, or a plain one before the window exists.
fn file_dialog() -> rfd::FileDialog {
    DIALOG_PARENT.get().cloned().unwrap_or_default()
}

/// The platform services; `cc` is the window the file dialogs belong to (its `egui_ctx` repaints
/// when a dialog closes), and `log_file` is for Help ▸ Open Log Folder.
pub(super) fn services(
    cc: &eframe::CreationContext<'_>,
    log_file: Option<&std::path::Path>,
) -> Services {
    let ctx = cc.egui_ctx.clone();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let _ = DIALOG_PARENT.set(rfd::FileDialog::new().set_parent(cc));
    Services {
        // Commands' file dialogs run on their own thread (#191): a dialog on the UI thread
        // stopped the window answering the compositor, which then reported the app as hung.
        picker: Some(Box::new(
            move |req: lightcraft_ui_egui::pick::PickRequest| {
                use lightcraft_ui_egui::pick::PickKind;
                let (tx, rx) = std::sync::mpsc::channel();
                let repaint = ctx.clone();
                std::thread::Builder::new()
                    .name("file-dialog".into())
                    .spawn(move || {
                        let mut d = file_dialog();
                        if let Some(t) = &req.title {
                            d = d.set_title(t);
                        }
                        if let Some((name, exts)) = &req.filter {
                            d = d.add_filter_nocase(name.clone(), exts);
                        }
                        if let Some(n) = &req.file_name {
                            d = d.set_file_name(n);
                        }
                        let s = |p: std::path::PathBuf| p.to_string_lossy().to_string();
                        let paths: Vec<String> = match req.kind {
                            PickKind::Files => d
                                .pick_files()
                                .unwrap_or_default()
                                .into_iter()
                                .map(s)
                                .collect(),
                            PickKind::File => d.pick_file().map(s).into_iter().collect(),
                            PickKind::Folder => d.pick_folder().map(s).into_iter().collect(),
                            PickKind::Save => d.save_file().map(s).into_iter().collect(),
                        };
                        // the app may have quit meanwhile: nobody to tell
                        let _ = tx.send(paths);
                        repaint.request_repaint();
                    })
                    .map_err(|e| format!("could not start the file dialog: {e}"))?;
                Ok(rx)
            },
        )),
        log_file: log_file.map(|p| p.display().to_string()),
        pick_lightroom_catalog: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Import Lightroom Catalog"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Lightroom Classic Catalog"),
                    &["lrcat"],
                )
                .pick_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_folder: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Open Library"))
                .pick_folder()
                .map(|p| p.to_string_lossy().to_string())
        })),
        #[cfg(target_os = "linux")]
        open_model_folder: Some(std::sync::Arc::new(|path: &str| {
            std::process::Command::new("xdg-open")
                .arg(path)
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    if s.success() {
                        Ok(())
                    } else {
                        Err(format!("opening folder failed: {s}"))
                    }
                })
        })),
        open_with: Some(Box::new(editors::open_with)),
        open_all_with: Some(Box::new(editors::open_all_with)),
        check_editor: Some(Box::new(editors::check)),
        open_url: Some(Box::new(|url: &str| {
            if !url.starts_with("https://") {
                return Err("only https links are opened".into());
            }
            let status = if cfg!(target_os = "macos") {
                std::process::Command::new("open").arg(url).status()
            } else if cfg!(target_os = "windows") {
                windows_open_url_command(url).status()
            } else {
                std::process::Command::new("xdg-open").arg(url).status()
            };
            status.map(|_| ()).map_err(|e| e.to_string())
        })),
        reveal: Some(Box::new(|path: &str| {
            let status = if cfg!(target_os = "macos") {
                std::process::Command::new("open")
                    .args(["-R", path])
                    .status()
            } else if cfg!(target_os = "windows") {
                explorer_select_command(path).status()
            } else {
                let dir = std::path::Path::new(path)
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".into());
                std::process::Command::new("xdg-open").arg(dir).status()
            };
            status.map_err(|e| e.to_string()).and_then(|s| {
                if s.success() || cfg!(target_os = "windows") {
                    Ok(())
                } else {
                    Err(format!("reveal failed: {s}"))
                }
            })
        })),
        pick_files: Some(Box::new(|| {
            file_dialog()
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Photos"),
                    lightcraft_engine::import::EXTENSIONS,
                )
                .pick_files()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })),
        pick_denoise_model: Some(Box::new(|| {
            file_dialog()
                .add_filter_nocase(lightcraft_ui_egui::i18n::tr("ONNX model"), &["onnx"])
                .pick_file()
                .map(|p| vec![p.to_string_lossy().into_owned()])
                .unwrap_or_default()
        })),
        pick_preset_files: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Import Presets"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Presets & Profiles"),
                    &[
                        "lcpreset",
                        "xmp",
                        "lrtemplate",
                        "zip",
                        "dng",
                        "lmp",
                        "mplumpack",
                        "cube",
                    ],
                )
                .pick_files()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })),
        pick_model_file: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Add a Face Recognition Model"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Face models (ONNX)"),
                    &["onnx"],
                )
                .pick_file()
                .map(|p| vec![p.to_string_lossy().to_string()])
                .unwrap_or_default()
        })),
        pick_tracklog: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Auto-Tag from Tracklog"))
                .add_filter_nocase(lightcraft_ui_egui::i18n::tr("GPS Track Log"), &["gpx"])
                .pick_file()
                .map(|p| vec![p.to_string_lossy().to_string()])
                .unwrap_or_default()
        })),
        // macOS doesn't colour-manage the window: previews follow the screen's profile (#787)
        #[cfg(target_os = "macos")]
        screen_profile: Some(Box::new(screen_profile::icc)),
        // Linux: the profile the desktop assigns in colord, read off the UI thread
        #[cfg(target_os = "linux")]
        screen_profile: Some(colord::service()),
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        screen_profile: None,
        pick_display_profile: Some(Box::new(|| {
            let mut d = file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Choose Monitor Profile"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("ICC Profiles"),
                    &["icc", "icm"],
                );
            if let Some(dir) = display_profile_dir() {
                d = d.set_directory(dir);
            }
            d.pick_file()
                .map(|p| vec![p.to_string_lossy().to_string()])
                .unwrap_or_default()
        })),
        save_preset_file: Some(Box::new(|name: &str| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Export Presets"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("LightCraft Preset"),
                    &["lcpreset"],
                )
                .set_file_name(name)
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_keyword_list: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Import Keywords"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Keyword Lists"),
                    &["txt", "utf8"],
                )
                .pick_file()
                .map(|p| vec![p.to_string_lossy().to_string()])
                .unwrap_or_default()
        })),
        save_keyword_list: Some(Box::new(|name: &str| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Export Keywords"))
                .add_filter_nocase(lightcraft_ui_egui::i18n::tr("Keyword Lists"), &["txt"])
                .set_file_name(name)
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_curve_preset_files: Some(Box::new(|| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Import Point Curve Presets"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Point Curve Presets"),
                    &["lccurve", "json"],
                )
                .pick_files()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })),
        save_curve_preset_file: Some(Box::new(|name: &str| {
            file_dialog()
                .set_title(lightcraft_ui_egui::i18n::tr("Export Point Curve Presets"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Point Curve Presets"),
                    &["lccurve"],
                )
                .set_file_name(name)
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_slideshow_templates: Some(Box::new(|| {
            rfd::FileDialog::new()
                .set_title(lightcraft_ui_egui::i18n::tr("Import Slideshow Templates"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Slideshow Templates"),
                    &["json"],
                )
                .pick_files()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })),
        save_slideshow_template: Some(Box::new(|name: &str| {
            rfd::FileDialog::new()
                .set_title(lightcraft_ui_egui::i18n::tr("Export Slideshow Template"))
                .add_filter_nocase(
                    lightcraft_ui_egui::i18n::tr("Slideshow Templates"),
                    &["json"],
                )
                .set_file_name(name)
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        // atomic (temp file + sync + rename): a failed write never leaves a truncated file
        write_shared: Some(std::sync::Arc::new(lightcraft_engine::export::write_file)),
        write: Some(Box::new(lightcraft_engine::export::write_file)),
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(
                &lightcraft_codecs::EncodeImage::rgba8(img),
                &lightcraft_codecs::EncodeMeta::default(),
            )
            .unwrap_or_default()
        })),
        // the library is a folder on disk: backed up with the user's other files
        backup_library: None,
        restore_library: None,
        system_theme: appearance::service(),
    }
}
