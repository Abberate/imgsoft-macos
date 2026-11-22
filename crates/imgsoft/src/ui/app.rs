//! Main window: settings sidebar on the left, the batch (files, progress, results) on the right.

use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, Key, Painter, Rect, ScrollArea, Sense, Stroke, TextureHandle, Ui, UiBuilder, ViewportCommand,
    pos2, vec2,
};
use imgsoft_core::{
    Effort, FileReport, FormatChoice, Input, Job, JobKind, Outcome, Profile, QUALITY_PRESETS, Settings, SourceFormat,
    fmt, resize,
};

use super::compare::Compare;
use super::theme::{self, DARK, LIGHT, medium, pal, regular, semibold, set_palette};
use super::widgets::{self as w, ButtonKind};
use super::worker::{self, Msg, Running, Scan};

pub const APP_NAME: &str = "ImgSoft";
const SIDEBAR_W: f32 = 372.0;
const PAD: f32 = 22.0;
const KEY_SETTINGS: &str = "settings";
const RESPONSIVE_CHOICES: [u32; 7] = [480, 640, 768, 1024, 1280, 1600, 1920];
const MAX_SIDE_CHOICES: [Option<u32>; 7] =
    [None, Some(3840), Some(2560), Some(1920), Some(1600), Some(1280), Some(1024)];

enum Row {
    Waiting,
    Running,
    Done(Box<FileReport>),
}

struct Finished {
    elapsed_ms: u64,
    cancelled: bool,
}

/// Test hooks: screenshots of real runs (see README, "Vérifications visuelles").
#[derive(Default)]
struct Automation {
    screenshot: Option<PathBuf>,
    frames: u32,
    autorun: bool,
    compare: Option<usize>,
    done_at: Option<u32>,
}

pub struct ImgSoftApp {
    settings: Settings,
    source: Option<Input>,
    output: Option<PathBuf>,
    output_custom: bool,
    jobs: Vec<Job>,
    planned_for: Option<(Input, Option<PathBuf>, bool, bool)>,
    scan: Option<Scan>,
    rows: Vec<Row>,
    run: Option<Running>,
    finished: Option<Finished>,
    compare: Option<Compare>,
    toast: Option<(String, f64)>,
    sidebar_h: f32,
    about: bool,
    logo: TextureHandle,
    persist: bool,
    auto: Automation,
}

