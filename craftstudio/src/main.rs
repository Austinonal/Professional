// Release builds on Windows open no console window beside the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! CraftStudio: LightCraft (photo library), PhotoCraft (images), VectorCraft (illustration) and
//! FilmCraft (video) in one window.
//!
//! All four apps are hosted as-is (their own `ui-egui` crates); this shell only adds a Home screen
//! with plain-language starting points and a switch (Home · Library · Photo · Vector · Video)
//! that is always visible, so it is clear which editor you are in. Where the apps hand work to
//! each other (LightCraft's Edit in PhotoCraft, VectorCraft's Edit Original), it lands in the
//! Photo tab of the same window.

mod effect;
mod fig;
mod film;
mod light;
mod vector;

// PhotoCraft's platform modules (file dialogs, open/save, clipboard, crash recovery, settings),
// compiled as they are. Declared by name inside a module pointing at PhotoCraft's `src`, so their
// own `mod tests;` files resolve, and re-exported at the root for their `crate::…` paths.
#[path = "../../photocraft/apps/photocraft/src"]
mod pc {
    pub mod app_dirs;
    pub mod appearance;
    // Partly used: the rest serves PhotoCraft's own launcher.
    #[allow(dead_code)]
    pub mod crash_guard;
    pub mod file_dialog_probe;
    #[cfg(any(target_os = "linux", test))]
    #[allow(dead_code)]
    pub mod linux_libs;
    pub mod photoshop_settings;
    #[cfg(target_os = "linux")]
    pub mod portal_save;
    #[allow(dead_code)]
    pub mod services;
}
use pc::{app_dirs, appearance, crash_guard, file_dialog_probe, photoshop_settings, services};
#[cfg(any(target_os = "linux", test))]
use pc::linux_libs;
#[cfg(target_os = "linux")]
use pc::portal_save;

// FigCraft's platform modules, compiled as they are and declared as PhotoCraft's are (above);
// its services name `crate::gamepad` and `crate::recovery`.
#[path = "../../figcraft/apps/figcraft/src"]
mod fg {
    pub mod gamepad;
    // Some helpers serve only FigCraft's own test files, which live in its main.rs.
    #[cfg_attr(test, allow(dead_code))]
    pub mod recovery;
    // Partly used: its window icon serves FigCraft's own launcher.
    #[allow(dead_code)]
    pub mod services;
}
use fg::{gamepad, recovery};

// What PhotoCraft's main.rs gives those modules' tests.
#[cfg(test)]
mod gpu_startup {
    pub const MARKER_FILE: &str = "gpu-starting.json";
}
#[cfg(test)]
pub(crate) static CHILD_SPAWNS: std::sync::RwLock<()> = std::sync::RwLock::new(());
#[cfg(test)]
pub(crate) fn no_child_spawns() -> std::sync::RwLockWriteGuard<'static, ()> {
    CHILD_SPAWNS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use filmcraft_ui_egui::FilmcraftApp;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};
use effectcraft_ui_egui::EffectcraftApp;
use vectorcraft_ui_egui::VectorcraftApp;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Home,
    Library,
    Photo,
    Vector,
    Design,
    Motion,
    Video,
}

impl Mode {
    /// The editors, in tab order.
    const EDITORS: [Mode; 6] = [Mode::Library, Mode::Photo, Mode::Vector, Mode::Design, Mode::Motion, Mode::Video];

    fn name(self) -> &'static str {
        match self {
            Mode::Home => "Home",
            Mode::Library => "Library",
            Mode::Photo => "Photo",
            Mode::Vector => "Vector",
            Mode::Design => "Design",
            Mode::Motion => "Motion",
            Mode::Video => "Video",
        }
    }
}

