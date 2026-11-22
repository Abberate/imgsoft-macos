//! Hand-drawn controls: everything is painted, egui only provides input.

use std::f32::consts::TAU;
use std::sync::Arc;

use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{
    Align2, Color32, CursorIcon, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, pos2, vec2,
};

use super::theme::{medium, pal, regular};

pub fn hit(ui: &Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let r = ui.interact(rect, Id::unique(("hit", id)), sense);
    if enabled && r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    r
}

/// Single-line text that fits `max_w`, with an ellipsis at the end.
pub fn fitted(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let g = p.layout_no_wrap(text.to_owned(), font.clone(), color);
    if g.size().x <= max_w {
        return g;
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if p.layout_no_wrap(s, font.clone(), color).size().x <= max_w { lo = mid } else { hi = mid - 1 }
    }
    p.layout_no_wrap(chars[..lo].iter().collect::<String>().trim_end().to_owned() + "…", font, color)
}

/// Single-line text that fits `max_w`, shortened in the middle (paths, file names).
pub fn fitted_middle(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let g = p.layout_no_wrap(text.to_owned(), font.clone(), color);
    if g.size().x <= max_w {
        return g;
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let build = |keep: usize| -> String {
        let head = keep / 2;
        let tail = keep - head;
        chars[..head].iter().collect::<String>() + "…" + &chars[n - tail..].iter().collect::<String>()
    };
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if p.layout_no_wrap(build(mid), font.clone(), color).size().x <= max_w { lo = mid } else { hi = mid - 1 }
    }
    p.layout_no_wrap(build(lo), font, color)
}

/// Wrapped paragraph; returns its height.
pub fn paragraph(p: &Painter, pos: Pos2, width: f32, text: &str, font: FontId, color: Color32) -> f32 {
    let g = p.layout(text.to_owned(), font, color, width);
    let h = g.size().y;
    p.galley(pos, g, color);
    h
}

pub fn card(p: &Painter, r: Rect) {
    let pl = pal();
    if !pl.dark {
        p.rect_filled(r.translate(vec2(0.0, 2.0)).expand(1.0), 15.0, Color32::from_black_alpha(8));
    }
    p.rect(r, 14.0, pl.surface, Stroke::new(1.0, pl.stroke), StrokeKind::Inside);
}

pub fn section_label(p: &Painter, pos: Pos2, label: &str) -> f32 {
    p.text(pos, Align2::LEFT_TOP, label.to_uppercase(), medium(10.5), pal().text_faint);
    18.0
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Quiet,
}

pub fn button(ui: &Ui, p: &Painter, r: Rect, label: &str, kind: ButtonKind, enabled: bool) -> bool {
    let pl = pal();
    let resp = hit(ui, r, ("btn", label, r.min.x as i32, r.min.y as i32), enabled);
    let hovered = resp.hovered() && enabled;
    let (bg, fg, stroke) = match kind {
        ButtonKind::Primary if !enabled => (pl.surface_hi, pl.text_faint, Stroke::NONE),
        ButtonKind::Primary => (if hovered { pl.accent_hi } else { pl.accent }, pl.on_accent, Stroke::NONE),
        ButtonKind::Secondary => (
            if hovered { pl.accent_soft } else { pl.surface },
            if enabled { pl.text } else { pl.text_faint },
            Stroke::new(1.0, if hovered { pl.accent } else { pl.stroke }),
        ),
        ButtonKind::Quiet => (
            if hovered { pl.surface_hi } else { Color32::TRANSPARENT },
            if enabled { pl.accent } else { pl.text_faint },
            Stroke::NONE,
        ),
    };
    let radius = (r.height() / 2.0).min(12.0);
    p.rect(r, radius, bg, stroke, StrokeKind::Inside);
    let size = if r.height() >= 44.0 { 15.0 } else { 13.0 };
    let g = fitted(p, label, medium(size), fg, r.width() - 16.0);
    p.galley(r.center() - g.size() / 2.0, g, fg);
    resp.clicked() && enabled
}

pub fn button_width(p: &Painter, label: &str, size: f32) -> f32 {
    p.layout_no_wrap(label.to_owned(), medium(size), Color32::WHITE).size().x + 28.0
}