pub(super) fn logo_texture(ctx: &egui::Context) -> TextureHandle {
    let img = imgsoft_core::load_bitmap_from_memory(include_bytes!("../../../../packaging/icon_1024.png"))
        .map(|b| resize::resize(&b, 128, 128))
        .map(|b| egui::ColorImage::from_rgba_unmultiplied([128, 128], &b.rgba))
        .unwrap_or_else(|_| egui::ColorImage::filled([1, 1], Color32::TRANSPARENT));
    ctx.load_texture("logo", img, egui::TextureOptions::LINEAR)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

impl ImgSoftApp {
    pub fn new(cc: &eframe::CreationContext<'_>, test_run: bool) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_visuals(&cc.egui_ctx);
        let settings = if test_run {
            Settings::default()
        } else {
            cc.storage
                .and_then(|s| s.get_string(KEY_SETTINGS))
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default()
        };
        let mut app = Self {
            settings,
            source: None,
            output: None,
            output_custom: false,
            jobs: Vec::new(),
            planned_for: None,
            scan: None,
            rows: Vec::new(),
            run: None,
            finished: None,
            compare: None,
            toast: None,
            sidebar_h: 0.0,
            about: false,
            logo: logo_texture(&cc.egui_ctx),
            persist: !test_run,
            auto: Automation {
                screenshot: env_path("IMGSOFT_SCREENSHOT"),
                frames: 0,
                autorun: std::env::var_os("IMGSOFT_AUTORUN").is_some(),
                compare: std::env::var("IMGSOFT_COMPARE").ok().and_then(|v| v.parse().ok()),
                done_at: None,
            },
        };
        if let Some(src) = env_path("IMGSOFT_SOURCE") {
            app.set_source(if src.is_dir() { Input::Folder(src) } else { Input::Files(vec![src]) }, &cc.egui_ctx);
        }
        if let Some(out) = env_path("IMGSOFT_OUTPUT") {
            app.output = Some(out);
            app.output_custom = true;
        }
        if let Ok(f) = std::env::var("IMGSOFT_FORMAT") {
            app.settings.format = match f.as_str() {
                "avif" => FormatChoice::Avif,
                "webp" => FormatChoice::Webp,
                "classic" => FormatChoice::Classic,
                "keep" => FormatChoice::Keep,
                _ => FormatChoice::Auto,
            };
        }
        app
    }

    fn locked(&self) -> bool {
        self.run.is_some()
    }

    // --- State changes --------------------------------------------------------------------

    fn set_source(&mut self, input: Input, ctx: &egui::Context) {
        if !self.output_custom {
            self.output = default_output(&input);
        }
        self.source = Some(input);
        self.rows.clear();
        self.finished = None;
        self.rescan(ctx);
    }

    fn rescan(&mut self, ctx: &egui::Context) {
        let Some(input) = self.source.clone() else { return };
        let out = self.output.clone().unwrap_or_else(|| std::env::temp_dir().join("imgsoft-none"));
        self.planned_for =
            Some((input.clone(), self.output.clone(), self.settings.slug_names, self.settings.copy_other_files));
        self.scan = Some(worker::scan(input, out, self.settings.clone(), ctx.clone()));
    }

    fn pick_source(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Dossier d'images à compresser").pick_folder() {
            self.set_source(Input::Folder(dir), ctx);
        }
    }

    fn pick_output(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new().set_title("Dossier où enregistrer les images optimisées");
        if let Some(dir) = self.output.as_ref().and_then(|o| o.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.output = Some(dir);
            self.output_custom = true;
            self.rescan(ctx);
        }
    }

    fn start(&mut self, ctx: &egui::Context, now: f64) {
        let (Some(input), Some(output)) = (self.source.clone(), self.output.clone()) else { return };
        match imgsoft_core::batch::plan(&input, &output, &self.settings) {
            Ok(jobs) if jobs.is_empty() => self.toast("Aucune image à compresser dans ce dossier", now),
            Ok(jobs) => {
                self.rows = jobs.iter().map(|_| Row::Waiting).collect();
                self.jobs = jobs.clone();
                self.finished = None;
                self.compare = None;
                self.run = Some(worker::run(jobs, self.settings.clone(), output, ctx.clone()));
            }
            Err(e) => self.toast(&capitalize(&e.to_string()), now),
        }
    }

    fn toast(&mut self, text: &str, now: f64) {
        self.toast = Some((text.to_owned(), now));
    }

    fn poll(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(scan) = &self.scan
            && let Ok(result) = scan.rx.try_recv()
        {
            self.scan = None;
            match result {
                Ok(jobs) => {
                    // A new file list: earlier results no longer line up with it.
                    if self.run.is_none() {
                        self.rows.clear();
                        self.finished = None;
                        self.compare = None;
                    }
                    self.jobs = jobs;
                }
                Err(e) => {
                    self.jobs.clear();
                    self.toast(&capitalize(&e), now);
                }
            }
        }
        // Settings that change the file list (names, copied files) → plan again.
        if let (Some(input), None) = (&self.source, &self.run) {
            let key = (input.clone(), self.output.clone(), self.settings.slug_names, self.settings.copy_other_files);
            if self.planned_for.as_ref() != Some(&key) {
                self.rescan(ctx);
            }
        }
        let mut done = false;
        if let Some(run) = &self.run {
            while let Ok(msg) = run.rx.try_recv() {
                match msg {
                    Msg::Started(i) => {
                        if let Some(r) = self.rows.get_mut(i) {
                            *r = Row::Running;
                        }
                    }
                    Msg::Finished(report) => {
                        let i = report.index;
                        if let Some(r) = self.rows.get_mut(i) {
                            *r = Row::Done(report);
                        }
                    }
                    Msg::Done => done = true,
                }
            }
            if done {
                self.finished = Some(Finished {
                    elapsed_ms: run.started.elapsed().as_millis() as u64,
                    cancelled: run.cancelling(),
                });
            }
        }
        if done {
            self.run = None;
            for r in &mut self.rows {
                if matches!(r, Row::Running | Row::Waiting) {
                    *r = Row::Waiting;
                }
            }
        }
    }

    fn reports(&self) -> impl Iterator<Item = &FileReport> {
        self.rows.iter().filter_map(|r| if let Row::Done(rep) = r { Some(rep.as_ref()) } else { None })
    }

    fn open_compare(&mut self, index: usize, ctx: &egui::Context) {
        let (Some(job), Some(Row::Done(report)), Some(out)) =
            (self.jobs.get(index), self.rows.get(index), &self.output)
        else {
            return;
        };
        let Some(rel) = &report.output else { return };
        if !matches!(report.outcome, Outcome::Compressed | Outcome::KeptOriginal) {
            return;
        }
        self.compare = Some(Compare::new(report, job.source.clone(), out.join(rel), ctx));
    }

    fn handle_drops(&mut self, ui: &Ui) -> bool {
        let (hovering, dropped) = ui.input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(),
            )
        });
        if !dropped.is_empty() && !self.locked() {
            let input = match dropped.as_slice() {
                [one] if one.is_dir() => Input::Folder(one.clone()),
                many => Input::Files(many.iter().filter(|p| p.is_file()).cloned().collect()),
            };
            self.set_source(input, ui.ctx());
        }
        hovering && !self.locked()
    }

    // --- Drawing ---------------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut Ui, area: Rect, now: f64) {
        let pl = pal();
        let p = ui.painter().clone();
        p.rect_filled(area, 0.0, pl.surface);
        p.line_segment([area.right_top(), area.right_bottom()], Stroke::new(1.0, pl.stroke));
        let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - 104.0), area.max);
        let scroll = Rect::from_min_max(area.min, pos2(area.right() - 1.0, footer.top()));
        let content_h = self.sidebar_h.max(scroll.height());
        ui.scope_builder(UiBuilder::new().max_rect(scroll), |ui| {
            ScrollArea::vertical()
                .id_salt("sidebar")
                .auto_shrink([false; 2])
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), content_h), Sense::hover());
                    let p = ui.painter().clone();
                    let bottom = self.sidebar_content(ui, &p, rect);
                    self.sidebar_h = bottom - rect.top() + 12.0;
                });
        });
        // The title bar is hidden: the strip above the brand moves the window.
        let strip = Rect::from_min_size(area.min, vec2(area.width(), 38.0));
        if ui.interact(strip, egui::Id::unique("drag-sidebar"), Sense::drag()).drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        // Fade where the settings scroll under the footer.
        for i in 0..12 {
            let t = i as f32 / 12.0;
            let band =
                Rect::from_min_size(pos2(area.left(), footer.top() - 24.0 + t * 24.0), vec2(area.width() - 1.0, 2.0));
            p.rect_filled(band, 0.0, pl.surface.gamma_multiply(t * t));
        }
        p.line_segment([footer.left_top(), pos2(footer.right() - 1.0, footer.top())], Stroke::new(1.0, pl.stroke));
        self.sidebar_footer(ui, &p, footer, now);
    }

    fn sidebar_content(&mut self, ui: &Ui, p: &Painter, rect: Rect) -> f32 {
        let pl = pal();
        let ctx = ui.ctx().clone();
        let locked = self.locked();
        let x0 = rect.left() + PAD;
        let cw = rect.width() - PAD * 2.0;
        let mut y = rect.top() + 46.0;

        // Brand.
        let logo = Rect::from_min_size(pos2(x0 - 2.0, y), vec2(40.0, 40.0));
        p.image(self.logo.id(), logo, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.text(pos2(x0 + 46.0, y + 1.0), Align2::LEFT_TOP, APP_NAME, semibold(19.0), pl.text);
        p.text(
            pos2(x0 + 46.0, y + 24.0),
            Align2::LEFT_TOP,
            "Images web légères, qualité intacte",
            regular(11.5),
            pl.text_dim,
        );
        let info = Rect::from_center_size(pos2(x0 + cw - 10.0, y + 20.0), vec2(26.0, 26.0));
        let info_resp = w::hit(ui, info, "about", true);
        p.circle_stroke(
            info.center(),
            10.0,
            Stroke::new(1.3, if info_resp.hovered() { pl.accent } else { pl.text_faint }),
        );
        p.text(
            info.center(),
            Align2::CENTER_CENTER,
            "i",
            medium(12.0),
            if info_resp.hovered() { pl.accent } else { pl.text_faint },
        );
        if info_resp.clicked() {
            self.about = true;
        }
        y += 40.0 + 26.0;

        // Source.
        y += w::section_label(p, pos2(x0, y), "Images à compresser");
        let r = Rect::from_min_size(pos2(x0, y), vec2(cw, 78.0));
        let resp = w::hit(ui, r, "source-box", !locked);
        match &self.source {
            None => {
                let c = if resp.hovered() { pl.accent } else { pl.text_faint };
                p.rect_filled(r, 12.0, if resp.hovered() { pl.accent_soft } else { pl.bg });
                w::dashed_rect(p, r, 12.0, c, 1.3);
                w::folder_icon(p, pos2(r.left() + 34.0, r.center().y), 26.0, pl.accent);
                p.text(
                    pos2(r.left() + 60.0, r.center().y - 10.0),
                    Align2::LEFT_CENTER,
                    "Choisir un dossier…",
                    medium(13.5),
                    pl.text,
                );
                p.text(
                    pos2(r.left() + 60.0, r.center().y + 10.0),
                    Align2::LEFT_CENTER,
                    "ou glissez-le dans la fenêtre",
                    regular(11.5),
                    pl.text_dim,
                );
            }
            Some(input) => {
                p.rect(
                    r,
                    12.0,
                    if resp.hovered() { pl.accent_soft } else { pl.bg },
                    Stroke::new(1.0, pl.stroke),
                    egui::StrokeKind::Inside,
                );
                w::folder_icon(p, pos2(r.left() + 30.0, r.center().y), 24.0, pl.accent);
                let (name, sub) = describe_input(input);
                let tw = cw - 70.0;
                let g = w::fitted(p, &name, medium(13.5), pl.text, tw);
                p.galley(pos2(r.left() + 54.0, r.top() + 12.0), g, pl.text);
                let g = w::fitted_middle(p, &sub, regular(11.0), pl.text_faint, tw);
                p.galley(pos2(r.left() + 54.0, r.top() + 32.0), g, pl.text_faint);
                let line = if self.scan.is_some() {
                    "Analyse du dossier…".to_string()
                } else {
                    let n = self.jobs.iter().filter(|j| j.kind == JobKind::Convert).count();
                    let total: u64 = self.jobs.iter().map(|j| j.bytes).sum();
                    format!("{} · {}", plural(n, "image", "images"), fmt::bytes(total))
                };
                p.text(pos2(r.left() + 54.0, r.top() + 52.0), Align2::LEFT_TOP, line, medium(11.5), pl.accent);
            }
        }
        if resp.clicked() {
            self.pick_source(&ctx);
        }
        y += 78.0 + 18.0;

        // Output.
        y += w::section_label(p, pos2(x0, y), "Enregistrer dans");
        let r = Rect::from_min_size(pos2(x0, y), vec2(cw, 58.0));
        let resp = w::hit(ui, r, "output-box", !locked);
        p.rect(
            r,
            12.0,
            if resp.hovered() { pl.accent_soft } else { pl.bg },
            Stroke::new(1.0, pl.stroke),
            egui::StrokeKind::Inside,
        );
        w::folder_icon(p, pos2(r.left() + 30.0, r.center().y), 22.0, pl.text_dim);
        match &self.output {
            None => {
                p.text(
                    pos2(r.left() + 54.0, r.center().y),
                    Align2::LEFT_CENTER,
                    "Choisir le dossier de sortie…",
                    medium(13.0),
                    pl.text_dim,
                );
            }
            Some(out) => {
                let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let tw = cw - 70.0;
                let g = w::fitted(p, &name, medium(13.0), pl.text, tw);
                p.galley(pos2(r.left() + 54.0, r.top() + 11.0), g, pl.text);
                let parent = out.parent().map(tilde).unwrap_or_default();
                let g = w::fitted_middle(p, &parent, regular(11.0), pl.text_faint, tw);
                p.galley(pos2(r.left() + 54.0, r.top() + 31.0), g, pl.text_faint);
            }
        }
        if resp.clicked() {
            self.pick_output(&ctx);
        }
        y += 58.0 + 24.0;

        // Profiles.
        y += w::section_label(p, pos2(x0, y), "Destination");
        let tiles = [
            (Profile::ModernWeb, "Site web moderne", "AVIF · WebP · 2560 px"),
            (Profile::Cms, "WordPress · CMS", "WebP · 2048 px"),
            (Profile::Compatibility, "Compatibilité max", "JPEG · PNG · 1920 px"),
            (Profile::Archive, "Archive HD", "Format d'origine · HD"),
        ];
        let tw = (cw - 8.0) / 2.0;
        for (i, (profile, name, sub)) in tiles.iter().enumerate() {
            let r =
                Rect::from_min_size(pos2(x0 + (i % 2) as f32 * (tw + 8.0), y + (i / 2) as f32 * 66.0), vec2(tw, 58.0));
            let selected = profile.matches(&self.settings);
            let resp = w::hit(ui, r, ("profile", i), !locked);
            let (bg, stroke) = if selected {
                (pl.accent_soft, Stroke::new(1.5, pl.accent))
            } else if resp.hovered() {
                (pl.surface_hi, Stroke::new(1.0, pl.stroke))
            } else {
                (pl.surface, Stroke::new(1.0, pl.stroke))
            };
            p.rect(r, 12.0, bg, stroke, egui::StrokeKind::Inside);
            let g = w::fitted(p, name, medium(13.0), if selected { pl.accent } else { pl.text }, tw - 24.0);
            p.galley(pos2(r.left() + 12.0, r.top() + 11.0), g, pl.text);
            let g = w::fitted(p, sub, regular(11.0), pl.text_dim, tw - 24.0);
            p.galley(pos2(r.left() + 12.0, r.top() + 32.0), g, pl.text_dim);
            if resp.clicked() {
                profile.apply(&mut self.settings);
            }
        }
        y += 66.0 * 2.0 + 16.0;

        // Format.
        y += w::section_label(p, pos2(x0, y), "Format");
        let formats = ["Auto", "AVIF", "WebP", "JPEG·PNG", "Origine"];
        let sel = FormatChoice::ALL.iter().position(|f| *f == self.settings.format).unwrap_or(0);
        if let Some(i) = w::segmented(ui, p, Rect::from_min_size(pos2(x0, y), vec2(cw, 36.0)), &formats, sel, "format")
            && !locked
        {
            self.settings.format = FormatChoice::ALL[i];
        }
        y += 36.0 + 10.0;
        y += w::paragraph(p, pos2(x0, y), cw, format_help(self.settings.format), regular(11.5), pl.text_dim) + 22.0;

        // Quality.
        y += w::section_label(p, pos2(x0, y), "Qualité visuelle");
        let (name, help) = quality_name(self.settings.target_score);
        p.text(pos2(x0, y), Align2::LEFT_TOP, name, semibold(14.0), pl.text);
        w::pill(
            p,
            pos2(x0 + cw, y + 9.0),
            Align2::RIGHT_CENTER,
            &format!("score {:.0}", self.settings.target_score),
            pl.accent,
            pl.accent_soft,
        );
        y += 26.0;
        let track = Rect::from_min_size(pos2(x0, y), vec2(cw, 28.0));
        if let Some(v) = quality_slider(ui, p, track, self.settings.target_score)
            && !locked
        {
            self.settings.target_score = v;
        }
        y += 28.0 + 6.0;
        y += w::paragraph(p, pos2(x0, y), cw, help, regular(11.5), pl.text_dim) + 22.0;

        // Max size.
        y += w::section_label(p, pos2(x0, y), "Taille maximale (côté le plus long, px)");
        let items: Vec<(String, bool)> = MAX_SIDE_CHOICES
            .iter()
            .map(|m| (m.map(|v| v.to_string()).unwrap_or_else(|| "Originale".into()), *m == self.settings.max_side))
            .collect();
        let (clicked, h) = w::chips(ui, p, pos2(x0, y), cw, &items, "max-side");
        if let Some(i) = clicked
            && !locked
        {
            self.settings.max_side = MAX_SIDE_CHOICES[i];
        }
        y += h + 22.0;

        // Responsive variants.
        y += w::section_label(p, pos2(x0, y), "Images responsive");
        let on = !self.settings.responsive_widths.is_empty();
        let r = Rect::from_min_size(pos2(x0, y), vec2(cw, 44.0));
        if w::toggle_row(
            ui,
            p,
            r,
            "Variantes pour srcset",
            Some("Fichiers nom-640w… pour <img srcset>"),
            on,
            "responsive",
        ) && !locked
        {
            self.settings.responsive_widths = if on { Vec::new() } else { vec![640, 1024, 1600] };
        }
        y += 44.0 + 10.0;
        if on {
            let items: Vec<(String, bool)> = RESPONSIVE_CHOICES
                .iter()
                .map(|v| (v.to_string(), self.settings.responsive_widths.contains(v)))
                .collect();
            let (clicked, h) = w::chips(ui, p, pos2(x0, y), cw, &items, "widths");
            if let Some(i) = clicked
                && !locked
            {
                let v = RESPONSIVE_CHOICES[i];
                let list = &mut self.settings.responsive_widths;
                if let Some(pos) = list.iter().position(|x| *x == v) {
                    list.remove(pos);
                } else {
                    list.push(v);
                    list.sort_unstable();
                }
            }
            y += h + 10.0;
        }
        y += 12.0;

        // Options.
        y += w::section_label(p, pos2(x0, y), "Options");
        let toggles: [(&str, &str, bool, &str); 3] = [
            ("Noms de fichiers web", "Photo Été (1).JPG → photo-ete-1.avif", self.settings.slug_names, "slug"),
            ("Manifeste JSON", "Dimensions et srcset prêts pour le code", self.settings.write_manifest, "manifest"),
            (
                "Copier SVG, ICO et animations",
                "Le dossier de sortie est complet",
                self.settings.copy_other_files,
                "copy",
            ),
        ];
        for (label, sub, on, id) in toggles {
            let r = Rect::from_min_size(pos2(x0, y), vec2(cw, 44.0));
            if w::toggle_row(ui, p, r, label, Some(sub), on, id) && !locked {
                match id {
                    "slug" => self.settings.slug_names = !on,
                    "manifest" => self.settings.write_manifest = !on,
                    _ => self.settings.copy_other_files = !on,
                }
            }
            y += 50.0;
        }
        y += 8.0;
        y += w::section_label(p, pos2(x0, y), "Effort de compression");
        let sel = Effort::ALL.iter().position(|e| *e == self.settings.effort).unwrap_or(1);
        if let Some(i) = w::segmented(
            ui,
            p,
            Rect::from_min_size(pos2(x0, y), vec2(cw, 36.0)),
            &["Rapide", "Équilibré", "Maximum"],
            sel,
            "effort",
        ) && !locked
        {
            self.settings.effort = Effort::ALL[i];
        }
        y += 36.0 + 10.0;
        let effort_help = match self.settings.effort {
            Effort::Fast => "Encodeurs plus rapides, fichiers un peu plus lourds.",
            Effort::Balanced => "Le meilleur rapport temps / poids.",
            Effort::Max => "Quelques % de mieux, nettement plus lent.",
        };
        y += w::paragraph(p, pos2(x0, y), cw, effort_help, regular(11.5), pl.text_dim);

        if locked {
            p.rect_filled(rect, 0.0, pl.surface.gamma_multiply(0.45));
        }
        y
    }

    fn sidebar_footer(&mut self, ui: &Ui, p: &Painter, r: Rect, now: f64) {
        let pl = pal();
        p.rect_filled(r.shrink2(vec2(0.5, 0.0)), 0.0, pl.surface);
        let x0 = r.left() + PAD;
        let cw = r.width() - PAD * 2.0;
        let button = Rect::from_min_size(pos2(x0, r.top() + 16.0), vec2(cw, 46.0));
        let ctx = ui.ctx().clone();
        if let Some(run) = &self.run {
            let label = if run.cancelling() { "Annulation…" } else { "Annuler" };
            if w::button(ui, p, button, label, ButtonKind::Secondary, !run.cancelling()) {
                run.cancel();
            }
            let done = self.reports().count();
            p.text(
                pos2(r.center().x, button.bottom() + 16.0),
                Align2::CENTER_CENTER,
                format!("{done} / {} traitées · les originaux ne sont pas modifiés", self.rows.len()),
                regular(11.0),
                pl.text_faint,
            );
            return;
        }
        let images = self.jobs.iter().filter(|j| j.kind == JobKind::Convert).count();
        let (label, enabled) = match (&self.source, &self.output, self.scan.is_some()) {
            (None, _, _) => ("Choisissez des images".to_string(), false),
            (_, None, _) => ("Choisissez le dossier de sortie".to_string(), false),
            (_, _, true) => ("Analyse…".to_string(), false),
            _ if images == 0 && self.jobs.is_empty() => ("Aucune image trouvée".to_string(), false),
            _ if self.finished.is_some() => ("Recompresser".to_string(), true),
            _ => (format!("Compresser {}", plural(images, "image", "images")), true),
        };
        if (w::button(ui, p, button, &label, ButtonKind::Primary, enabled)
            || (enabled && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter))))
            && enabled
        {
            self.start(&ctx, now);
        }
        p.text(
            pos2(r.center().x, button.bottom() + 16.0),
            Align2::CENTER_CENTER,
            "Les originaux ne sont jamais modifiés",
            regular(11.0),
            pl.text_faint,
        );
    }

    fn main_area(&mut self, ui: &mut Ui, area: Rect, now: f64) {
        let pl = pal();
        let p = ui.painter().clone();
        let x0 = area.left() + 32.0;
        let cw = area.width() - 64.0;

        // Header (also the window's drag handle, the title bar being hidden).
        let header = Rect::from_min_size(area.min, vec2(area.width(), 96.0));
        let drag = ui.interact(header, egui::Id::unique("drag-window"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        let (title, subtitle) = self.headline();
        p.text(pos2(x0, area.top() + 34.0), Align2::LEFT_TOP, &title, semibold(22.0), pl.text);
        let g = w::fitted(&p, &subtitle, regular(13.0), pl.text_dim, cw - 220.0);
        p.galley(pos2(x0, area.top() + 64.0), g, pl.text_dim);

        let show_open = self.finished.is_some() || self.run.is_some();
        if show_open && let Some(out) = self.output.clone() {
            let bw = w::button_width(&p, "Ouvrir le dossier", 13.0);
            let b = Rect::from_min_size(pos2(x0 + cw - bw, area.top() + 40.0), vec2(bw, 36.0));
            if w::button(ui, &p, b, "Ouvrir le dossier", ButtonKind::Secondary, true) {
                worker::open_in_finder(&out);
            }
        }

        let mut y = header.bottom();
        if let Some(run) = &self.run {
            let done = self.reports().count();
            let frac = if self.rows.is_empty() { 0.0 } else { done as f32 / self.rows.len() as f32 };
            w::progress_bar(&p, Rect::from_min_size(pos2(x0, y - 8.0), vec2(cw, 6.0)), frac, pl.accent);
            let _ = run;
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        y += 12.0;

        if self.source.is_none() {
            self.empty_state(ui, &p, Rect::from_min_max(pos2(x0, y), pos2(x0 + cw, area.bottom() - 28.0)));
            return;
        }

        // Stat tiles.
        let tiles = self.tiles();
        let gap = 12.0;
        let tw = (cw - gap * (tiles.len() as f32 - 1.0)) / tiles.len() as f32;
        for (i, (label, value, sub, accent)) in tiles.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x0 + i as f32 * (tw + gap), y), vec2(tw, 86.0));
            w::card(&p, r);
            p.text(
                r.left_top() + vec2(16.0, 14.0),
                Align2::LEFT_TOP,
                label.to_uppercase(),
                medium(10.5),
                pl.text_faint,
            );
            let color = if *accent { pl.good } else { pl.text };
            let g = w::fitted(&p, value, semibold(22.0), color, tw - 32.0);
            p.galley(r.left_top() + vec2(16.0, 30.0), g, color);
            let g = w::fitted(&p, sub, regular(11.5), pl.text_dim, tw - 32.0);
            p.galley(r.left_top() + vec2(16.0, 60.0), g, pl.text_dim);
        }
        y += 86.0 + 16.0;
        if self.finished.is_some() {
            let hint = "Cliquez sur une image pour la comparer avant / après";
            p.text(pos2(x0 + 2.0, y), Align2::LEFT_TOP, hint, regular(11.5), pl.text_faint);
            y += 24.0;
        }

        let list = Rect::from_min_max(pos2(x0, y), pos2(x0 + cw, area.bottom() - 24.0));
        self.file_list(ui, &p, list, now);
    }

    fn headline(&self) -> (String, String) {
        let n = self.rows.len().max(self.jobs.len());
        if let Some(run) = &self.run {
            let done = self.reports().count();
            let elapsed = run.started.elapsed().as_millis() as u64;
            let eta = if done > 0 && done < n {
                let remaining = elapsed * (n - done) as u64 / done as u64;
                format!(" · encore ~{}", fmt::duration_ms(remaining.max(1000)))
            } else {
                String::new()
            };
            let title = if run.cancelling() { "Annulation…" } else { "Compression en cours…" };
            return (title.into(), format!("{done} sur {n}{eta}"));
        }
        if let Some(f) = &self.finished {
            let s = imgsoft_core::Summary::of(&self.reports().cloned().collect::<Vec<_>>());
            let title = if f.cancelled { "Compression interrompue" } else { "C'est prêt" };
            let mut sub = format!(
                "{} · de {} à {} ({}) en {}",
                plural(s.compressed + s.kept_original, "image", "images"),
                fmt::bytes(s.original_bytes),
                fmt::bytes(s.output_bytes),
                fmt::change(s.original_bytes, s.output_bytes),
                fmt::duration_ms(f.elapsed_ms)
            );
            if s.failed > 0 {
                sub += &format!(" · {} en échec", s.failed);
            }
            return (title.into(), sub);
        }
        match &self.source {
            None => ("Bienvenue".into(), "Choisissez les images d'un client : ImgSoft les prépare pour le web.".into()),
            Some(_) if self.scan.is_some() => ("Analyse…".into(), "Recherche des images dans le dossier".into()),
            Some(_) => {
                let n = self.jobs.iter().filter(|j| j.kind == JobKind::Convert).count();
                (
                    format!("{} prête{}", plural(n, "image", "images"), if n > 1 { "s" } else { "" }),
                    "Vérifiez les réglages à gauche, puis lancez la compression.".into(),
                )
            }
        }
    }

    /// (label, value, explanation, highlighted)
    fn tiles(&self) -> Vec<(String, String, String, bool)> {
        if self.run.is_some() || self.finished.is_some() {
            let reports: Vec<FileReport> = self.reports().cloned().collect();
            let s = imgsoft_core::Summary::of(&reports);
            let elapsed = match (&self.run, &self.finished) {
                (Some(r), _) => r.started.elapsed().as_millis() as u64,
                (_, Some(f)) => f.elapsed_ms,
                _ => 0,
            };
            let avg_score: Vec<f64> = reports.iter().filter_map(|r| r.score).collect();
            let fidelity = if avg_score.is_empty() {
                "—".to_string()
            } else {
                format!("{:.1}", avg_score.iter().cloned().fold(f64::INFINITY, f64::min)).replace('.', ",")
            };
            let mut variants = String::new();
            if s.variant_files > 0 {
                variants = format!("+ {} variantes, {}", s.variant_files, fmt::bytes(s.variant_bytes));
            }
            return vec![
                (
                    "Avant".into(),
                    fmt::bytes(s.original_bytes),
                    plural(s.compressed + s.kept_original, "image traitée", "images traitées"),
                    false,
                ),
                (
                    "Après".into(),
                    fmt::bytes(s.output_bytes),
                    if variants.is_empty() { "fichiers principaux".into() } else { variants },
                    false,
                ),
                (
                    "Gain".into(),
                    fmt::change(s.original_bytes, s.output_bytes),
                    format!("{} économisés", fmt::bytes(s.original_bytes.saturating_sub(s.output_bytes))),
                    true,
                ),
                (
                    "Fidélité min.".into(),
                    fidelity,
                    format!("objectif {:.0} · {}", self.settings.target_score, fmt::duration_ms(elapsed)),
                    false,
                ),
            ];
        }
        let images: Vec<&Job> = self.jobs.iter().filter(|j| j.kind == JobKind::Convert).collect();
        let total: u64 = images.iter().map(|j| j.bytes).sum();
        let mut kinds: Vec<&'static str> =
            images.iter().filter_map(|j| SourceFormat::from_extension(&j.source).map(|f| f.label())).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let copies = self.jobs.len() - images.len();
        vec![
            (
                "Images".into(),
                images.len().to_string(),
                if copies > 0 {
                    format!(
                        "+ {} tel{} quel{}",
                        plural(copies, "fichier copié", "fichiers copiés"),
                        if copies > 1 { "s" } else { "" },
                        if copies > 1 { "s" } else { "" }
                    )
                } else {
                    "à compresser".into()
                },
                false,
            ),
            ("Poids actuel".into(), fmt::bytes(total), kinds.join(" · "), false),
            (
                "Format".into(),
                format_short(self.settings.format).into(),
                self.settings.max_side.map(|m| format!("max {m} px")).unwrap_or_else(|| "taille d'origine".into()),
                false,
            ),
            (
                "Qualité".into(),
                quality_name(self.settings.target_score).0.into(),
                format!("score SSIMULACRA2 ≥ {:.0}", self.settings.target_score),
                false,
            ),
        ]
    }

    fn empty_state(&mut self, ui: &Ui, p: &Painter, area: Rect) {
        let pl = pal();
        let zone_h = (area.height() - 150.0).clamp(240.0, 360.0);
        let zone = Rect::from_min_size(area.min, vec2(area.width(), zone_h));
        let resp = w::hit(ui, zone, "drop-zone", true);
        p.rect_filled(zone, 18.0, if resp.hovered() { pl.accent_soft } else { pl.surface });
        w::dashed_rect(
            p,
            zone.shrink(1.0),
            18.0,
            if resp.hovered() { pl.accent } else { pl.text_faint.gamma_multiply(0.7) },
            1.5,
        );
        let c = zone.center() - vec2(0.0, 52.0);
        p.circle_filled(c, 46.0, pl.accent_soft);
        w::logo_symbol(p, c, 64.0, pl.accent, Color32::from_rgb(0xFF, 0xA8, 0x4C), pl.accent);
        p.text(c + vec2(0.0, 66.0), Align2::CENTER_TOP, "Glissez un dossier d'images ici", semibold(18.0), pl.text);
        p.text(c + vec2(0.0, 94.0), Align2::CENTER_TOP, "ou cliquez pour le choisir", regular(13.0), pl.text_dim);
        p.text(
            c + vec2(0.0, 126.0),
            Align2::CENTER_TOP,
            "JPEG · PNG · HEIC · WebP · AVIF · TIFF · GIF · BMP",
            medium(11.5),
            pl.text_faint,
        );
        if resp.clicked() {
            self.pick_source(ui.ctx());
        }

        let features = [
            ("Qualité vérifiée", "Chaque image est mesurée (SSIMULACRA2) : on garde la plus légère qui reste fidèle."),
            ("Format optimal", "AVIF, WebP, MozJPEG ou PNG optimisé, selon le contenu et la destination choisie."),
            ("Prête pour le web", "Redimensionnée, bien orientée, en sRGB, sans GPS ni métadonnées, noms propres."),
        ];
        let y = zone.bottom() + 18.0;
        let gap = 14.0;
        let fw = (area.width() - gap * 2.0) / 3.0;
        let fh = (area.bottom() - y).clamp(0.0, 118.0);
        if fh < 90.0 {
            return;
        }
        for (i, (t, d)) in features.iter().enumerate() {
            let r = Rect::from_min_size(pos2(area.left() + i as f32 * (fw + gap), y), vec2(fw, fh));
            w::card(p, r);
            w::check_icon(p, r.left_top() + vec2(26.0, 26.0), 10.0, pl.accent_soft, pl.accent);
            p.text(r.left_top() + vec2(44.0, 17.0), Align2::LEFT_TOP, *t, semibold(13.5), pl.text);
            w::paragraph(p, r.left_top() + vec2(18.0, 46.0), fw - 36.0, d, regular(11.5), pl.text_dim);
        }
    }

    fn file_list(&mut self, ui: &mut Ui, p: &Painter, area: Rect, now: f64) {
        let pl = pal();
        w::card(p, area);
        let wide = area.width() > 900.0;
        let show_score = area.width() > 700.0;
        // Columns, right-aligned from the right edge.
        let right = area.right() - 18.0;
        let col_score = right;
        let col_format = col_score - if show_score { 62.0 } else { 0.0 };
        let col_gain = col_format - 104.0;
        let col_after = col_gain - 68.0;
        let col_before = col_after - 84.0;
        let col_dims = col_before - 88.0;
        let name_right = if wide { col_dims - 104.0 } else { col_before - 90.0 };
        let name_x = area.left() + 52.0;

        let head_y = area.top() + 22.0;
        let head = |x: f32, align: Align2, label: &str| {
            p.text(pos2(x, head_y), align, label.to_uppercase(), medium(10.0), pl.text_faint);
        };
        head(name_x, Align2::LEFT_CENTER, "Fichier");
        if wide {
            head(col_dims, Align2::RIGHT_CENTER, "Dimensions");
        }
        if show_score {
            head(col_score, Align2::RIGHT_CENTER, "Score");
        }
        head(col_before, Align2::RIGHT_CENTER, "Avant");
        head(col_after, Align2::RIGHT_CENTER, "Après");
        head(col_gain, Align2::RIGHT_CENTER, "Gain");
        head(col_format - 46.0, Align2::CENTER_CENTER, "Format");
        p.line_segment(
            [pos2(area.left() + 1.0, area.top() + 42.0), pos2(area.right() - 1.0, area.top() + 42.0)],
            Stroke::new(1.0, pl.stroke),
        );

        let body = Rect::from_min_max(pos2(area.left() + 1.0, area.top() + 43.0), area.max - vec2(1.0, 6.0));
        let row_h = 50.0;
        let n = self.jobs.len();
        let mut clicked = None;
        let time = now;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ScrollArea::vertical().id_salt("files").auto_shrink([false; 2]).scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..egui::scroll_area::ScrollSource::ALL }).show_rows(ui, row_h, n, |ui, range| {
                for i in range {
                    let (r, resp) = ui.allocate_exact_size(vec2(body.width(), row_h), Sense::click());
                    let p = ui.painter();
                    let job = &self.jobs[i];
                    let row = self.rows.get(i);
                    let done = matches!(row, Some(Row::Done(rep)) if matches!(rep.outcome, Outcome::Compressed | Outcome::KeptOriginal));
                    if resp.hovered() && done {
                        p.rect_filled(r.shrink2(vec2(6.0, 2.0)), 10.0, pl.surface_hi);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() && done {
                        clicked = Some(i);
                    }
                    if i + 1 < n {
                        p.line_segment(
                            [pos2(r.left() + 18.0, r.bottom()), pos2(r.right() - 18.0, r.bottom())],
                            Stroke::new(1.0, pl.stroke.gamma_multiply(0.6)),
                        );
                    }
                    let cy = r.center().y;
                    let icon = pos2(r.left() + 27.0, cy);
                    let name = job.display.display().to_string();
                    let name_w = name_right - name_x;
                    let src_fmt = SourceFormat::from_extension(&job.source).map(|f| f.label()).unwrap_or("—");
                    match row {
                        None | Some(Row::Waiting) => {
                            w::dot(p, icon, 4.0, pl.text_faint.gamma_multiply(0.6));
                            let g = w::fitted_middle(p, &name, medium(13.0), pl.text, name_w);
                            p.galley(pos2(name_x, cy - g.size().y / 2.0), g, pl.text);
                            p.text(pos2(col_before, cy), Align2::RIGHT_CENTER, fmt::bytes(job.bytes), regular(12.5), pl.text_dim);
                            w::pill(p, pos2(col_format - 46.0, cy), Align2::CENTER_CENTER, src_fmt, pl.text_dim, pl.surface_hi);
                        }
                        Some(Row::Running) => {
                            w::spinner(p, icon, 8.0, pl.accent, time);
                            let g = w::fitted_middle(p, &name, medium(13.0), pl.text, name_w);
                            p.galley(pos2(name_x, cy - 17.0), g, pl.text);
                            p.text(pos2(name_x, cy + 3.0), Align2::LEFT_TOP, "Recherche du meilleur réglage…", regular(11.5), pl.accent);
                            p.text(pos2(col_before, cy), Align2::RIGHT_CENTER, fmt::bytes(job.bytes), regular(12.5), pl.text_dim);
                            w::pill(p, pos2(col_format - 46.0, cy), Align2::CENTER_CENTER, src_fmt, pl.text_dim, pl.surface_hi);
                        }
                        Some(Row::Done(rep)) => {
                            let (sub, sub_color) = match &rep.outcome {
                                Outcome::Compressed => {
                                    w::check_icon(p, icon, 9.0, pl.good_soft, pl.good);
                                    (arrow_name(rep), pl.text_dim)
                                }
                                Outcome::KeptOriginal => {
                                    w::equal_icon(p, icon, 9.0, pl.accent_soft, pl.accent);
                                    (format!("{} · déjà optimale, original nettoyé", arrow_name(rep)), pl.text_dim)
                                }
                                Outcome::Copied => {
                                    w::equal_icon(p, icon, 9.0, pl.surface_hi, pl.text_dim);
                                    (format!("{} · {}", arrow_name(rep), rep.note.as_deref().unwrap_or("copié")), pl.text_faint)
                                }
                                Outcome::Failed(e) => {
                                    w::cross_icon(p, icon, 9.0, pl.bad.gamma_multiply(0.15), pl.bad);
                                    (capitalize(e), pl.bad)
                                }
                                Outcome::Cancelled => {
                                    w::dot(p, icon, 4.0, pl.text_faint);
                                    ("Annulé".into(), pl.text_faint)
                                }
                            };
                            let g = w::fitted_middle(p, &name, medium(13.0), pl.text, name_w);
                            p.galley(pos2(name_x, cy - 17.0), g, pl.text);
                            let sub = match (&rep.note, &rep.outcome) {
                                (Some(note), Outcome::Compressed) => format!("{sub} · {note}"),
                                _ => sub,
                            };
                            let sub_color = if rep.note.is_some() && rep.outcome == Outcome::Compressed { pl.warn } else { sub_color };
                            let sub_x = if rep.output.is_some() && !matches!(rep.outcome, Outcome::Failed(_)) {
                                w::arrow_right(p, pos2(name_x + 6.0, cy + 11.0), 11.0, pl.text_faint);
                                name_x + 17.0
                            } else {
                                name_x
                            };
                            let g = w::fitted_middle(p, &sub, regular(11.5), sub_color, name_right - sub_x);
                            p.galley(pos2(sub_x, cy + 3.0), g, sub_color);

                            p.text(pos2(col_before, cy), Align2::RIGHT_CENTER, fmt::bytes(rep.original_bytes), regular(12.5), pl.text_dim);
                            if matches!(rep.outcome, Outcome::Compressed | Outcome::KeptOriginal | Outcome::Copied) {
                                p.text(pos2(col_after, cy), Align2::RIGHT_CENTER, fmt::bytes(rep.output_bytes), medium(12.5), pl.text);
                                let grew = rep.output_bytes > rep.original_bytes;
                                p.text(
                                    pos2(col_gain, cy),
                                    Align2::RIGHT_CENTER,
                                    fmt::change(rep.original_bytes, rep.output_bytes),
                                    semibold(12.5),
                                    if grew { pl.warn } else if rep.outcome == Outcome::Copied { pl.text_faint } else { pl.good },
                                );
                            }
                            if wide && let Some((ow, oh)) = rep.output_size {
                                p.text(pos2(col_dims, cy), Align2::RIGHT_CENTER, format!("{ow} × {oh}"), regular(12.5), pl.text_dim);
                            }
                            if show_score
                                && let Some(sc) = rep.score {
                                    let s = if rep.outcome == Outcome::KeptOriginal { "100".into() } else { format!("{sc:.1}").replace('.', ",") };
                                    p.text(pos2(col_score, cy), Align2::RIGHT_CENTER, s, medium(12.5), pl.text);
                                }
                            let label = rep.codec.map(|c| c.label()).unwrap_or(src_fmt);
                            let (fg, bg) = if rep.codec.is_some() { (pl.accent, pl.accent_soft) } else { (pl.text_dim, pl.surface_hi) };
                            w::pill(p, pos2(col_format - 46.0, cy), Align2::CENTER_CENTER, label, fg, bg);
                        }
                    }
                }
            });
        });
        if let Some(i) = clicked {
            self.open_compare(i, ui.ctx());
        }
    }

    fn about_modal(&mut self, ui: &Ui, p: &Painter, screen: Rect) {
        let pl = pal();
        let blocker = ui.interact(screen, egui::Id::unique("about-blocker"), Sense::click());
        p.rect_filled(screen, 0.0, pl.overlay);
        let card = Rect::from_center_size(screen.center(), vec2(460.0, 400.0));
        w::card(p, card);
        let x0 = card.left() + 28.0;
        let mut y = card.top() + 28.0;
        p.image(
            self.logo.id(),
            Rect::from_min_size(pos2(x0 - 4.0, y - 4.0), vec2(64.0, 64.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        p.text(pos2(x0 + 70.0, y + 4.0), Align2::LEFT_TOP, APP_NAME, semibold(22.0), pl.text);
        p.text(
            pos2(x0 + 70.0, y + 34.0),
            Align2::LEFT_TOP,
            format!("Version {}", env!("CARGO_PKG_VERSION")),
            regular(12.5),
            pl.text_dim,
        );
        y += 84.0;
        let text = "Compresse les images de vos clients pour le web : chaque fichier est encodé \
                    puis mesuré avec SSIMULACRA2, et ImgSoft garde le plus léger qui atteint la \
                    qualité visée. Les originaux ne sont jamais modifiés.";
        y += w::paragraph(p, pos2(x0, y), card.width() - 56.0, text, regular(12.5), pl.text) + 16.0;
        y += w::section_label(p, pos2(x0, y), "Moteurs");
        let engines = "rav1e / ravif (AVIF) · libwebp (WebP) · MozJPEG (JPEG) · oxipng et libimagequant (PNG) · \
                       SSIMULACRA2 (mesure) · fast_image_resize (Lanczos en lumière linéaire) · ImageIO (HEIC)";
        y += w::paragraph(p, pos2(x0, y), card.width() - 56.0, engines, regular(12.0), pl.text_dim) + 16.0;
        w::paragraph(
            p,
            pos2(x0, y),
            card.width() - 56.0,
            "Logiciel libre, GPL-3.0-or-later.",
            regular(12.0),
            pl.text_dim,
        );
        let b = Rect::from_min_size(pos2(card.right() - 128.0, card.bottom() - 60.0), vec2(100.0, 38.0));
        let close_btn = w::button(ui, p, b, "Fermer", ButtonKind::Primary, true);
        let outside =
            blocker.clicked() && !ui.input(|i| i.pointer.interact_pos().is_some_and(|pos| card.contains(pos)));
        if close_btn || outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            self.about = false;
        }
    }

    fn drop_overlay(&self, p: &Painter, screen: Rect) {
        let pl = pal();
        p.rect_filled(screen, 0.0, pl.overlay);
        let r = screen.shrink(24.0);
        p.rect_filled(r, 20.0, pl.accent.gamma_multiply(0.12));
        w::dashed_rect(p, r, 20.0, Color32::WHITE, 2.0);
        p.text(
            r.center() - vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            "Déposez pour ajouter ces images",
            semibold(22.0),
            Color32::WHITE,
        );
        p.text(
            r.center() + vec2(0.0, 20.0),
            Align2::CENTER_CENTER,
            "Un dossier est parcouru avec ses sous-dossiers",
            regular(13.5),
            Color32::from_white_alpha(220),
        );
    }

    fn draw_toast(&mut self, p: &Painter, screen: Rect, now: f64) {
        let Some((text, at)) = &self.toast else { return };
        let age = now - at;
        if age > 4.0 {
            self.toast = None;
            return;
        }
        let pl = pal();
        let alpha = ((4.0 - age) / 0.3).clamp(0.0, 1.0) as f32;
        let g = p.layout_no_wrap(text.clone(), medium(13.0), Color32::WHITE);
        let r = Rect::from_center_size(pos2(screen.center().x, screen.bottom() - 48.0), g.size() + vec2(36.0, 20.0));
        p.rect_filled(r, r.height() / 2.0, pl.text.gamma_multiply(0.92 * alpha));
        let fg = pl.bg.gamma_multiply(alpha);
        p.galley(r.center() - g.size() / 2.0, g, fg);
    }

    fn apply_theme(&self, ui: &Ui) {
        let forced = std::env::var("IMGSOFT_THEME").ok();
        let dark = match forced.as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => ui.ctx().system_theme() == Some(egui::Theme::Dark),
        };
        set_palette(if dark { DARK } else { LIGHT });
        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    }

    fn automation(&mut self, ui: &Ui, now: f64) {
        let auto = &mut self.auto;
        if auto.screenshot.is_none() && !auto.autorun {
            return;
        }
        auto.frames += 1;
        if auto.autorun && auto.frames == 30 && self.scan.is_none() {
            auto.autorun = false;
            self.start(&ui.ctx().clone(), now);
        }
        let auto = &mut self.auto;
        if self.finished.is_some() && auto.done_at.is_none() {
            auto.done_at = Some(auto.frames);
            if let Some(i) = auto.compare {
                self.open_compare(i, &ui.ctx().clone());
            }
        }
        let auto = &mut self.auto;
        let waiting_run = std::env::var_os("IMGSOFT_AUTORUN").is_some() && auto.done_at.is_none();
        let compare_loading = self.compare.as_ref().is_some_and(|c| c.loading());
        let auto = &mut self.auto;
        let ready = match auto.done_at {
            _ if compare_loading => false,
            Some(d) => auto.frames >= d + 40,
            None => !waiting_run && auto.frames >= 40,
        };
        if ready && let Some(path) = auto.screenshot.take() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let [w, h] = img.size;
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = imgsoft_core::save_png(&path, w as u32, h as u32, rgba);
                std::process::exit(0);
            })));
        }
        ui.ctx().request_repaint();
    }
}

