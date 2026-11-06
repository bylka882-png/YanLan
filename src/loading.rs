//! Индикатор загрузки: анимированный GIF `assets/loading.gif`.
//!
//! У GIF чёрный фон, который на панелях лаунчера выглядел бы прямоугольником.
//! Поэтому кадры декодируются один раз, чёрный цвет превращается в прозрачность,
//! а готовые кадры уменьшаются и загружаются в текстуры egui.

use std::io::Cursor;
use std::time::{Duration, Instant};

use egui::{Color32, Sense, TextureHandle, TextureOptions, Ui, Vec2};

/// Кадры рисуются с запасом по размеру, чтобы индикатор оставался чётким
/// при любом масштабе интерфейса.
const FRAME: u32 = 192;
const GIF: &[u8] = include_bytes!("../assets/loading.gif");

pub struct Anim {
    frames: Vec<TextureHandle>,
    /// Накопительное время кадров в миллисекундах.
    stops: Vec<u32>,
    total: u32,
    start: Instant,
}

impl Anim {
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(GIF)).ok()?;
        let frames = image::AnimationDecoder::into_frames(decoder)
            .collect_frames()
            .ok()?;
        if frames.is_empty() {
            return None;
        }

        let mut textures = Vec::with_capacity(frames.len());
        let mut stops = Vec::with_capacity(frames.len());
        let mut total = 0u32;
        for (index, frame) in frames.into_iter().enumerate() {
            let (num, den) = frame.delay().numer_denom_ms();
            let delay = if den == 0 {
                100
            } else {
                (num / den).clamp(20, 1000)
            };
            total += delay;
            stops.push(total);

            let rgba = frame.into_buffer();
            let small = image::imageops::thumbnail(&rgba, FRAME, FRAME);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [small.width() as usize, small.height() as usize],
                &key_black(&small),
            );
            textures.push(ctx.load_texture(
                format!("loading-frame-{index}"),
                image,
                TextureOptions::LINEAR,
            ));
        }

        Some(Self {
            frames: textures,
            stops,
            total: total.max(1),
            start: Instant::now(),
        })
    }

    /// Рисует текущий кадр индикатора заданного размера.
    pub fn show(&self, ui: &mut Ui, size: f32) {
        let elapsed = self.start.elapsed().as_millis() as u32 % self.total;
        let index = self
            .stops
            .iter()
            .position(|stop| elapsed < *stop)
            .unwrap_or(self.frames.len() - 1);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let painter = ui.painter();
        painter.image(
            self.frames[index].id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

/// Заменяет чёрный (и почти чёрный) фон на прозрачность с мягкой кромкой.
fn key_black(image: &image::RgbaImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.len() * 4);
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        let max = r.max(g).max(b) as f32;
        // Ниже 10 — чистый фон, выше 52 — сам рисунок. Между ними плавный
        // переход: так пропадает тёмная кайма вокруг силуэта.
        let keyed = ((max - 10.0) / 42.0).clamp(0.0, 1.0);
        let alpha = (a as f32 / 255.0 * keyed * 255.0).round() as u8;
        out.extend_from_slice(&[r, g, b, alpha]);
    }
    out
}
