//! FigCraft (interface design and prototyping) inside CraftStudio, started as the FigCraft app
//! starts it: system fonts indexed in the background, live (collaboration) sessions, documents a
//! crashed run left come back, its preferences, plugins and invite links, and FigCraft's own
//! services (`crate::fg`). FigCraft keeps its app in a global (`figcraft_ui_egui::windows`).

use std::sync::{Arc, Mutex};

use crate::{fg::services, recovery};
use figcraft_engine::Session;
use figcraft_ui_egui::FigApp;
use figcraft_ui_egui::windows::with_app;
use serde_json::json;

pub struct Fig {
    /// The window's context, for the font scan and network threads to wake it.
    ctx_slot: Arc<Mutex<Option<egui::Context>>>,
    /// Shared with the services, which write the copies after a panic.
    recovery: recovery::Shared,
}

/// With nothing to open, whether FigCraft starts in a new file rather than on its home screen
/// (as FigCraft: a first run shows the home screen's tips and templates).
fn opens_new_file(first_run: bool, start_on_home: bool) -> bool {
    !first_run && !start_on_home
}

impl Fig {
    pub fn new() -> Self {
        let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
        let fonts_slot = ctx_slot.clone();
        std::thread::spawn(move || {
            figcraft_text::scan_system_fonts();
            if let Some(c) = fonts_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                c.request_repaint();
            }
        });
        let mut session = Session::empty();
        let wake_slot = ctx_slot.clone();
        session.live.install(Box::new(figcraft_collab_net::NetLink::new(Arc::new(move || {
            if let Some(c) = wake_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                c.request_repaint();
            }
        }))));
        let recovery_root = services::config_dir().map(|d| d.join("Recovery"));
        let recovery = recovery::Shared::default();
        let svc = services::Desktop::load(recovery.clone());
        if let Some(prefs) = svc.prefs.clone() {
            session.prefs = prefs;
        }
        let recovered = recovery_root.as_deref().map(|r| recovery::recover_all(r, &mut session)).unwrap_or_default();
        let mut copies = recovery_root.as_deref().and_then(recovery::Recovery::start);
        if let Some(r) = copies.as_mut() {
            r.adopt(recovered.waiting);
        }
        *recovery::lock(&recovery) = copies;
        session.plugins.install_dir = services::config_dir().map(|d| d.join("Plugins"));
        session.live.links = figcraft_collab_net::links::system();
        session.invite_links_startup();
        if session.docs.is_empty() && opens_new_file(svc.prefs.is_none(), session.prefs.start_on_home) {
            let _ = session.execute("file.new", &json!({}));
        }
        figcraft_ui_egui::i18n::set(&session.prefs.language, figcraft_ui_egui::Services::system_language(&svc).as_deref());
        let mut fig = FigApp::new(session, Box::new(svc));
        fig.ui.home = false;
        fig.note_recovered(recovered.copies);
        figcraft_ui_egui::windows::install(fig);
        Self { ctx_slot, recovery }
    }

    /// Every frame, shown or not: recovery copies keep being written.
    pub fn logic(&mut self, ctx: &egui::Context, shown: bool) {
        {
            let mut slot = self.ctx_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        if shown {
            // Closing the window asks about unsaved changes.
            with_app(|fig| figcraft_ui_egui::windows::on_close_request(fig, ctx));
        }
        let now = ctx.input(|i| i.time);
        if let Some(wait) = with_app(|fig| recovery::lock(&self.recovery).as_mut().and_then(|r| r.tick(&fig.session, now))).flatten() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }

    pub fn raw_input_hook(&mut self, raw_input: &mut egui::RawInput) {
        if let Some(events) = with_app(|fig| figcraft_ui_egui::windows::take_input(fig, raw_input.viewport_id)) {
            raw_input.events.extend(events);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let ctx = ui.ctx().clone();
        services::parented(frame, || with_app(|fig| figcraft_ui_egui::windows::guard(fig, &ctx, |fig| fig.ui(ui))));
    }

    /// FigCraft's look: it sets its colours every frame, its fonts only when they change.
    // shortcut: CJK faces for names in documents come back on FigCraft's next font change, not at
    // once; upgrade if FigCraft exposes its script-font state.
    pub fn restyle(ctx: &egui::Context) {
        figcraft_ui_egui::theme::install_fonts(ctx, figcraft_text::system_fonts_scanned(), 0);
    }

    pub fn has_unsaved() -> bool {
        with_app(|fig| fig.session.docs.iter().any(|d| d.is_dirty())).unwrap_or(false)
    }

    pub fn exit(&mut self) {
        let copies = recovery::lock(&self.recovery).take();
        with_app(|fig| services::shutdown(fig, services::config_dir().as_deref(), copies));
    }
}