struct Studio {
    mode: Mode,
    light: light::Light,
    photo: PhotocraftApp,
    vector: VectorcraftApp,
    fig: fig::Fig,
    motion: EffectcraftApp,
    video: FilmcraftApp,
    /// The editor whose look (fonts, colours) the context has now; switching re-installs it.
    styled_for: Mode,
    /// New fonts take effect a frame later: the editor waits that frame before drawing.
    settling: bool,
    /// Files the other editors hand to Photo (Edit in PhotoCraft, Edit Original).
    to_photo: Receiver<String>,
    /// Closing was asked while these hidden tabs hold unsaved work (the shown one asks itself).
    unsaved_on_close: Vec<Mode>,
    /// "Quit anyway" was chosen: the next close goes ahead past the hidden tabs.
    quit_anyway: bool,
    /// A Home-screen action to run once its editor is showing: (mode, command id, params).
    pending: Option<(Mode, &'static str, Value)>,
    /// Paths given on the command line: images open in Photo, everything else in Video.
    launch: Vec<PathBuf>,
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x8c, 0xff);
const VIDEO_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x5c, 0xf6);
const LIBRARY_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x14, 0x9e, 0x82);
const VECTOR_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0x7b, 0x10);
const DESIGN_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0x3e, 0x8a);
const MOTION_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xd9, 0x45, 0x2b);
const DESIGN_EXTS: &[&str] = &["figcraft", "fig"];
const MOTION_EXTS: &[&str] = &["ecproj", "ecprojx"];
const VECTOR_EXTS: &[&str] = &[
    "vectorcraft",
    "drawcraft",
    "ai",
    "eps",
    "pdf",
    "cdr",
    "afdesign",
    "dxf",
    "wmf",
    "emf",
];
const PHOTO_EXTS: &[&str] = &[
    "psd", "psb", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "heic", "heif",
    "avif", "dng", "cr2", "nef", "arw", "pcraft", "svg", "exr",
];

