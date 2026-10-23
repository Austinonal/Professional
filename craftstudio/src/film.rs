//! FilmCraft (video editing) inside CraftStudio. The app is FilmCraft's own `FilmcraftApp`,
//! started as the FilmCraft app starts it: crash log, hardware decoders, the GPU export renderer,
//! auto-save and crash recovery, the microphone for voice-over, sound output and every host hook
//! (the upstream modules, compiled as they are, and `film_services.rs`).

#[path = "../../filmcraft/apps/filmcraft/src/appearance.rs"]
mod appearance;
#[path = "../../filmcraft/apps/filmcraft/src/audio.rs"]
mod audio;
#[path = "../../filmcraft/apps/filmcraft/src/audio_in.rs"]
mod audio_in;
#[path = "../../filmcraft/apps/filmcraft/src/file_filters.rs"]
mod file_filters;
#[path = "film_services.rs"]
mod services;
#[path = "../../filmcraft/apps/filmcraft/src/window_raise.rs"]
mod window_raise;

use filmcraft_engine::Session;
use filmcraft_engine::autosave::{AutosaveConfig, default_data_dir};
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::json;

/// Before the window opens, as in FilmCraft's `main`.
pub fn start() {
    // Panics go to <data dir>/Logs/crash-<day>.log; FilmCraft's UI shows what it caught from there.
    filmcraft_ui_egui::crash::install(default_data_dir().map(|d| d.join("Logs")));
    services::register_hardware_decoders();
    services::register_gpu_frame_renderer();
}

pub fn new(cc: &eframe::CreationContext<'_>) -> FilmcraftApp {
    let mut session = Session::default();
    if let Some(dir) = default_data_dir() {
        let mut cfg = AutosaveConfig::new(dir);
        cfg.local_offset = services::local_offset;
        if let Err(e) = session.start_autosave(cfg) {
            log::warn!("auto-save and crash recovery unavailable: {e}");
        }
    }
    session.voiceover.input = Some(Box::new(audio_in::CpalIn::new(
        &session.prefs.audio_hardware.device_class,
    )));
    // Settings ▸ General ▸ At Startup ▸ Open Most Recent
    if session.prefs.general.at_startup == "openMostRecent" {
        let recent = session
            .prefs
            .general
            .recent_projects
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .cloned();
        let opened = match recent {
            Some(p) => session.execute("file.open", json!({"path": p})),
            None => session.execute("file.openDemoProject", json!({})),
        };
        if let Err(e) = opened {
            log::error!("{e}");
        }
    }
    let mut app = FilmcraftApp::new(session);
    if let Some(rs) = cc.wgpu_render_state.clone()
        && std::env::var_os("FILMCRAFT_CPU_COMPOSITE").is_none()
    {
        app.set_wgpu(rs);
    }
    services::hooks(&mut app);
    app
}
