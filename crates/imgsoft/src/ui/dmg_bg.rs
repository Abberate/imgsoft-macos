//! Renders the DMG window background (drag-to-Applications arrow + instructions) with the
//! app's own fonts and colours, saves a screenshot and quits.
//! Used by `scripts/make_dmg.sh` through `IMGSOFT_DMG_BG=<out.png>`.
//!
//! The picture is a little taller than the Finder window so it covers it whatever the
//! title bar height; everything that matters sits above y = 520.

use egui::{Align2, Color32, Pos2, Rect, Stroke, TextureHandle, Ui, ViewportCommand, pos2, vec2};

use super::theme;

/// Picture size in points. Icon centres must match `scripts/dmg_layout.py`.
pub const SIZE: [f32; 2] = [760.0, 570.0];
const APP_ICON: Pos2 = pos2(210.0, 200.0);
const APPLICATIONS_ICON: Pos2 = pos2(550.0, 200.0);

pub struct DmgBackground {
    out: std::path::PathBuf,
    frames: u32,
    logo: TextureHandle,
}

impl DmgBackground {
    pub fn new(cc: &eframe::CreationContext<'_>, out: std::path::PathBuf) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_zoom_factor(1.0);
        Self { out, frames: 0, logo: super::app::logo_texture(&cc.egui_ctx) }
    }
}

impl eframe::App for DmgBackground {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        theme::set_palette(theme::LIGHT);
        let pl = theme::LIGHT;
        let p = ui.painter();
        let o = ui.max_rect().min;
        let at = |x: f32, y: f32| o + vec2(x, y);
        p.rect_filled(ui.max_rect(), 0.0, pl.bg);
        let cx = SIZE[0] / 2.0;

        // Title with the app mark, version line.
        let title = p.layout_no_wrap("ImgSoft".into(), theme::semibold(26.0), pl.text);
        let total = 36.0 + 12.0 + title.size().x;
        let left = cx - total / 2.0;
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        p.image(self.logo.id(), Rect::from_min_size(at(left, 32.0), vec2(36.0, 36.0)), uv, Color32::WHITE);
        p.galley(at(left + 48.0, 50.0 - title.size().y / 2.0), title, pl.text);
        p.text(
            at(cx, 88.0),
            Align2::CENTER_CENTER,
            format!("Version {} · macOS 11+ · Apple Silicon et Intel", env!("CARGO_PKG_VERSION")),
            theme::regular(13.0),
            pl.text_dim,
        );

        // Dotted arrow from the app icon to the Applications folder.
        let (from, to) = (APP_ICON.x + 82.0, APPLICATIONS_ICON.x - 82.0);
        let y = APP_ICON.y;
        let n = 10;
        for i in 0..n {
            let x = from + (to - from - 14.0) * i as f32 / (n - 1) as f32;
            let fade = 0.25 + 0.75 * i as f32 / (n - 1) as f32;
            p.circle_filled(at(x, y), 3.0, pl.accent.gamma_multiply(fade));
        }
        for k in 1..=3 {
            let d = k as f32 * 7.0;
            p.circle_filled(at(to - d, y - d), 3.0, pl.accent);
            p.circle_filled(at(to - d, y + d), 3.0, pl.accent);
        }
        p.circle_filled(at(to, y), 3.5, pl.accent);

        p.line_segment([at(40.0, 306.0), at(SIZE[0] - 40.0, 306.0)], Stroke::new(1.0, pl.stroke));

        // Numbered steps, French then English (the read-me icon sits on the right).
        let steps = [
            ("Glissez ImgSoft sur le dossier Applications", "Drag ImgSoft onto the Applications folder"),
            (
                "1er lancement : Réglages > Confidentialité et sécurité > Ouvrir quand même",
                "First launch: Settings > Privacy & Security > Open Anyway",
            ),
            (
                "Glissez un dossier d'images dans la fenêtre, choisissez la destination, c'est tout",
                "Drop a folder of images on the window, pick a destination, done",
            ),
        ];
        let mut sy = 328.0;
        for (i, (fr, en)) in steps.iter().enumerate() {
            let c = at(50.0, sy + 8.0);
            p.circle_filled(c, 10.0, pl.accent);
            p.text(c, Align2::CENTER_CENTER, (i + 1).to_string(), theme::medium(11.5), pl.on_accent);
            p.text(at(70.0, sy + 1.0), Align2::LEFT_TOP, *fr, theme::medium(12.5), pl.text);
            p.text(at(70.0, sy + 19.0), Align2::LEFT_TOP, *en, theme::regular(11.5), pl.text_dim);
            sy += 46.0;
        }

        p.text(
            at(cx, 500.0),
            Align2::CENTER_CENTER,
            "Logiciel libre · Free software · GPL-3.0-or-later",
            theme::regular(10.5),
            pl.text_faint,
        );

        self.frames += 1;
        if self.frames == 20 {
            let out = self.out.clone();
            ui.ctx().send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let [w, h] = img.size;
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = imgsoft_core::save_png(&out, w as u32, h as u32, rgba);
                std::process::exit(0);
            })));
        }
        ui.ctx().request_repaint();
    }
}