impl eframe::App for ImgSoftApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let now = ui.input(|i| i.time);
        self.apply_theme(ui);
        self.poll(&ui.ctx().clone(), now);
        let screen = ui.max_rect();
        let pl = pal();
        ui.painter().rect_filled(screen, 0.0, pl.bg);
        let hovering = self.handle_drops(ui);

        let sidebar = Rect::from_min_size(screen.min, vec2(SIDEBAR_W, screen.height()));
        let main = Rect::from_min_max(pos2(sidebar.right(), screen.top()), screen.max);
        self.sidebar(ui, sidebar, now);
        self.main_area(ui, main, now);

        let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("overlay")));
        let fg_ui = ui.new_child(UiBuilder::new().layer_id(fg.layer_id()).max_rect(screen));
        if let Some(compare) = &mut self.compare
            && !compare.show(&fg_ui, &fg, screen)
        {
            self.compare = None;
        }
        if self.about {
            self.about_modal(&fg_ui, &fg, screen);
        }
        if hovering {
            self.drop_overlay(&fg, screen);
        }
        self.draw_toast(&fg, screen, now);
        if self.toast.is_some() || self.scan.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.automation(ui, now);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.persist
            && let Ok(json) = serde_json::to_string(&self.settings)
        {
            storage.set_string(KEY_SETTINGS, json);
        }
    }
}

