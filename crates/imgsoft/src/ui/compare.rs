//! Before/after viewer: the source scaled like the output (what was measured) against the
//! file actually written, with a draggable split, fitted or at true 100 %.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use egui::{Align2, Color32, Key, Painter, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, Vec2, pos2, vec2};
use imgsoft_core::{FileReport, Outcome, fmt};

use super::theme::{pal, regular, semibold};
use super::widgets::{self as w};
use super::worker::{self, ComparePictures};

pub struct Compare {
    title: String,
    subtitle: String,
    left_label: String,
    right_label: String,
    rx: Option<Receiver<Result<ComparePictures, String>>>,
    before: Option<TextureHandle>,
    after: Option<TextureHandle>,
    error: Option<String>,
    split: f32,
    actual_size: bool,
    pan: Vec2,
    output: PathBuf,
}

impl Compare {
    pub fn new(report: &FileReport, source: PathBuf, output: PathBuf, ctx: &egui::Context) -> Self {
        let codec = report.codec.map(|c| c.label()).unwrap_or("Sortie");
        let title = report.output.as_ref().map(|o| o.display().to_string()).unwrap_or_default();
        let mut subtitle = format!(
            "de {} à {} ({})",
            fmt::bytes(report.original_bytes),
            fmt::bytes(report.output_bytes),
            fmt::change(report.original_bytes, report.output_bytes)
        );
        if let Some(s) = report.score {
            subtitle += &format!(" · fidélité SSIMULACRA2 {}", format!("{s:.1}").replace('.', ","));
        }
        if report.outcome == Outcome::KeptOriginal {
            subtitle += " · original conservé (déjà optimal)";
        }
        let source_label = report.source_format.map(|f| f.label()).unwrap_or("Original");
        Self {
            title,
            subtitle,
            left_label: format!("Original {source_label} · {}", fmt::bytes(report.original_bytes)),
            right_label: format!("{codec} · {}", fmt::bytes(report.output_bytes)),
            rx: Some(worker::load_compare(source, output.clone(), report.output_size, ctx.clone())),
            before: None,
            after: None,
            error: None,
            split: 0.5,
            actual_size: false,
            pan: Vec2::ZERO,
            output,
        }
    }

