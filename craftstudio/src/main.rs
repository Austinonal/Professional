//! CraftStudio: PhotoCraft (images) and FilmCraft (video) in one window.
//!
//! Both apps are hosted as-is (their own `ui-egui` crates); this shell only adds a Home screen
//! with plain-language starting points and a three-way switch (Home · Photo · Video) that is
//! always visible, so it is clear which editor you are in.

mod photo_services;

use std::path::PathBuf;

use eframe::egui;
use filmcraft_ui_egui::FilmcraftApp;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Home,
    Photo,
    Video,
}

struct Studio {
    mode: Mode,
    photo: PhotocraftApp,
    video: FilmcraftApp,
    /// The editor whose look (fonts, colours) the context has now; switching re-installs it.
    styled_for: Mode,
    /// A Home-screen action to run once its editor is showing: (mode, command id, params).
    pending: Option<(Mode, &'static str, Value)>,
    /// Paths given on the command line: images open in Photo, everything else in Video.
    launch: Vec<PathBuf>,
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x8c, 0xff);
const VIDEO_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x5c, 0xf6);
const PHOTO_EXTS: &[&str] = &[
    "psd", "psb", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "heic", "heif",
    "avif", "dng", "cr2", "nef", "arw", "pcraft", "svg", "exr",
];

impl Studio {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut photo =
            PhotocraftApp::new(photocraft_engine::Session::new(), photo_services::native());
        photo.background_jobs = true;
        if let Some(rs) = cc.wgpu_render_state.clone() {
            photo.set_wgpu(rs);
        }
        let mut video = FilmcraftApp::new(filmcraft_engine::Session::default());
        if let Some(rs) = cc.wgpu_render_state.clone() {
            video.set_wgpu(rs);
        }
        video_hooks(&mut video);
        // The Home screen uses PhotoCraft's look until an editor is opened.
        PhotocraftApp::setup_context(&cc.egui_ctx, photo.ui.theme);
        let launch: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        Self {
            mode: Mode::Home,
            photo,
            video,
            styled_for: Mode::Photo,
            pending: None,
            launch,
        }
    }

    fn switch(&mut self, ctx: &egui::Context, mode: Mode) {
        self.mode = mode;
        if mode == Mode::Home || mode == self.styled_for {
            return;
        }
        // Both editors install their own fonts and colours into the shared context; the fonts use
        // the same family names, so re-installing the one being shown is enough.
        match mode {
            Mode::Photo => PhotocraftApp::setup_context(ctx, self.photo.ui.theme),
            Mode::Video => filmcraft_ui_egui::theme::install(ctx, &self.video.tokens),
            Mode::Home => {}
        }
        self.styled_for = mode;
    }

    fn open_launch_files(&mut self, ctx: &egui::Context) {
        let (photos, videos): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.launch).into_iter().partition(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| PHOTO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            });
        let s = |v: &[PathBuf]| {
            v.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        if !videos.is_empty() {
            self.switch(ctx, Mode::Video);
            self.pending = Some((Mode::Video, "file.import", json!({"paths": s(&videos)})));
        }
        if !photos.is_empty() {
            // Photos win the first screen; a video import (if any) waits until Video is opened.
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
                            Mode::Photo,
                            "🖼  Photo",
                            "Edit pictures: retouch, layers, PSD (PhotoCraft)",
                            ACCENT,
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
                        .min_size(egui::vec2(96.0, 28.0));
                        if ui.add(button).on_hover_text(tip).clicked() && !on {
                            self.switch(&ctx, mode);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let hint = match self.mode {
                            Mode::Home => "Pick Photo or Video to begin",
                            Mode::Photo => {
                                "Photo editor · switch to Video any time, your work stays open"
                            }
                            Mode::Video => {
                                "Video editor · switch to Photo any time, your work stays open"
                            }
                        };
                        ui.weak(hint);
                    });
                });
            });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.12).min(80.0));
                ui.heading(egui::RichText::new("What do you want to make?").size(26.0));
                ui.add_space(4.0);
                ui.weak("Both editors stay open side by side; the switch at the top always takes you back.");
                ui.add_space(28.0);
            });
            let card_w = 340.0_f32.min((ui.available_width() - 60.0) / 2.0).max(220.0);
            let total = card_w * 2.0 + 24.0;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                let photo = card(ui, card_w, ACCENT, "🖼", "Photo", "Retouch pictures, work with layers,\nopen and save PSD, PNG, JPEG…", &[
                    ("New image", "file.new"),
                    ("Open an image…", "file.open"),
                ]);
                ui.add_space(24.0);
                let video = card(ui, card_w, VIDEO_ACCENT, "🎬", "Video", "Cut clips on a timeline, add titles\nand sound, export a movie.", &[
                    ("Add video clips…", "file.import"),
                    ("Open a project…", "file.open"),
                    ("Try the demo project", "file.openDemoProject"),
                ]);
                if let Some(id) = photo {
                    self.switch(&ctx, Mode::Photo);
                    self.pending = Some((Mode::Photo, id, json!({})));
                }
                if let Some(id) = video {
                    self.switch(&ctx, Mode::Video);
                    self.pending = Some((Mode::Video, id, json!({})));
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

/// FilmCraft's file dialogs (the same rfd calls its own app makes).
fn video_hooks(app: &mut FilmcraftApp) {
    let save = |filter: &str, exts: &[&str], name: &str| {
        rfd::FileDialog::new()
            .add_filter(filter, exts)
            .set_file_name(name)
            .save_file()
    };
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    app.hooks.pick_files = Some(Box::new(move |exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter("Media", exts)
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(s)
            .collect()
    }));
    app.hooks.pick_file_for_relink = Some(Box::new(move |exts: &[&str], _| {
        rfd::FileDialog::new()
            .add_filter("Media", exts)
            .pick_file()
            .map(s)
    }));
    app.hooks.pick_save = Some(Box::new(move |name: &str| {
        save("FilmCraft Project", &["fcproj"], name).map(s)
    }));
    app.hooks.pick_save_as = Some(Box::new(move |filter: &str, exts: &[&str], name: &str| {
        let path = std::path::Path::new(name);
        let file = path
            .file_name()
            .map_or(name.into(), |n| n.to_string_lossy().into_owned());
        save(filter, exts, &file).map(s)
    }));
    app.hooks.pick_open_project = Some(Box::new(move || {
        rfd::FileDialog::new()
            .add_filter("FilmCraft Project", &["fcproj"])
            .pick_file()
            .map(s)
    }));
    app.hooks.pick_open_file = Some(Box::new(move |filter: &str, exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter(filter, exts)
            .pick_file()
            .map(s)
    }));
    app.hooks.pick_folder = Some(Box::new(move || {
        rfd::FileDialog::new().pick_folder().map(s)
    }));
    app.hooks.pick_folder_at = Some(Box::new(move |dir: &str| {
        rfd::FileDialog::new()
            .set_directory(dir)
            .pick_folder()
            .map(s)
    }));
}

