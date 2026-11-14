//! PoB's own images outside the tree (the tooltip headers, separators and icons, the oils, the
//! slot icons): read by their path under PoB's folder, decoded off the UI frame on the art
//! thread, uploaded once with the pixels dropped, and freed when nothing has drawn them for
//! [`KEEP_SECONDS`].

use eframe::egui::{self, TextureHandle, TextureId, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// A texture nothing drew for this long is freed
pub const KEEP_SECONDS: f64 = 10.0;

/// The PoB folder the engine runs from, which the images' paths are under
static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Sets the PoB folder images are read from (the engine's, once it starts)
pub fn set_root(dir: &Path) {
    *ROOT.write().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
}

/// Work for the art thread
pub type ArtJob = Box<dyn FnOnce() + Send>;

/// Runs `work` on the process's one thread for PoB's art, started the first time. One thread,
/// not one per batch: each new thread would get a malloc arena of its own, which keeps what the
/// decoding left in it (about 25 MB more resident after a build was opened).
pub fn send_to_art_thread(work: ArtJob) -> std::io::Result<()> {
    static QUEUE: std::sync::OnceLock<Option<Mutex<std::sync::mpsc::Sender<ArtJob>>>> =
        std::sync::OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<ArtJob>();
        std::thread::Builder::new()
            .name("tree-art".into())
            .spawn(move || {
                for job in receive {
                    job();
                }
            })
            .ok()
            .map(|_| Mutex::new(send))
    });
    let Some(queue) = queue else {
        return Err(std::io::Error::other("the art thread didn't start"));
    };
    queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(work)
        .map_err(|_| std::io::Error::other("the art thread has ended"))
}

/// The decoded pixels, once the art thread has them; `None` inside when they couldn't be read
type Slot = Arc<Mutex<Option<Option<egui::ColorImage>>>>;

enum State {
    Decoding(Slot),
    Ready(TextureHandle),
    Failed,
}

struct Entry {
    state: State,
    /// `input.time` when it was last asked for
    last: f64,
}

#[derive(Default)]
struct Cache {
    /// This context's folder instead of the engine's (tests)
    root: Option<PathBuf>,
    entries: HashMap<String, Entry>,
}

type Shared = Arc<Mutex<Cache>>;

fn cache(ctx: &egui::Context) -> Shared {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Shared>(egui::Id::NULL.with("pob-art"))
            .clone()
    })
}

/// Reads this context's images from `dir` instead of the engine's folder
pub fn set_context_root(ctx: &egui::Context, dir: &Path) {
    cache(ctx).lock().unwrap_or_else(|e| e.into_inner()).root = Some(dir.to_path_buf());
}

/// The image at `path` (relative to PoB's folder, as PoB names it), with its size in pixels,
/// once it is on the GPU. The first ask starts its decoding and returns `None`; a frame is asked
/// for when it is ready. A file that can't be read stays `None`.
pub fn image(ctx: &egui::Context, path: &str) -> Option<(TextureId, Vec2)> {
    image_with(ctx, path, false)
}

/// `image`, uploaded to repeat past its edges (a tiled background)
pub fn repeating_image(ctx: &egui::Context, path: &str) -> Option<(TextureId, Vec2)> {
    image_with(ctx, path, true)
}

fn image_with(ctx: &egui::Context, path: &str, repeat: bool) -> Option<(TextureId, Vec2)> {
    let shared = cache(ctx);
    let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
    let now = ctx.input(|i| i.time);
    let root = match &cache.root {
        Some(root) => root.clone(),
        None => ROOT.read().unwrap_or_else(|e| e.into_inner()).clone()?,
    };
    let key = if repeat {
        format!("{path}#repeat")
    } else {
        path.to_owned()
    };
    let entry = cache.entries.entry(key).or_insert_with(|| {
        let slot: Slot = Arc::default();
        let (done, file, ctx) = (Arc::clone(&slot), root.join(path), ctx.clone());
        let started = send_to_art_thread(Box::new(move || {
            let image = image::open(&file).ok().map(|i| {
                let rgba = i.into_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
            });
            *done.lock().unwrap_or_else(|e| e.into_inner()) = Some(image);
            ctx.request_repaint();
        }));
        Entry {
            state: if started.is_ok() {
                State::Decoding(slot)
            } else {
                State::Failed
            },
            last: now,
        }
    });
    entry.last = now;
    if let State::Decoding(slot) = &entry.state {
        // the pixels move into the texture: no copy is kept
        let decoded = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        match decoded {
            None => {}
            Some(None) => entry.state = State::Failed,
            Some(Some(pixels)) => {
                let mut options = egui::TextureOptions::LINEAR;
                if repeat {
                    options.wrap_mode = egui::TextureWrapMode::Repeat;
                }
                let texture = ctx.load_texture(format!("pob-art:{path}"), pixels, options);
                entry.state = State::Ready(texture);
            }
        }
    }
    match &entry.state {
        State::Ready(texture) => {
            let [w, h] = texture.size();
            Some((texture.id(), Vec2::new(w as f32, h as f32)))
        }
        _ => None,
    }
}

