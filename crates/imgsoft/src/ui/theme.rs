//! Colours and fonts: the logo's ultraviolet on a quiet ground, light and dark.

use std::cell::Cell;
use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hi: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub good: Color32,
    pub good_soft: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub overlay: Color32,
}

pub const LIGHT: Palette = Palette {
    dark: false,
    bg: Color32::from_rgb(0xF4, 0xF3, 0xF9),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface_hi: Color32::from_rgb(0xEE, 0xEC, 0xF6),
    stroke: Color32::from_rgb(0xE3, 0xE0, 0xEE),
    text: Color32::from_rgb(0x16, 0x13, 0x2B),
    text_dim: Color32::from_rgb(0x5E, 0x5A, 0x78),
    text_faint: Color32::from_rgb(0x9B, 0x97, 0xB2),
    accent: Color32::from_rgb(0x5E, 0x3F, 0xEF),
    accent_hi: Color32::from_rgb(0x7B, 0x5C, 0xFF),
    accent_soft: Color32::from_rgb(0xEC, 0xE7, 0xFF),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(0x12, 0x9E, 0x55),
    good_soft: Color32::from_rgb(0xE2, 0xF6, 0xEA),
    warn: Color32::from_rgb(0xD9, 0x8A, 0x00),
    bad: Color32::from_rgb(0xDC, 0x3B, 0x47),
    overlay: Color32::from_rgba_premultiplied(10, 8, 24, 120),
};

pub const DARK: Palette = Palette {
    dark: true,
    bg: Color32::from_rgb(0x0E, 0x0D, 0x16),
    surface: Color32::from_rgb(0x18, 0x16, 0x24),
    surface_hi: Color32::from_rgb(0x22, 0x1F, 0x31),
    stroke: Color32::from_rgb(0x2B, 0x28, 0x3D),
    text: Color32::from_rgb(0xF2, 0xF0, 0xFA),
    text_dim: Color32::from_rgb(0xA8, 0xA3, 0xC2),
    text_faint: Color32::from_rgb(0x6E, 0x69, 0x88),
    accent: Color32::from_rgb(0x84, 0x6B, 0xFF),
    accent_hi: Color32::from_rgb(0x9D, 0x88, 0xFF),
    accent_soft: Color32::from_rgb(0x26, 0x1F, 0x47),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(0x34, 0xC7, 0x7B),
    good_soft: Color32::from_rgb(0x12, 0x2E, 0x22),
    warn: Color32::from_rgb(0xF2, 0xB1, 0x34),
    bad: Color32::from_rgb(0xFF, 0x6B, 0x75),
    overlay: Color32::from_rgba_premultiplied(0, 0, 0, 160),
};

thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(LIGHT) };
}

pub fn set_palette(p: Palette) {
    CURRENT.with(|c| c.set(p));
}

/// Palette of the frame being drawn.
pub fn pal() -> Palette {
    CURRENT.with(|c| c.get())
}

const REGULAR_TTF: &[u8] = include_bytes!("../../assets/fonts/Poppins-Regular.ttf");
const MEDIUM_TTF: &[u8] = include_bytes!("../../assets/fonts/Poppins-Medium.ttf");
const SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Poppins-SemiBold.ttf");

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, data) in [("pp-regular", REGULAR_TTF), ("pp-medium", MEDIUM_TTF), ("pp-semibold", SEMIBOLD_TTF)] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    }
    let proportional = fonts.families.get_mut(&FontFamily::Proportional).expect("proportional family");
    proportional.insert(0, "pp-regular".into());
    // egui's own fonts stay as fallbacks (symbols, other scripts).
    let fallbacks: Vec<String> = proportional.iter().skip(1).cloned().collect();
    for (family, face) in [("medium", "pp-medium"), ("semibold", "pp-semibold")] {
        let mut list = vec![face.to_string()];
        list.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    ctx.set_fonts(fonts);
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn visuals(p: Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface_hi;
    v.faint_bg_color = p.surface_hi;
    v.window_stroke = Stroke::new(1.0, p.stroke);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.text);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(8);
    }
    v.widgets.inactive.bg_fill = p.surface_hi;
    v.widgets.inactive.weak_bg_fill = p.surface_hi;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = p.accent_soft;
    v.widgets.hovered.weak_bg_fill = p.accent_soft;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v
}

/// egui's own widgets (scroll bars, tooltips) in our colours, for both themes.
pub fn install_visuals(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(LIGHT));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(DARK));
    ctx.all_styles_mut(|s| {
        s.spacing.scroll.floating = true;
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.text_styles.insert(egui::TextStyle::Body, regular(13.5));
        s.text_styles.insert(egui::TextStyle::Button, medium(13.0));
        s.text_styles.insert(egui::TextStyle::Small, regular(11.5));
    });
}