impl eframe::App for Studio {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        match self.mode {
            Mode::Photo => self.photo.raw_input_hook(ctx, raw_input),
            Mode::Video => self.video.raw_input_hook(ctx, raw_input),
            Mode::Home => {}
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.launch.is_empty() {
            self.open_launch_files(ctx);
        }
        match self.mode {
            Mode::Photo => self.photo.logic(ctx, frame),
            Mode::Video => self.video.logic(ctx, frame),
            Mode::Home => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.top_bar(ui);
        // The editors lay out over their ui's whole max_rect: give them only the area below the bar.
        let rect = ui.available_rect_before_wrap();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect);
            match self.mode {
                Mode::Home => self.home(ui),
                Mode::Photo => self.photo.ui(ui, frame),
                Mode::Video => self.video.ui(ui, frame),
            }
        });
        if self.mode != Mode::Home {
            let ctx = ui.ctx().clone();
            self.run_pending(&ctx);
        }
    }

    fn on_exit(&mut self) {
        self.photo.on_exit();
        self.video.on_exit();
    }
}

fn main() -> eframe::Result {
    env_logger_lite();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CraftStudio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "CraftStudio",
        options,
        Box::new(|cc| Ok(Box::new(Studio::new(cc)))),
    )
}

/// Warnings and errors to stderr (no logger dependency for this).
fn env_logger_lite() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Warn
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
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