pub fn chip_width(p: &Painter, label: &str) -> f32 {
    p.layout_no_wrap(label.to_owned(), medium(12.5), Color32::WHITE).size().x + 24.0
}

pub fn chip(
    ui: &Ui,
    p: &Painter,
    r: Rect,
    label: &str,
    selected: bool,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let pl = pal();
    let resp = hit(ui, r, ("chip", id), true);
    let (bg, fg, stroke) = if selected {
        (pl.accent_soft, pl.accent, Stroke::new(1.5, pl.accent))
    } else if resp.hovered() {
        (pl.surface_hi, pl.text, Stroke::new(1.0, pl.stroke))
    } else {
        (pl.surface, pl.text_dim, Stroke::new(1.0, pl.stroke))
    };
    p.rect(r, r.height() / 2.0, bg, stroke, StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, medium(12.5), fg);
    resp.clicked()
}

/// A wrapping row of chips. Returns (clicked index, total height).
pub fn chips(
    ui: &Ui,
    p: &Painter,
    origin: Pos2,
    max_w: f32,
    items: &[(String, bool)],
    id: &str,
) -> (Option<usize>, f32) {
    const H: f32 = 30.0;
    const GAP: f32 = 6.0;
    let (mut x, mut y) = (origin.x, origin.y);
    let mut clicked = None;
    for (i, (label, selected)) in items.iter().enumerate() {
        let w = chip_width(p, label);
        if x > origin.x && x + w > origin.x + max_w {
            x = origin.x;
            y += H + GAP;
        }
        let r = Rect::from_min_size(pos2(x, y), vec2(w, H));
        if chip(ui, p, r, label, *selected, (id, i)) {
            clicked = Some(i);
        }
        x += w + GAP;
    }
    (clicked, y + H - origin.y)
}

/// Segmented control; returns the clicked segment.
pub fn segmented(ui: &Ui, p: &Painter, r: Rect, labels: &[&str], selected: usize, id: &str) -> Option<usize> {
    let pl = pal();
    p.rect_filled(r, 10.0, pl.surface_hi);
    let n = labels.len().max(1);
    let w = (r.width() - 6.0) / n as f32;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let seg = Rect::from_min_size(pos2(r.left() + 3.0 + w * i as f32, r.top() + 3.0), vec2(w, r.height() - 6.0));
        let resp = hit(ui, seg, (id, i), true);
        if i == selected {
            p.rect(seg, 8.0, pl.surface, Stroke::new(1.0, pl.stroke), StrokeKind::Inside);
            if !pl.dark {
                p.rect_filled(seg.translate(vec2(0.0, 1.0)), 8.0, Color32::from_black_alpha(6));
                p.rect(seg, 8.0, pl.surface, Stroke::new(1.0, pl.stroke), StrokeKind::Inside);
            }
        } else if resp.hovered() {
            p.rect_filled(seg, 8.0, pl.stroke.gamma_multiply(0.6));
        }
        let fg = if i == selected { pl.accent } else { pl.text_dim };
        let g = fitted(p, label, medium(12.0), fg, seg.width() - 6.0);
        p.galley(seg.center() - g.size() / 2.0, g, fg);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Label (and optional explanation) with a switch on the right. Returns true when toggled.
pub fn toggle_row(ui: &Ui, p: &Painter, r: Rect, label: &str, sub: Option<&str>, on: bool, id: &str) -> bool {
    let pl = pal();
    let resp = hit(ui, r, ("toggle", id), true);
    let sw = Rect::from_center_size(pos2(r.right() - 20.0, r.center().y), vec2(36.0, 21.0));
    let t = ui.ctx().animate_bool(Id::unique(("toggle-anim", id)), on);
    let track = lerp(pl.stroke, pl.accent, t);
    p.rect_filled(sw, 10.5, track);
    let knob_x = egui::lerp(sw.left() + 10.5..=sw.right() - 10.5, t);
    p.circle_filled(pos2(knob_x, sw.center().y), 8.0, Color32::WHITE);
    let text_w = r.width() - 52.0;
    match sub {
        Some(sub) => {
            let g = fitted(p, label, medium(13.0), pl.text, text_w);
            p.galley(pos2(r.left(), r.center().y - 17.0), g, pl.text);
            let g = fitted(p, sub, regular(11.5), pl.text_dim, text_w);
            p.galley(pos2(r.left(), r.center().y + 2.0), g, pl.text_dim);
        }
        None => {
            let g = fitted(p, label, medium(13.0), pl.text, text_w);
            p.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, pl.text);
        }
    }
    resp.clicked()
}