    pub fn loading(&self) -> bool {
        self.rx.is_some()
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        if let Ok(result) = rx.try_recv() {
            self.rx = None;
            match result {
                Ok(pics) => {
                    let opts = egui::TextureOptions::LINEAR;
                    self.before = Some(ctx.load_texture("compare-before", pics.before, opts));
                    self.after = Some(ctx.load_texture("compare-after", pics.after, opts));
                }
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// Draws the viewer; returns false once closed.
    pub fn show(&mut self, ui: &Ui, p: &Painter, screen: Rect) -> bool {
        self.poll(ui.ctx());
        let pl = pal();
        // Swallow clicks meant for what's underneath.
        let _ = ui.interact(screen, egui::Id::unique("compare-blocker"), Sense::click_and_drag());
        p.rect_filled(screen, 0.0, pl.overlay);
        let card = screen.shrink2(vec2(28.0, 24.0));
        w::card(p, card);

        // Top bar.
        let x0 = card.left() + 22.0;
        let close = Rect::from_center_size(pos2(card.right() - 30.0, card.top() + 32.0), vec2(30.0, 30.0));
        let close_resp = w::hit(ui, close, "compare-close", true);
        if close_resp.hovered() {
            p.circle_filled(close.center(), 15.0, pl.surface_hi);
        }
        w::close_icon(p, close.center(), 10.0, pl.text_dim);
        let seg = Rect::from_min_size(pos2(close.left() - 186.0, card.top() + 16.0), vec2(170.0, 32.0));
        if let Some(i) = w::segmented(ui, p, seg, &["Ajusté", "100 %"], self.actual_size as usize, "compare-zoom") {
            self.actual_size = i == 1;
            self.pan = Vec2::ZERO;
        }
        let reveal = Rect::from_min_size(pos2(seg.left() - 150.0, seg.top()), vec2(138.0, 32.0));
        if w::button(ui, p, reveal, "Afficher le fichier", w::ButtonKind::Quiet, true) {
            worker::reveal_in_finder(&self.output);
        }
        let text_w = reveal.left() - x0 - 16.0;
        let g = w::fitted_middle(p, &self.title, semibold(15.0), pl.text, text_w);
        p.galley(pos2(x0, card.top() + 14.0), g, pl.text);
        let g = w::fitted(p, &self.subtitle, regular(12.0), pl.text_dim, text_w);
        p.galley(pos2(x0, card.top() + 36.0), g, pl.text_dim);

        let area = Rect::from_min_max(pos2(card.left() + 16.0, card.top() + 64.0), card.max - vec2(16.0, 16.0));
        p.rect_filled(area, 10.0, if pl.dark { Color32::from_gray(10) } else { Color32::from_gray(232) });

        let closed = close_resp.clicked() || ui.input(|i| i.key_pressed(Key::Escape));
        match (&self.before, &self.after, &self.error) {
            (_, _, Some(e)) => {
                p.text(area.center(), Align2::CENTER_CENTER, format!("Aperçu impossible : {e}"), regular(13.0), pl.bad);
            }
            (Some(before), Some(after), None) => {
                let (before, after) = (before.clone(), after.clone());
                self.draw_pictures(ui, p, area, &before, &after);
            }
            _ => {
                let t = ui.input(|i| i.time);
                w::spinner(p, area.center() - vec2(0.0, 14.0), 12.0, pl.accent, t);
                p.text(
                    area.center() + vec2(0.0, 16.0),
                    Align2::CENTER_TOP,
                    "Préparation de l'aperçu…",
                    regular(13.0),
                    pl.text_dim,
                );
                ui.ctx().request_repaint();
            }
        }
        !closed
    }

    fn draw_pictures(&mut self, ui: &Ui, p: &Painter, area: Rect, before: &TextureHandle, after: &TextureHandle) {
        let pl = pal();
        let [iw, ih] = after.size();
        let (iw, ih) = (iw as f32, ih as f32);
        let ppp = ui.ctx().pixels_per_point();
        let scale = if self.actual_size { 1.0 / ppp } else { (area.width() / iw).min(area.height() / ih).min(1.0) };
        let size = vec2(iw * scale, ih * scale);

        let resp = ui.interact(area, egui::Id::unique("compare-area"), Sense::click_and_drag());
        if self.actual_size && resp.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta);
            self.pan += d;
        }
        let max_pan = ((size - area.size()) / 2.0).max(Vec2::ZERO);
        self.pan = self.pan.clamp(-max_pan, max_pan);
        let disp = Rect::from_center_size(area.center() + self.pan, size);
        if (resp.dragged() || resp.clicked())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.split = ((pos.x - disp.left()) / disp.width()).clamp(0.0, 1.0);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        let clip = p.with_clip_rect(area.intersect(p.clip_rect()));
        checkerboard(&clip, disp.intersect(area));
        let sx = disp.left() + disp.width() * self.split;
        let uv_full = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let left = Rect::from_min_max(disp.min, pos2(sx, disp.bottom()));
        let right = Rect::from_min_max(pos2(sx, disp.top()), disp.max);
        let uv_left = Rect::from_min_max(uv_full.min, pos2(self.split, 1.0));
        let uv_right = Rect::from_min_max(pos2(self.split, 0.0), uv_full.max);
        clip.image(before.id(), left, uv_left, Color32::WHITE);
        clip.image(after.id(), right, uv_right, Color32::WHITE);

        // Split handle.
        let top = disp.top().max(area.top());
        let bottom = disp.bottom().min(area.bottom());
        clip.line_segment([pos2(sx, top), pos2(sx, bottom)], Stroke::new(2.0, Color32::WHITE));
        let knob = pos2(sx, (top + bottom) / 2.0);
        clip.circle(knob, 17.0, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(40)));
        for dir in [-1.0f32, 1.0] {
            let c = knob + vec2(dir * 6.0, 0.0);
            let st = Stroke::new(1.8, pl.accent);
            clip.line_segment([c + vec2(-dir * 3.0, -4.5), c], st);
            clip.line_segment([c + vec2(-dir * 3.0, 4.5), c], st);
        }

        // Labels.
        let fg = Color32::WHITE;
        let bg = Color32::from_black_alpha(150);
        w::pill(&clip, area.left_top() + vec2(12.0, 22.0), Align2::LEFT_CENTER, &self.left_label, fg, bg);
        w::pill(&clip, area.right_top() + vec2(-12.0, 22.0), Align2::RIGHT_CENTER, &self.right_label, fg, bg);
        let hint = if self.actual_size {
            "Glissez pour comparer · défilez pour vous déplacer"
        } else {
            "Glissez pour comparer · « 100 % » pour voir chaque pixel"
        };
        w::pill(&clip, area.center_bottom() - vec2(0.0, 20.0), Align2::CENTER_CENTER, hint, fg, bg);
    }
}

fn checkerboard(p: &Painter, r: Rect) {
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    const S: f32 = 12.0;
    p.rect_filled(r, 0.0, Color32::from_gray(250));
    let dark = Color32::from_gray(222);
    let cols = (r.width() / S).ceil() as i32;
    let rows = (r.height() / S).ceil() as i32;
    if cols * rows > 40_000 {
        return;
    }
    for j in 0..rows {
        for i in 0..cols {
            if (i + j) % 2 == 0 {
                let cell = Rect::from_min_size(r.min + vec2(i as f32 * S, j as f32 * S), vec2(S, S)).intersect(r);
                p.rect_filled(cell, 0.0, dark);
            }
        }
    }
}
