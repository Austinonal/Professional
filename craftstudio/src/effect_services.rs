//! EffectCraft's platform services, copied unchanged from `effectcraft/apps/effectcraft/src/main.rs`
//! (the app's hooks, its non-blocking file dialogs and the window's wgpu options), so every hook
//! the EffectCraft app wires up works here too. Re-copy it when upstream changes.

use super::{appearance, audio_out};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::sheet_dialogs;
use effectcraft_ui_egui::EffectcraftApp;

/// The app's file dialogs, audio output, system theme and lost-device message.
pub(super) fn hooks(app: &mut EffectcraftApp, ctx: &egui::Context) {
    app.hooks.pick_files = Some(Box::new(|exts: &[&str]| {
        rfd::FileDialog::new().add_filter("Media", exts).pick_files().unwrap_or_default().into_iter().map(|p| p.to_string_lossy().to_string()).collect()
    }));
    app.hooks.pick_save = Some(Box::new(|name: &str| {
        rfd::FileDialog::new().add_filter("EffectCraft Project", &["ecproj"]).set_file_name(name).save_file().map(|p| p.to_string_lossy().to_string())
    }));
    // The window closes after a lost graphics device (#591); say why, unless an agent drives
    // the app through the control channel.
    {
        app.hooks.device_lost = Some(Box::new(|saved: Option<Result<String, String>>| {
            let saved = match saved {
                Some(Ok(path)) => format!("\n\nYour unsaved changes were saved to:\n{path}\n\nReopen EffectCraft to recover them."),
                Some(Err(e)) => format!("\n\nYour unsaved changes could not be saved ({e})."),
                None => String::new(),
            };
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("EffectCraft: graphics device lost")
                .set_description(format!("The graphics driver stopped responding, so EffectCraft has to close.{saved}"))
                .show();
        }));
    }
    app.hooks.pick_open_project = Some(Box::new(|| {
        rfd::FileDialog::new().add_filter("EffectCraft Project", &["ecproj", "ecprojx"]).pick_file().map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.system_theme = appearance::service();
    app.hooks.audio_device = Some(Box::new(audio_out::open));
    app.hooks.audio_devices = Some(Box::new(audio_out::devices));
    app.hooks.pick_folder = Some(Box::new(|| rfd::FileDialog::new().pick_folder().map(|p| p.to_string_lossy().to_string())));
    app.hooks.pick_save_file = Some(Box::new(|name: &str, ext: &str| {
        // A default with a folder that exists opens the dialog there.
        let path = std::path::Path::new(name);
        // (No filter for a name without an extension, such as a folder to create.)
        let mut d = if ext.is_empty() { rfd::FileDialog::new() } else { rfd::FileDialog::new().add_filter(ext, &[ext]) };
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        d.set_file_name(file).save_file().map(|p| p.to_string_lossy().to_string())
    }));
    // The commands' file dialogs don't block the frame where the platform's backend
    // would (#511, #1275); see [`install_file_dialogs`].
    install_file_dialogs(app, ctx);
}

fn install_file_dialogs(app: &mut EffectcraftApp, ctx: &egui::Context) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let ctx = ctx.clone();
        app.hooks.file_dialog_async = Some(Box::new(move |dialog, done| sheet_dialogs::show(dialog, done, ctx.clone())));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (app, ctx);
}


/// The window's wgpu device asks for the adapter's own limits (textures up to 16384 px)
/// instead of eframe's fixed set: an older GPU that can't meet those (Intel HD Graphics 5500,
/// #198) still opens the app, and the compositor then checks what it needs and falls back to
/// the CPU when the device can't run it. `backends`: Window Graphics' choice, when it makes one.
pub(crate) fn wgpu_options(backends: Option<eframe::wgpu::Backends>) -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::WgpuSetup;
    let mut config = eframe::WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        if let Some(b) = backends {
            setup.instance_descriptor.backends = b;
        }
        // Nothing draws or dispatches indirectly, so wgpu's check of indirect arguments only costs
        // a compute shader when the device is created, one that Metal can't compile on an Intel
        // Iris Pro: the device is lost and the window never opens.
        // `WGPU_VALIDATION_INDIRECT_CALL=1` turns the check back on.
        setup.instance_descriptor.flags = setup.instance_descriptor.flags.difference(eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
        setup.device_descriptor = std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("EffectCraft device"),
            required_limits: device_limits(adapter.limits()),
            ..Default::default()
        });
    }
    config
}

/// The limits to request from an adapter that offers `adapter`: all of them, with textures
/// capped at 16384 px like the headless renderer's device.
fn device_limits(adapter: eframe::wgpu::Limits) -> eframe::wgpu::Limits {
    eframe::wgpu::Limits { max_texture_dimension_2d: adapter.max_texture_dimension_2d.min(16384), ..adapter }
}