impl Studio {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut photo_services = services::native(None);
        photo_services.preset_store =
            services::presets_dir().map(photocraft_engine::preset_store::open_dir_async);
        #[cfg(target_os = "linux")]
        file_dialog_probe::start();
        let mut photo = PhotocraftApp::new(photocraft_engine::Session::new(), photo_services);
        photo.background_jobs = true;
        if let Some(rs) = cc.wgpu_render_state.clone() {
            photo.set_wgpu(rs);
        }
        let video = film::new(cc);
        let (tx, to_photo): (Sender<String>, _) = std::sync::mpsc::channel();
        let light = light::Light::new(cc, tx.clone());
        let vector = vector::new(cc, tx);
        let fig = fig::Fig::new();
        let motion = effect::new(cc);
        // The Home screen uses PhotoCraft's look until an editor is opened.
        PhotocraftApp::setup_context(&cc.egui_ctx, photo.ui.theme);
        let launch: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        Self {
            mode: Mode::Home,
            light,
            photo,
            vector,
            fig,
            motion,
            video,
            styled_for: Mode::Photo,
            settling: false,
            to_photo,
            unsaved_on_close: Vec::new(),
            quit_anyway: false,
            pending: None,
            launch,
        }
    }

    fn switch(&mut self, ctx: &egui::Context, mode: Mode) {
        // A hidden editor's logic doesn't run: playback there would go on with nobody watching.
        self.video.playback.playing = false;
        self.motion.playback.playing = false;
        self.mode = mode;
        self.quit_anyway = false;
        if mode == Mode::Home || mode == self.styled_for {
            return;
        }
        // Each editor installs its own fonts and colours into the shared context: re-install the
        // one being shown, and let it draw once its fonts (whose family names differ) are in.
        match mode {
            Mode::Library => {
                let kind = self
                    .light
                    .app
                    .ui
                    .settings
                    .theme(self.light.app.system_theme(ctx));
                lightcraft_ui_egui::theme::install_fonts(ctx);
                lightcraft_ui_egui::theme::apply_kind(ctx, kind);
            }
            Mode::Photo => PhotocraftApp::setup_context(ctx, self.photo.ui.theme),
            Mode::Vector => {
                vectorcraft_ui_egui::theme::install_fonts(ctx);
                vectorcraft_ui_egui::theme::apply(ctx, self.vector.ui.brightness);
            }
            Mode::Design => fig::Fig::restyle(ctx),
            Mode::Motion => effect::restyle(&self.motion, ctx),
            Mode::Video => filmcraft_ui_egui::theme::install(ctx, &self.video.tokens),
            Mode::Home => {}
        }
        self.styled_for = mode;
        self.settling = true;
    }

    /// Whether the editor behind `mode` holds work that closing would lose.
    fn has_unsaved(&self, mode: Mode) -> bool {
        match mode {
            Mode::Home => false,
            Mode::Library => {
                let s = &self.light.app.session;
                s.unsaved().is_some() || !s.activity.running_cancellable().is_empty()
            }
            Mode::Photo => self.photo.session.documents().iter().any(|d| d.is_dirty()),
            Mode::Vector => self.vector.session.documents().iter().any(|d| d.is_dirty()),
            Mode::Design => fig::Fig::has_unsaved(),
            Mode::Motion => effect::has_unsaved(&self.motion),
            Mode::Video => self.video.session.is_dirty(),
        }
    }

    /// Unsaved work in tabs that aren't showing: listed, and the window stays open until the user
    /// goes to one of them or chooses Quit anyway.
    fn quit_prompt(&mut self, ctx: &egui::Context) {
        if self.unsaved_on_close.is_empty() {
            return;
        }
        let tabs: Vec<Mode> = self.unsaved_on_close.clone();
        let mut go = None;
        let mut quit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("craftstudio-unsaved")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Unsaved work");
            ui.add_space(6.0);
            let list = tabs.iter().map(|m| m.name()).collect::<Vec<_>>().join(", ");
            ui.label(format!(
                "These tabs have changes that aren't saved: {list}. Quitting now loses them."
            ));
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for m in &tabs {
                    if ui.button(format!("Go to {}", m.name())).clicked() {
                        go = Some(*m);
                    }
                }
                if ui.button("Quit anyway").clicked() {
                    quit = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if let Some(m) = go {
            self.unsaved_on_close.clear();
            self.switch(ctx, m);
        } else if quit {
            self.unsaved_on_close.clear();
            self.quit_anyway = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if cancel {
            self.unsaved_on_close.clear();
        }
    }

    fn open_launch_files(&mut self, ctx: &egui::Context) {
        let has = |p: &PathBuf, exts: &[&str]| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
        };
        let (drawings, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.launch)
            .into_iter()
            .partition(|p| has(p, VECTOR_EXTS));
        let (designs, rest): (Vec<_>, Vec<_>) = rest.into_iter().partition(|p| has(p, DESIGN_EXTS));
        let (motions, rest): (Vec<_>, Vec<_>) = rest.into_iter().partition(|p| has(p, MOTION_EXTS));
        let (photos, videos): (Vec<_>, Vec<_>) = rest.into_iter().partition(|p| has(p, PHOTO_EXTS));
        for p in &designs {
            self.switch(ctx, Mode::Design);
            figcraft_ui_egui::windows::with_app(|fig| fig.run("file.open", json!({"path": p.to_string_lossy()})));
        }
        for p in &motions {
            self.switch(ctx, Mode::Motion);
            if let Err(e) = self.motion.session.execute("file.open", json!({"path": p.to_string_lossy()})) {
                log::error!("couldn't open {}: {e}", p.display());
            }
        }
        for p in &drawings {
            self.switch(ctx, Mode::Vector);
            if let Err(e) = self
                .vector
                .run("file.open", json!({"path": p.to_string_lossy()}))
            {
                log::error!("couldn't open {}: {e}", p.display());
            }
        }
        let s = |v: &[PathBuf]| {
            v.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        if !videos.is_empty() {
            self.switch(ctx, Mode::Video);
            // Now, not as a Home action: one waiting for Video would be lost to the next one.
            if let Err(e) = filmcraft_ui_egui::menus::invoke(&mut self.video, ctx, "file.import", json!({"paths": s(&videos)})) {
                log::error!("couldn't import the videos: {e}");
            }
        }
        if !photos.is_empty() {
            self.switch(ctx, Mode::Photo);
            for p in s(&photos) {
                if let Err(e) = photocraft_ui_egui::menus::invoke(
                    &mut self.photo,
                    ctx,
                    "file.open",
                    json!({"path": p}),
                ) {
                    log::error!("couldn't open {p}: {e}");
                }
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("craftstudio-switch")
            .exact_size(40.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("CraftStudio").strong().size(15.0));
                    ui.add_space(14.0);
                    let tabs = [
                        (
                            Mode::Home,
                            "🏠  Home",
                            "Start here: choose what you want to make",
                            ui.visuals().widgets.active.bg_fill,
                        ),
                        (
                            Mode::Library,
                            "📚  Library",
                            "Organize and develop your photos, raw files (LightCraft)",
                            LIBRARY_ACCENT,
                        ),
                        (
                            Mode::Photo,
                            "🖼  Photo",
                            "Edit pictures: retouch, layers, PSD (PhotoCraft)",
                            ACCENT,
                        ),
                        (
                            Mode::Vector,
                            "✏  Vector",
                            "Draw logos and illustrations, SVG, PDF (VectorCraft)",
                            VECTOR_ACCENT,
                        ),
                        (
                            Mode::Design,
                            "🎨  Design",
                            "Design apps and websites: frames, components, prototypes (FigCraft)",
                            DESIGN_ACCENT,
                        ),
                        (
                            Mode::Motion,
                            "✨  Motion",
                            "Animate titles and add visual effects (EffectCraft)",
                            MOTION_ACCENT,
                        ),
                        (
                            Mode::Video,
                            "🎬  Video",
                            "Edit videos: timeline, cuts, sound (FilmCraft)",
                            VIDEO_ACCENT,
                        ),
                    ];
                    for (mode, label, tip, color) in tabs {
                        let on = self.mode == mode;
                        let text = egui::RichText::new(label).size(14.0);
                        let button = egui::Button::new(if on {
                            text.color(egui::Color32::WHITE)
                        } else {
                            text
                        })
                        .fill(if on {
                            color
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .corner_radius(6.0)
                        .min_size(egui::vec2(88.0, 28.0));
                        if ui.add(button).on_hover_text(tip).clicked() && !on {
                            self.switch(&ctx, mode);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let hint = match self.mode {
                            Mode::Home => "Pick what you want to make",
                            Mode::Library => "Edit in PhotoCraft opens in the Photo tab",
                            _ => "Your work stays open when you switch tabs",
                        };
                        // Truncated rather than run under the tabs in a narrow window.
                        ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate());
                    });
                });
            });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.12).min(80.0));
                ui.heading(egui::RichText::new("What do you want to make?").size(26.0));
                ui.add_space(4.0);
                ui.weak("Every app stays open side by side; the switch at the top always takes you back.");
                ui.add_space(28.0);
            });
            let card_w = 320.0_f32.min((ui.available_width() - 80.0) / 3.0).max(220.0);
            let total = card_w * 3.0 + 48.0;
            let mut picked = None;
            let rows: [[(Mode, egui::Color32, &str, &str, &str, &[(&str, &'static str)]); 3]; 2] = [
                [
                    (Mode::Library, LIBRARY_ACCENT, "📚", "Library", "Bring in photos from a camera or folder,\nsort, rate and develop raw files.", &[
                        ("Import photos…", "file.addPhotos"),
                        ("Import a folder…", "file.addFolder"),
                        ("Open my library", ""),
                    ]),
                    (Mode::Photo, ACCENT, "🖼", "Photo", "Retouch pictures, work with layers,\nopen and save PSD, PNG, JPEG…", &[
                        ("New image", "file.new"),
                        ("Open an image…", "file.open"),
                    ]),
                    (Mode::Vector, VECTOR_ACCENT, "✏", "Vector", "Draw logos, icons and illustrations\nthat stay sharp at any size (SVG, PDF).", &[
                        ("New drawing", "file.newDialog"),
                        ("Open a drawing…", "file.open"),
                    ]),
                ],
                [
                    (Mode::Design, DESIGN_ACCENT, "🎨", "Design", "Design screens for apps and websites,\nwith components and clickable prototypes.", &[
                        ("New design", "file.new"),
                        ("Open a design…", "file.open"),
                    ]),
                    (Mode::Motion, MOTION_ACCENT, "✨", "Motion", "Animate titles and graphics,\nadd visual effects to footage.", &[
                        ("New animation", "comp.new"),
                        ("Open a project…", "file.open"),
                        ("Try the demo project", "file.openDemoProject"),
                    ]),
                    (Mode::Video, VIDEO_ACCENT, "🎬", "Video", "Cut clips on a timeline, add titles\nand sound, export a movie.", &[
                        ("Add video clips…", "file.import"),
                        ("Open a project…", "file.open"),
                        ("Try the demo project", "file.openDemoProject"),
                    ]),
                ],
            ];
            for row in rows {
                ui.horizontal_top(|ui| {
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    for (i, (mode, accent, icon, title, body, actions)) in row.into_iter().enumerate() {
                        if i > 0 {
                            ui.add_space(24.0);
                        }
                        if let Some(id) = card(ui, card_w, accent, icon, title, body, actions) {
                            picked = Some((mode, id));
                        }
                    }
                });
                ui.add_space(24.0);
            }
            if let Some((mode, id)) = picked {
                self.switch(&ctx, mode);
                self.pending = (!id.is_empty()).then(|| (mode, id, json!({})));
            }
            });
        });
    }

    /// Runs the queued Home action once its editor has drawn (so its fonts and dialogs are ready).
    fn run_pending(&mut self, ctx: &egui::Context) {
        let Some((mode, id, params)) = self.pending.take_if(|(m, _, _)| *m == self.mode) else {
            return;
        };
        let result = match mode {
            Mode::Photo => photocraft_ui_egui::menus::invoke(&mut self.photo, ctx, id, params),
            Mode::Video => filmcraft_ui_egui::menus::invoke(&mut self.video, ctx, id, params),
            Mode::Library => self.light.app.run(id, params),
            Mode::Design => {
                figcraft_ui_egui::windows::with_app(|fig| fig.run(id, params));
                Ok(Value::Null)
            }
            Mode::Motion => {
                // EffectCraft's own start screen would stay over what the action opened.
                self.motion.ui.start_screen = false;
                effectcraft_ui_egui::menus::invoke(&mut self.motion, ctx, id, params)
            }
            Mode::Vector => {
                vectorcraft_ui_egui::menus::invoke(&mut self.vector, id, params);
                Ok(Value::Null)
            }
            Mode::Home => Ok(Value::Null),
        };
        if let Err(e) = result {
            log::warn!("{id}: {e}");
        }
    }
}