/// Frees the textures nothing asked for in [`KEEP_SECONDS`], and asks for a frame then while
/// one that this frame didn't draw is kept, so it is freed on an idle screen too. Called once a
/// frame, after everything drew.
pub fn sweep(ctx: &egui::Context) {
    let shared = cache(ctx);
    let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
    let now = ctx.input(|i| i.time);
    // a decode in flight is kept: its pixels would arrive to nobody
    cache
        .entries
        .retain(|_, e| matches!(e.state, State::Decoding(_)) || now - e.last < KEEP_SECONDS);
    // what this frame drew (a tab's background) is kept without waking an idle screen
    if let Some(oldest) = cache
        .entries
        .values()
        .filter(|e| matches!(e.state, State::Ready(_)) && e.last < now)
        .map(|e| e.last)
        .reduce(f64::min)
    {
        let wait = (oldest + KEEP_SECONDS - now).max(0.1);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }
}

/// Draws `path` over `rect`, tinted; nothing until it is loaded. Returns whether it drew.
pub fn paint(
    ctx: &egui::Context,
    painter: &egui::Painter,
    path: &str,
    rect: egui::Rect,
    tint: egui::Color32,
) -> bool {
    let Some((texture, _)) = image(ctx, path) else {
        return false;
    };
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(texture, rect, uv, tint);
    true
}

/// A `size` square for the image at `path`, drawn once it is loaded (a slot icon)
pub fn icon(ui: &mut egui::Ui, path: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint(ui.ctx(), ui.painter(), path, rect, egui::Color32::WHITE);
    response
}

/// PoB's background behind a tab (main:DrawBackground, Main.lua:1449-1458): the tree's
/// Background2 tile, 100 points a repeat from the view's corner, at half brightness
pub fn paint_background(ctx: &egui::Context, painter: &egui::Painter, rect: egui::Rect) {
    let Some((texture, _)) = background_tile(ctx) else {
        return;
    };
    let uv = egui::Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2(rect.width() / 100.0, rect.height() / 100.0),
    );
    painter.image(texture, rect, uv, TILE_TINT);
}

/// The background tile's texture, uploaded to repeat, once loaded
pub fn background_tile(ctx: &egui::Context) -> Option<(TextureId, Vec2)> {
    repeating_image(ctx, BACKGROUND_TILE)
}

/// PoB's half brightness for the background (`SetDrawColor(0.5, 0.5, 0.5)`)
pub const TILE_TINT: egui::Color32 = egui::Color32::from_gray(128);

/// The latest tree's Background2 (`main.tree[latestTreeVersion].assets.Background2`)
const BACKGROUND_TILE: &str = "TreeData/Background2.png";

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::RawInput;

    /// Runs a frame; returns how many textures it uploaded and which it freed
    fn frame(
        ctx: &egui::Context,
        time: f64,
        mut f: impl FnMut(&egui::Context),
    ) -> (usize, Vec<TextureId>) {
        let input = RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| f(ui.ctx()));
        let seen = (
            out.textures_delta.set.len(),
            out.textures_delta.free.iter().copied().collect(),
        );
        out.textures_delta.clear();
        seen
    }

    /// The image is decoded off the frame, uploaded once with its pixels dropped, and freed
    /// once nothing has drawn it for a while; a missing file stays missing
    #[test]
    fn images_load_off_the_frame_and_are_freed_when_unused() {
        let dir = crate::test_dir::TestDir::new("art");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]))
            .save(dir.join("a.png"))
            .unwrap();
        let ctx = egui::Context::default();
        set_context_root(&ctx, &dir);
        // the font atlas's upload
        frame(&ctx, 0.0, |_| {});
        let mut got = None;
        let mut uploads = 0;
        for i in 0..200 {
            let out = frame(&ctx, 0.01 * f64::from(i), |ctx| {
                got = image(ctx, "a.png");
                assert!(image(ctx, "missing.png").is_none());
            });
            uploads += out.0;
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (id, size) = got.expect("the image loads");
        assert_eq!(size, Vec2::new(3.0, 2.0));
        assert_eq!(uploads, 1);
        // asked for again: no second upload
        let out = frame(&ctx, 3.0, |ctx| {
            assert!(image(ctx, "a.png").is_some());
            sweep(ctx);
        });
        assert_eq!(out.0, 0);
        // unused past the wait: freed
        let out = frame(&ctx, 3.0 + KEEP_SECONDS + 0.1, sweep);
        let freed = frame(&ctx, 3.0 + KEEP_SECONDS + 0.2, |_| {});
        assert!(
            out.1.contains(&id) || freed.1.contains(&id),
            "the texture was freed"
        );
        assert!(cache(&ctx).lock().unwrap().entries.is_empty());
    }

    /// PoB's tab background: Background2 repeated every 100 points at half brightness
    #[test]
    fn the_background_tiles_every_100_points() {
        let dir = crate::test_dir::TestDir::new("art-bg");
        std::fs::create_dir_all(dir.join("TreeData")).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]))
            .save(dir.join("TreeData/Background2.png"))
            .unwrap();
        let ctx = egui::Context::default();
        set_context_root(&ctx, &dir);
        frame(&ctx, 0.0, |_| {});
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::new(300.0, 200.0));
        let mut mesh = None;
        for i in 0..200 {
            let input = RawInput {
                time: Some(0.01 * f64::from(i)),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| paint_background(ui.ctx(), ui.painter(), rect));
            out.textures_delta.clear();
            mesh = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Mesh(m) => Some(m.clone()),
                _ => None,
            });
            if mesh.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mesh = mesh.expect("the background is drawn once loaded");
        let max_uv = mesh
            .vertices
            .iter()
            .fold(egui::Pos2::ZERO, |a, v| a.max(v.uv));
        assert_eq!(max_uv, egui::pos2(3.0, 2.0));
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.color == egui::Color32::from_gray(128))
        );
    }
}