// --- Helpers -----------------------------------------------------------------------------

fn default_output(input: &Input) -> Option<PathBuf> {
    match input {
        Input::Folder(dir) => {
            let name = dir.file_name()?.to_string_lossy().into_owned();
            Some(dir.with_file_name(format!("{name}-web")))
        }
        Input::Files(files) => Some(files.first()?.parent()?.join("images-web")),
    }
}

fn describe_input(input: &Input) -> (String, String) {
    match input {
        Input::Folder(dir) => (
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string()),
            dir.parent().map(tilde).unwrap_or_default(),
        ),
        Input::Files(files) => (
            plural(files.len(), "fichier déposé", "fichiers déposés"),
            files.first().and_then(|f| f.parent()).map(tilde).unwrap_or_default(),
        ),
    }
}

/// `/Users/me/Desktop` → `~/Desktop`.
fn tilde(path: &Path) -> String {
    let s = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if s.starts_with(&home) => format!("~{}", &s[home.len()..]),
        _ => s,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn arrow_name(rep: &FileReport) -> String {
    rep.output.as_ref().map(|o| o.display().to_string()).unwrap_or_default()
}

fn format_help(f: FormatChoice) -> &'static str {
    match f {
        FormatChoice::Auto => {
            "Le plus léger image par image : AVIF pour les photos, WebP sans perte pour les logos et captures. Lu par tous les navigateurs actuels."
        }
        FormatChoice::Avif => "Les fichiers les plus légers. Chrome, Safari 16+, Firefox et Edge le lisent.",
        FormatChoice::Webp => "Le choix sûr pour WordPress et la plupart des CMS, lu partout, très léger.",
        FormatChoice::Classic => {
            "Compatibilité maximale (e-mails, anciens sites) : MozJPEG pour les photos, PNG optimisé pour la transparence."
        }
        FormatChoice::Keep => {
            "Garde le format de chaque fichier et l'optimise. HEIC, TIFF et BMP deviennent JPEG ou PNG."
        }
    }
}

