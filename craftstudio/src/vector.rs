//! VectorCraft (vector illustration) inside CraftStudio. The app is VectorCraft's own
//! `VectorcraftApp` with its preferences, libraries, clipboard, printing and file dialogs (the
//! upstream modules, compiled as they are, and `vector_services.rs`). Links ▸ Edit Original on a
//! placed raster image opens it in this window's Photo tab.

#[path = "../../vectorcraft/apps/vectorcraft/src/clipboard.rs"]
mod clipboard;
// Only its missing-theme probe is used here; the rest serves VectorCraft's own launcher.
#[allow(dead_code)]
#[path = "../../vectorcraft/apps/vectorcraft/src/cursor_theme.rs"]
mod cursor_theme;
#[cfg(target_os = "linux")]
#[path = "../../vectorcraft/apps/vectorcraft/src/portal_save.rs"]
mod portal_save;
#[path = "../../vectorcraft/apps/vectorcraft/src/prefs_dir.rs"]
mod prefs_dir;
#[path = "../../vectorcraft/apps/vectorcraft/src/printing.rs"]
mod printing;
#[path = "vector_services.rs"]
mod services;

use std::sync::mpsc::Sender;

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_ui_egui::VectorcraftApp;

/// Placed images that Edit Original hands to the Photo tab.
const RASTER: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "psd", "psb", "webp", "gif", "bmp",
];

fn prefs_path() -> Option<std::path::PathBuf> {
    prefs_dir::prefs_path_for("VectorCraft", "vectorcraft")
}

fn is_raster(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RASTER.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn new(cc: &eframe::CreationContext<'_>, to_photo: Sender<String>) -> VectorcraftApp {
    prefs_dir::install_fonts();
    let mut session = Session::new();
    prefs_dir::attach_profile_store(&mut session);
    prefs_dir::attach_plugin_store(&mut session);
    let mut services = services::services(cc.winit_window().cloned(), cc.egui_ctx.clone());
    services.open_file = Some(Box::new(move |path: &str| {
        if is_raster(path) {
            to_photo.send(path.to_string()).map_err(|e| e.to_string())
        } else {
            services::open_file(path)
        }
    }));
    let mut app = VectorcraftApp::new(session, services);
    // An unreadable preferences file means defaults, as in VectorCraft.
    if let Some(ui) = prefs_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<vectorcraft_ui_egui::UiState>(&b).ok())
    {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(&mut app);
    prefs_dir::attach_pdf_preset_store(&mut app.session);
    let beside = |name: &str| {
        prefs_path().and_then(|p| Some(p.parent()?.join(name).to_string_lossy().to_string()))
    };
    app.session
        .swatch_libraries
        .set_user_dir(beside("Swatches"));
    app.session
        .style_libraries
        .set_user_dir(beside("Graphic Styles"));
    vectorcraft_ui_egui::panels::library_panel::restore(&mut app);
    app.session.libraries.set_dir(beside("Libraries"));
    app.session
        .recovery
        .set_default_folder(beside("Data Recovery"));
    if let Some(rs) = &cc.wgpu_render_state {
        let info = rs.adapter.get_info();
        app.graphics_adapter = Some(format!(
            "{} ({:?}, {:?})",
            info.name.trim(),
            info.backend,
            info.device_type
        ));
    }
    // CraftStudio's bar sits above it: VectorCraft keeps the window's normal title bar.
    app.custom_titlebar = false;
    app
}

/// Preferences are written when confirmed and at exit (as VectorCraft); best effort.
pub fn save_prefs(app: &VectorcraftApp) {
    let Some(p) = prefs_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut ui = app.ui.clone();
    ui.engine_prefs = app.session.prefs.to_json();
    if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
        let _ = fileio::write_atomic(&p, &bytes);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_original_routes_only_raster_images() {
        assert!(super::is_raster("/a/b/photo.JPG"));
        assert!(!super::is_raster("/a/b/logo.svg"));
        assert!(!super::is_raster("noext"));
    }
}
