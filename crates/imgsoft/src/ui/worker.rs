//! Work done off the UI thread: folder scans, the compression batch, comparison pictures.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use egui::ColorImage;
use imgsoft_core::{Event, FileReport, Input, Job, Settings, batch, resize};

pub struct Scan {
    pub rx: Receiver<Result<Vec<Job>, String>>,
}

pub fn scan(input: Input, output: PathBuf, settings: Settings, ctx: egui::Context) -> Scan {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(batch::plan(&input, &output, &settings).map_err(|e| e.to_string()));
        ctx.request_repaint();
    });
    Scan { rx }
}

pub enum Msg {
    Started(usize),
    Finished(Box<FileReport>),
    Done,
}

pub struct Running {
    pub rx: Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
}

impl Running {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub fn run(jobs: Vec<Job>, settings: Settings, output: PathBuf, ctx: egui::Context) -> Running {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let workers = batch::default_workers();
        batch::run(&jobs, &settings, &output, &flag, workers, &|event| {
            let msg = match event {
                Event::Started(i) => Msg::Started(i),
                Event::Finished(r) => Msg::Finished(r),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
        let _ = tx.send(Msg::Done);
        ctx.request_repaint();
    });
    Running { rx, cancel, started: Instant::now() }
}

pub struct ComparePictures {
    pub before: ColorImage,
    pub after: ColorImage,
}

/// Source (scaled like the output, i.e. exactly what was measured) and the output file.
pub fn load_compare(
    source: PathBuf,
    output: PathBuf,
    size: Option<(u32, u32)>,
    ctx: egui::Context,
) -> Receiver<Result<ComparePictures, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = (|| {
            let after = imgsoft_core::load_bitmap(&output).map_err(|e| e.to_string())?;
            let mut before = imgsoft_core::load_bitmap(&source).map_err(|e| e.to_string())?;
            let (w, h) = size.unwrap_or((after.width, after.height));
            if (before.width, before.height) != (w, h) {
                before = resize::resize(&before, w, h);
            }
            let to_image = |b: imgsoft_core::Bitmap| {
                ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba)
            };
            Ok(ComparePictures { before: to_image(before), after: to_image(after) })
        })();
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

pub fn open_in_finder(path: &std::path::Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

pub fn reveal_in_finder(path: &std::path::Path) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}
