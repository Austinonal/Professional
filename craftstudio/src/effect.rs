//! EffectCraft (motion graphics and visual effects) inside CraftStudio, started as the EffectCraft
//! app starts it: its logger and panic hook, the system font scan, the GPU failure bridge on the
//! window's device, plug-ins, settings and crash recovery from its config folder, audio output
//! and every host hook (the upstream modules, compiled as they are, and `effect_services.rs`).

#[path = "../../effectcraft/apps/effectcraft/src/appearance.rs"]
mod appearance;
#[path = "../../effectcraft/apps/effectcraft/src/audio_out.rs"]
mod audio_out;
#[path = "effect_services.rs"]
mod services;
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "../../effectcraft/apps/effectcraft/src/sheet_dialogs.rs"]
mod sheet_dialogs;

use effectcraft_ui_egui::EffectcraftApp;
use serde_json::json;

pub(crate) use services::wgpu_options;

/// Before the window opens, as in EffectCraft's `main`. EffectCraft's logger is the one the whole
/// window uses: records go to stderr and `<config>/Logs/effectcraft.log`.
pub fn start() {
    effectcraft_engine::logging::install();
    effectcraft_engine::logging::install_panic_hook();
    if let Some(dir) = effectcraft_host::config_dir() {
        if let Err(e) = effectcraft_engine::logging::attach_session_log(&effectcraft_engine::logging::log_dir(Some(&dir))) {
            log::warn!("no log file: {e}");
        }
    }
    effectcraft_engine::text::fonts::scan_system_in_background();
}

pub fn new(cc: &eframe::CreationContext<'_>) -> EffectcraftApp {
    // The window's device handlers report to EffectCraft, before its compositor pipelines exist.
    let gpu_failures = effectcraft_ui_egui::gpu_failure::GpuFailureBridge::new(&cc.egui_ctx);
    if let Some(rs) = &cc.wgpu_render_state {
        let errors = gpu_failures.clone();
        rs.device.on_uncaptured_error(std::sync::Arc::new(move |error| {
            let message = format!("uncaptured GPU error: {error}");
            if errors.report(&message, false) {
                log::error!("{message}");
            }
        }));
        let lost = gpu_failures.clone();
        rs.device.set_device_lost_callback(move |reason, message| {
            lost.report(&format!("GPU device lost ({reason:?}): {message}"), true);
        });
    }
    let mut session = effectcraft_host::session();
    session.check_footage_on_open = true;
    session.autosave.background = true;
    if let Some(dir) = effectcraft_host::config_dir() {
        // WebAssembly effect plug-ins in <config>/Plug-ins load before the menus are built.
        let plugins = dir.join("Plug-ins");
        if plugins.is_dir() {
            let _ = session.execute("effect.plugins.load", json!({"folder": plugins.to_string_lossy()}));
        }
        session.config = Some(std::sync::Arc::new(effectcraft_engine::config::DirConfig::new(dir)));
    }
    session.load_settings();
    let recovery = session.begin_recovery();
    let show_home = session.prefs.startup.show_home_on_launch;
    let _ = session.execute("window.restoreScriptPanels", json!({}));
    let mut app = EffectcraftApp::new(session);
    app.set_gpu_failure_bridge(gpu_failures);
    app.ui.start_screen = show_home;
    if let Some(r) = recovery {
        app.offer_recovery(r);
    }
    services::hooks(&mut app, &cc.egui_ctx);
    app
}

/// EffectCraft's look, as its first frame installs it (fonts for the interface language, colours).
pub fn restyle(app: &EffectcraftApp, ctx: &egui::Context) {
    let language = match app.session.prefs.general.language.as_str() {
        "system" => "en",
        l => l,
    };
    effectcraft_ui_egui::theme::install(ctx, &app.tokens, language);
}

pub fn has_unsaved(app: &EffectcraftApp) -> bool {
    app.session.is_dirty()
}