fn format_short(f: FormatChoice) -> &'static str {
    match f {
        FormatChoice::Auto => "Auto",
        FormatChoice::Avif => "AVIF",
        FormatChoice::Webp => "WebP",
        FormatChoice::Classic => "JPEG · PNG",
        FormatChoice::Keep => "D'origine",
    }
}

fn quality_name(score: f32) -> (&'static str, &'static str) {
    let (_, name, help) = QUALITY_PRESETS
        .iter()
        .min_by(|a, b| (a.0 - score).abs().total_cmp(&(b.0 - score).abs()))
        .copied()
        .unwrap_or((85.0, "Excellente", ""));
    (name, help)
}

const SLIDER_MIN: f32 = 55.0;
const SLIDER_MAX: f32 = 95.0;

/// Track with the named presets as stops; returns a new value when moved.
fn quality_slider(ui: &Ui, p: &Painter, r: Rect, value: f32) -> Option<f32> {
    let pl = pal();
    let x_of = |v: f32| r.left() + 9.0 + (r.width() - 18.0) * (v - SLIDER_MIN) / (SLIDER_MAX - SLIDER_MIN);
    let resp = ui.interact(r, egui::Id::unique("quality-slider"), Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = r.center().y;
    p.line_segment([pos2(x_of(SLIDER_MIN), cy), pos2(x_of(SLIDER_MAX), cy)], Stroke::new(4.0, pl.surface_hi));
    p.line_segment([pos2(x_of(SLIDER_MIN), cy), pos2(x_of(value), cy)], Stroke::new(4.0, pl.accent));
    for (score, _, _) in QUALITY_PRESETS {
        let x = x_of(score);
        p.circle_filled(pos2(x, cy), 3.5, if score <= value { pl.accent } else { pl.stroke });
    }
    let knob = pos2(x_of(value), cy);
    p.circle(knob, 9.0, Color32::WHITE, Stroke::new(2.0, pl.accent));

    let pos = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked())?;
    let raw = SLIDER_MIN + (pos.x - r.left() - 9.0) / (r.width() - 18.0) * (SLIDER_MAX - SLIDER_MIN);
    let mut v = raw.clamp(SLIDER_MIN, SLIDER_MAX).round();
    if let Some((s, _, _)) = QUALITY_PRESETS.iter().find(|(s, _, _)| (s - raw).abs() < 1.3) {
        v = *s;
    }
    (v != value).then_some(v)
}