pub fn progress_bar(p: &Painter, r: Rect, frac: f32, color: Color32) {
    let pl = pal();
    p.rect_filled(r, r.height() / 2.0, pl.surface_hi);
    if frac > 0.0 {
        let w = (r.width() * frac.clamp(0.0, 1.0)).max(r.height());
        p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), r.height() / 2.0, color);
    }
}

/// Small rounded label; `pos` is the anchor given by `align` (vertically centred).
pub fn pill(p: &Painter, pos: Pos2, align: Align2, text: &str, fg: Color32, bg: Color32) -> Rect {
    let g = p.layout_no_wrap(text.to_owned(), medium(11.0), fg);
    let size = vec2(g.size().x + 14.0, 20.0);
    let x = match align.x() {
        egui::Align::Min => pos.x,
        egui::Align::Center => pos.x - size.x / 2.0,
        egui::Align::Max => pos.x - size.x,
    };
    let r = Rect::from_min_size(pos2(x, pos.y - size.y / 2.0), size);
    p.rect_filled(r, 10.0, bg);
    p.galley(r.center() - g.size() / 2.0, g, fg);
    r
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// Dashed rounded rectangle (drop zones).
pub fn dashed_rect(p: &Painter, r: Rect, radius: f32, color: Color32, width: f32) {
    let mut pts = Vec::new();
    let corners = [
        (pos2(r.right() - radius, r.top() + radius), -TAU / 4.0),
        (pos2(r.right() - radius, r.bottom() - radius), 0.0),
        (pos2(r.left() + radius, r.bottom() - radius), TAU / 4.0),
        (pos2(r.left() + radius, r.top() + radius), TAU / 2.0),
    ];
    for (c, a0) in corners {
        for i in 0..=8 {
            let a = a0 + TAU / 4.0 * i as f32 / 8.0;
            pts.push(c + vec2(a.cos(), a.sin()) * radius);
        }
    }
    pts.push(pts[0]);
    let stroke = Stroke::new(width, color);
    p.extend(Shape::dashed_line(&pts, stroke, 7.0, 5.0));
}

// --- Icons -------------------------------------------------------------------------------

/// The ImgSoft symbol: a picture frame squeezed at the waist.
pub fn logo_symbol(p: &Painter, center: Pos2, size: f32, frame: Color32, sun: Color32, mountain: Color32) {
    let s = size / 256.0;
    let at = |x: f32, y: f32| center + vec2((x - 128.0) * s, (y - 128.0) * s);
    let w = 20.0 * s;
    // Frame outline (centre line of the stroke).
    let mut path: Vec<Pos2> = Vec::new();
    let arc = |path: &mut Vec<Pos2>, cx: f32, cy: f32, a0: f32, a1: f32| {
        for i in 0..=8 {
            let a = a0 + (a1 - a0) * i as f32 / 8.0;
            path.push(at(cx + 34.0 * a.cos(), cy + 34.0 * a.sin()));
        }
    };
    let bez = |path: &mut Vec<Pos2>, pts: [(f32, f32); 4]| {
        let c = CubicBezierShape::from_points_stroke(
            [at(pts[0].0, pts[0].1), at(pts[1].0, pts[1].1), at(pts[2].0, pts[2].1), at(pts[3].0, pts[3].1)],
            false,
            Color32::TRANSPARENT,
            Stroke::NONE,
        );
        path.extend(c.flatten(Some(0.2)).into_iter().skip(1));
    };
    use std::f32::consts::PI;
    arc(&mut path, 186.0, 76.0, -PI / 2.0, 0.0);
    bez(&mut path, [(220.0, 76.0), (220.0, 98.0), (200.0, 108.0), (200.0, 128.0)]);
    bez(&mut path, [(200.0, 128.0), (200.0, 148.0), (220.0, 158.0), (220.0, 180.0)]);
    arc(&mut path, 186.0, 180.0, 0.0, PI / 2.0);
    arc(&mut path, 70.0, 180.0, PI / 2.0, PI);
    bez(&mut path, [(36.0, 180.0), (36.0, 158.0), (56.0, 148.0), (56.0, 128.0)]);
    bez(&mut path, [(56.0, 128.0), (56.0, 108.0), (36.0, 98.0), (36.0, 76.0)]);
    arc(&mut path, 70.0, 76.0, PI, PI * 1.5);
    p.add(PathShape::closed_line(path, PathStroke::new(w, frame)));
    p.circle_filled(at(158.0, 94.0), 17.0 * s, sun);
    let m = [at(74.0, 184.0), at(110.0, 132.0), at(134.0, 162.0), at(150.0, 146.0), at(182.0, 184.0)];
    p.add(PathShape::convex_polygon(vec![m[0], m[1], m[2], m[4]], mountain, Stroke::new(8.0 * s, mountain)));
    p.add(PathShape::convex_polygon(vec![m[2], m[3], m[4]], mountain, Stroke::new(8.0 * s, mountain)));
}

pub fn folder_icon(p: &Painter, center: Pos2, size: f32, color: Color32) {
    let w = size;
    let h = size * 0.78;
    let r = Rect::from_center_size(center + vec2(0.0, size * 0.04), vec2(w, h));
    let tab = Rect::from_min_size(r.min - vec2(0.0, size * 0.1), vec2(w * 0.42, size * 0.22));
    p.rect_filled(tab, size * 0.08, color.gamma_multiply(0.75));
    p.rect_filled(r, size * 0.12, color);
}

pub fn check_icon(p: &Painter, center: Pos2, r: f32, bg: Color32, fg: Color32) {
    p.circle_filled(center, r, bg);
    let s = r / 9.0;
    p.add(PathShape::line(
        vec![center + vec2(-4.0, 0.2) * s, center + vec2(-1.2, 3.0) * s, center + vec2(4.2, -2.8) * s],
        PathStroke::new(1.9 * s, fg),
    ));
}

pub fn cross_icon(p: &Painter, center: Pos2, r: f32, bg: Color32, fg: Color32) {
    p.circle_filled(center, r, bg);
    let d = r * 0.36;
    let st = Stroke::new(r / 5.0, fg);
    p.line_segment([center + vec2(-d, -d), center + vec2(d, d)], st);
    p.line_segment([center + vec2(-d, d), center + vec2(d, -d)], st);
}

pub fn equal_icon(p: &Painter, center: Pos2, r: f32, bg: Color32, fg: Color32) {
    p.circle_filled(center, r, bg);
    let d = r * 0.4;
    let st = Stroke::new(r / 5.0, fg);
    p.line_segment([center + vec2(-d, -r * 0.2), center + vec2(d, -r * 0.2)], st);
    p.line_segment([center + vec2(-d, r * 0.2), center + vec2(d, r * 0.2)], st);
}

pub fn spinner(p: &Painter, center: Pos2, r: f32, color: Color32, time: f64) {
    p.circle_stroke(center, r, Stroke::new(2.0, color.gamma_multiply(0.2)));
    let start = (time * 5.0) as f32;
    let pts: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = start + i as f32 / 16.0 * TAU * 0.3;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(PathShape::line(pts, PathStroke::new(2.0, color)));
}

pub fn dot(p: &Painter, center: Pos2, r: f32, color: Color32) {
    p.circle_filled(center, r, color);
}

pub fn close_icon(p: &Painter, center: Pos2, size: f32, color: Color32) {
    let d = size / 2.0;
    let st = Stroke::new(1.8, color);
    p.line_segment([center + vec2(-d, -d), center + vec2(d, d)], st);
    p.line_segment([center + vec2(-d, d), center + vec2(d, -d)], st);
}

pub fn arrow_right(p: &Painter, center: Pos2, size: f32, color: Color32) {
    let st = Stroke::new(1.5, color);
    p.line_segment([center - vec2(size / 2.0, 0.0), center + vec2(size / 2.0, 0.0)], st);
    p.line_segment([center + vec2(size / 2.0, 0.0), center + vec2(size / 2.0 - 4.0, -4.0)], st);
    p.line_segment([center + vec2(size / 2.0, 0.0), center + vec2(size / 2.0 - 4.0, 4.0)], st);
}