/// A Home-screen card; returns the command id of the button clicked.
fn card(
    ui: &mut egui::Ui,
    width: f32,
    accent: egui::Color32,
    icon: &str,
    title: &str,
    body: &str,
    actions: &[(&str, &'static str)],
) -> Option<&'static str> {
    let mut clicked = None;
    egui::Frame::group(ui.style())
        .corner_radius(10.0)
        .inner_margin(20.0)
        .stroke(egui::Stroke::new(1.5, accent))
        .show(ui, |ui| {
            ui.set_width(width);
            // Cards sit in a row; their contents stack top to bottom.
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(icon).size(30.0));
                    ui.label(egui::RichText::new(title).size(22.0).strong().color(accent));
                });
                ui.add_space(6.0);
                ui.label(body);
                ui.add_space(14.0);
                for (i, (label, id)) in actions.iter().enumerate() {
                    let text = egui::RichText::new(*label).size(14.0);
                    // The first action is the main one: filled, the rest outlined.
                    let b = if i == 0 {
                        egui::Button::new(text.color(egui::Color32::WHITE)).fill(accent)
                    } else {
                        egui::Button::new(text)
                    };
                    if ui.add_sized([width, 32.0], b.corner_radius(6.0)).clicked() {
                        clicked = Some(*id);
                    }
                    ui.add_space(4.0);
                }
            });
        });
    clicked
}

