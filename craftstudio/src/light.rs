//! LightCraft (photo library and raw developer) inside CraftStudio. The app is LightCraft's own
//! `LightcraftApp`; its library and settings are the ones the LightCraft app uses, and its
//! platform services are LightCraft's (`light_services.rs`). Edit in PhotoCraft opens the photo
//! in this window's Photo tab instead of starting another program.

#[path = "../../lightcraft/apps/lightcraft/src/appearance.rs"]
mod appearance;
#[cfg(target_os = "linux")]
#[path = "../../lightcraft/apps/lightcraft/src/colord.rs"]
mod colord;
#[path = "../../lightcraft/apps/lightcraft/src/dialog_filter.rs"]
mod dialog_filter;
#[path = "../../lightcraft/apps/lightcraft/src/editors.rs"]
mod editors;
#[path = "light_services.rs"]
mod services;

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use lightcraft_engine::Session;
use lightcraft_ui_egui::panels::library_problem::LibraryProblem;
use lightcraft_ui_egui::{LightcraftApp, UiState};

/// How long quitting waits for renders that are running (as the LightCraft app).
const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct Light {
    pub app: LightcraftApp,
    /// `<config>/ui.json`; `None` when it must not be written (unreadable at launch, no home).
    prefs: Option<PathBuf>,
    written: Vec<u8>,
    checked: f64,
}

/// Whether Settings ▸ External Editing's editor is PhotoCraft (then it is this window's Photo tab).
fn is_photocraft(app: &str) -> bool {
    app.trim()
        .eq_ignore_ascii_case(lightcraft_ui_egui::state::PHOTOCRAFT)
}

impl Light {
    pub fn new(cc: &eframe::CreationContext<'_>, to_photo: Sender<String>) -> Self {
        let prefs_path = lightcraft_engine::config::config_dir().map(|d| d.join("ui.json"));
        let (saved, keep_file) = match prefs_path.as_ref().map(std::fs::read) {
            Some(Ok(bytes)) => match serde_json::from_slice::<UiState>(&bytes) {
                Ok(ui) => (Some(ui.sanitized()), false),
                // A damaged settings file is left as it is: defaults now, nothing written over it.
                Err(e) => {
                    log::warn!(
                        "LightCraft settings unreadable ({e}); using defaults and not saving them"
                    );
                    (None, true)
                }
            },
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => (None, true),
            _ => (None, false),
        };
        // The library the settings name (Settings ▸ Open Library…), else LightCraft's default.
        let dir = saved
            .as_ref()
            .map(|ui| ui.settings.library_path.clone())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(lightcraft_engine::library::default_dir);
        let (mut session, problem) = open_library(dir);
        session.face_models_dir = lightcraft_engine::config::default_face_models_dir();
        session.slide_templates_dir = lightcraft_engine::config::config_dir()
            .map(|d| d.join(lightcraft_engine::slide_template::DIR_NAME));
        session.apple_photos_background = true;
        let session = session.with_default_sam3_model();

        let mut services = services::services(cc, None);
        route_to_photo(&mut services, to_photo);
        let mut app = LightcraftApp::new(session, services);
        if let Some(ui) = saved {
            app.ui = ui;
        }
        lightcraft_ui_egui::i18n::set_language(app.ui.language);
        app.gpu_notice = true;
        match problem {
            Some(p) => app.library_problem = Some(p),
            None => lightcraft_ui_egui::panels::empty_library::open_on_empty_library(&mut app),
        }
        let written = serde_json::to_vec_pretty(&app.ui).unwrap_or_default();
        Self {
            app,
            prefs: prefs_path.filter(|_| !keep_file),
            written,
            checked: 0.0,
        }
    }

    /// The settings, written when they changed (checked every 3 s, as the LightCraft app does).
    pub fn tick(&mut self, now: f64) {
        if now - self.checked < 3.0 {
            return;
        }
        self.checked = now;
        self.save();
    }

    fn save(&mut self) {
        // Never while the library couldn't be opened: the settings would point at the temporary one.
        let Some(path) = self
            .prefs
            .as_ref()
            .filter(|_| self.app.library_problem.is_none())
        else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.app.ui) else {
            return;
        };
        if bytes == self.written {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match lightcraft_engine::export::write_file(&path.to_string_lossy(), &bytes) {
            Ok(()) => self.written = bytes,
            Err(e) => log::warn!("saving the LightCraft settings failed: {e}"),
        }
    }

    pub fn exit(&mut self) {
        self.save();
        if let Err(e) = self.app.session.close_library() {
            log::error!("saving the library failed: {e}");
        }
        if !self.app.shutdown(EXIT_WAIT) {
            log::warn!("quit: LightCraft background work was still running after {EXIT_WAIT:?}");
        }
    }
}

/// Edit in PhotoCraft hands the copy to this window's Photo tab; other editors start as in LightCraft.
fn route_to_photo(services: &mut lightcraft_ui_egui::Services, to_photo: Sender<String>) {
    let tx = to_photo.clone();
    services.open_with = Some(Box::new(move |path: &str, app: &str| {
        if is_photocraft(app) {
            tx.send(path.to_string()).map_err(|e| e.to_string())
        } else {
            editors::open_with(path, app)
        }
    }));
    services.open_all_with = Some(Box::new(move |paths: &[String], app: &str| {
        if is_photocraft(app) {
            paths
                .iter()
                .try_for_each(|p| to_photo.send(p.clone()).map_err(|e| e.to_string()))
        } else {
            editors::open_all_with(paths, app)
        }
    }));
    services.check_editor = Some(Box::new(|app: &str| {
        if is_photocraft(app) {
            Ok(())
        } else {
            editors::check(app)
        }
    }));
}

/// The library session at `dir`; if it can't be opened, an empty one in memory and why (the
/// window then asks what to do, as in LightCraft).
fn open_library(dir: Option<PathBuf>) -> (Session, Option<LibraryProblem>) {
    let base = || {
        Session::new()
            .with_fs()
            .with_default_denoise_models()
            .with_system_clock()
    };
    let Some(dir) = dir else {
        return (
            base(),
            Some(LibraryProblem::new(
                "",
                "There is no home folder to keep the library in. Choose a folder for it.",
            )),
        );
    };
    let mut s = base();
    match s.open_library(&dir, true) {
        Ok(_) => (s, None),
        Err(e) => {
            log::error!("can't open library {}: {e}", dir.display());
            (
                base(),
                Some(LibraryProblem::new(dir.to_string_lossy(), e.to_string())),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_in_photocraft_lands_in_the_photo_tab() {
        let dir = std::env::temp_dir().join(format!("craftstudio-editin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut services = lightcraft_ui_egui::Services::default();
        super::route_to_photo(&mut services, tx);
        let mut app = lightcraft_ui_egui::LightcraftApp::new(
            lightcraft_engine::Session::with_demo().with_fs(),
            services,
        );
        app.run(
            "photo.editInPhotoCraft",
            serde_json::json!({"dir": dir.to_string_lossy()}),
        )
        .unwrap();
        let path = rx.try_recv().expect("the copy goes to the Photo tab");
        assert!(
            path.ends_with("-Edit.psd") && std::path::Path::new(&path).exists(),
            "{path}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
