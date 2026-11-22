//! ImgSoft — compresses client images for the web at a guaranteed visual quality.

mod ui;

/// Default window size; `IMGSOFT_WINDOW=WxH` overrides it (visual checks).
fn window_size() -> [f32; 2] {
    std::env::var("IMGSOFT_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1220.0, 820.0])
}

/// Runs driven by `IMGSOFT_*` variables keep their own throw-away settings.
fn test_run() -> bool {
    std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("IMGSOFT_"))
}

/// Dock icon. Inside the .app bundle macOS uses AppIcon.icns, so eframe gets an empty one
/// (otherwise it would swap in its own logo); a bare `cargo run` uses the same artwork.
fn app_icon() -> egui::IconData {
    let bundled =
        std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false);
    if bundled {
        return egui::IconData::default();
    }
    imgsoft_core::load_bitmap_from_memory(include_bytes!("../../../packaging/icon_1024.png"))
        .map(|b| egui::IconData { width: b.width, height: b.height, rgba: b.rgba })
        .unwrap_or_default()
}

fn main() -> eframe::Result {
    if let Some(out) = std::env::var_os("IMGSOFT_DMG_BG") {
        // Installer artwork (see scripts/make_dmg.sh).
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(ui::dmg_bg::SIZE)
                .with_resizable(false)
                .with_decorations(false)
                .with_icon(egui::IconData::default()),
            persistence_path: Some(std::env::temp_dir().join("imgsoft-dmg.ron")),
            persist_window: false,
            ..Default::default()
        };
        return eframe::run_native(
            "ImgSoft installer art",
            options,
            Box::new(move |cc| Ok(Box::new(ui::dmg_bg::DmgBackground::new(cc, out.into())))),
        );
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(ui::APP_NAME)
            .with_app_id("io.github.imgsoft")
            .with_icon(app_icon())
            .with_inner_size(window_size())
            .with_min_inner_size([980.0, 660.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true)
            .with_drag_and_drop(true),
        persistence_path: test_run().then(|| std::env::temp_dir().join("imgsoft-test.ron")),
        // Scripted runs size their window themselves (IMGSOFT_WINDOW).
        persist_window: !test_run(),
        ..Default::default()
    };
    let test = test_run();
    eframe::run_native(ui::APP_NAME, options, Box::new(move |cc| Ok(Box::new(ui::ImgSoftApp::new(cc, test)))))
}