impl eframe::App for Studio {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        match self.mode {
            Mode::Library => self.light.app.raw_input_hook(raw_input),
            Mode::Photo => self.photo.raw_input_hook(ctx, raw_input),
            Mode::Vector => self.vector.raw_input_hook(raw_input),
            Mode::Design => self.fig.raw_input_hook(raw_input),
            Mode::Motion => eframe::App::raw_input_hook(&mut self.motion, ctx, raw_input),
            Mode::Video => self.video.raw_input_hook(ctx, raw_input),
            Mode::Home => {}
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.launch.is_empty() {
            self.open_launch_files(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_anyway {
            let hidden: Vec<Mode> = Mode::EDITORS
                .into_iter()
                .filter(|m| *m != self.mode && self.has_unsaved(*m))
                .collect();
            if !hidden.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.unsaved_on_close = hidden;
                // The shown editor doesn't see this close: it asks about its own work on Quit anyway.
                return;
            }
        }
        let handed: Vec<String> = self.to_photo.try_iter().collect();
        if !handed.is_empty() {
            self.switch(ctx, Mode::Photo);
            for p in handed {
                if let Err(e) = photocraft_ui_egui::menus::invoke(
                    &mut self.photo,
                    ctx,
                    "file.open",
                    json!({"path": p}),
                ) {
                    log::error!("couldn't open {p} in Photo: {e}");
                }
            }
        }
        self.fig.logic(ctx, self.mode == Mode::Design);
        match self.mode {
            Mode::Library => {
                self.light.app.logic(ctx);
                self.light.tick(ctx.input(|i| i.time));
            }
            Mode::Photo => self.photo.logic(ctx, frame),
            Mode::Vector => {
                self.vector.logic(ctx);
                if self.vector.ui.status == "quit" {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Mode::Motion => eframe::App::logic(&mut self.motion, ctx, frame),
            Mode::Video => self.video.logic(ctx, frame),
            Mode::Home | Mode::Design => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.top_bar(ui);
        // The editors lay out over their ui's whole max_rect: give them only the area below the bar.
        let rect = ui.available_rect_before_wrap();
        self.quit_prompt(&ui.ctx().clone());
        if std::mem::take(&mut self.settling) {
            ui.ctx().request_repaint();
            return;
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect);
            match self.mode {
                Mode::Home => self.home(ui),
                Mode::Library => {
                    self.light.app.perf.host_frame_ms =
                        frame.info().cpu_usage.map(|s| f64::from(s) * 1000.0);
                    self.light.app.ui(ui);
                }
                Mode::Photo => self.photo.ui(ui, frame),
                Mode::Vector => {
                    self.vector.ui(ui);
                    if self.vector.take_prefs_changed() {
                        vector::save_prefs(&self.vector);
                    }
                }
                Mode::Design => self.fig.ui(ui, frame),
                Mode::Motion => eframe::App::ui(&mut self.motion, ui, frame),
                Mode::Video => self.video.ui(ui, frame),
            }
        });
        if self.mode != Mode::Home {
            let ctx = ui.ctx().clone();
            self.run_pending(&ctx);
        }
        // Each editor titles the window after itself; the title says which tab is showing.
        let title = match self.mode {
            Mode::Home => "CraftStudio".to_string(),
            m => format!("CraftStudio · {}", m.name()),
        };
        if ui.ctx().input(|i| i.viewport().title.as_deref() != Some(title.as_str())) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn on_exit(&mut self) {
        self.light.exit();
        self.photo.on_exit();
        vector::save_prefs(&self.vector);
        self.fig.exit();
        eframe::App::on_exit(&mut self.motion);
        self.video.on_exit();
    }
}

fn main() -> eframe::Result {
    effect::start();
    film::start();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CraftStudio")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        // EffectCraft's: the adapter's own limits (textures up to 16384 px), which its compositor
        // needs; the other editors run within them too.
        wgpu_options: effect::wgpu_options(None),
        ..Default::default()
    };
    eframe::run_native(
        "CraftStudio",
        options,
        Box::new(|cc| Ok(Box::new(Studio::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn photo_extensions_are_lowercase() {
        assert!(
            super::PHOTO_EXTS
                .iter()
                .all(|e| e.chars().all(|c| !c.is_ascii_uppercase()))
        );
    }
}
