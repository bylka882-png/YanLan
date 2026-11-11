//! Встроенный плеер YouTube: дочерний WebView2 поверх окна лаунчера.
//!
//! Видео из описания мода открывается не в браузере, а здесь же. WebView2 —
//! отдельное дочернее окно, поэтому его границы задаются в физических
//! пикселях и обновляются при изменении размера окна.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use egui::{vec2, Color32, Id, LayerId, Order, Rect, RichText, Sense};

use crate::app::App;
use crate::i18n::{fill, pick};
use crate::mods;
use crate::ui::widgets as w;
use crate::util;

pub struct Player {
    view: wry::WebView,
    bounds: Option<(i32, i32, u32, u32)>,
    /// Локальный сервер с HTML-обёрткой: у iframe должен быть настоящий
    /// Referer, иначе YouTube отдаёт «Ошибку 153».
    _server: LocalServer,
}

impl Player {
    pub fn new(
        frame: &eframe::Frame,
        id: &str,
        rect: Rect,
        pixels_per_point: f32,
    ) -> Result<Self, String> {
        let server = LocalServer::start(id)?;
        let key = physical_key(rect, pixels_per_point);
        let bounds = physical(rect, pixels_per_point);
        let view = wry::WebViewBuilder::new_as_child(frame)
            .with_url(server.url())
            .with_bounds(bounds)
            .with_autoplay(true)
            .with_background_color((0, 0, 0, 255))
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self {
            view,
            bounds: Some(key),
            _server: server,
        })
    }

    pub fn set_bounds(&mut self, rect: Rect, pixels_per_point: f32) {
        let key = physical_key(rect, pixels_per_point);
        if self.bounds == Some(key) {
            return;
        }
        if self
            .view
            .set_bounds(physical(rect, pixels_per_point))
            .is_ok()
        {
            self.bounds = Some(key);
        }
    }
}

/// Локальный HTTP-сервер с одной страницей: iframe YouTube.
struct LocalServer {
    port: u16,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LocalServer {
    fn start(id: &str) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let page = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
             <style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}\
             iframe{{width:100%;height:100%;border:0}}</style></head><body>\
             <iframe src=\"https://www.youtube.com/embed/{id}?autoplay=1&rel=0&modestbranding=1&playsinline=1\" \
             allow=\"autoplay; encrypted-media; fullscreen\" allowfullscreen></iframe>\
             </body></html>"
        );
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                let mut buffer = [0u8; 2048];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                // Страница нужна только для iframe: favicon и прочее — 404.
                let response = if path == "/" || path.starts_with("/?") {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        page.len(),
                        page
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\
                     Connection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Ok(Self {
            port,
            shutdown,
            thread: Some(thread),
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Соединение нужно, чтобы разбудить accept и выйти из цикла.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn physical_key(rect: Rect, pixels_per_point: f32) -> (i32, i32, u32, u32) {
    (
        (rect.left() * pixels_per_point).round() as i32,
        (rect.top() * pixels_per_point).round() as i32,
        (rect.width() * pixels_per_point).round().max(1.0) as u32,
        (rect.height() * pixels_per_point).round().max(1.0) as u32,
    )
}

fn physical(rect: Rect, pixels_per_point: f32) -> wry::Rect {
    let (x, y, width, height) = physical_key(rect, pixels_per_point);
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(x, y).into(),
        size: wry::dpi::PhysicalSize::new(width, height).into(),
    }
}

impl App {
    /// Начинает воспроизведение ролика во встроенном окне.
    pub fn watch_youtube(&mut self, id: impl Into<String>) {
        let id = id.into();
        if self.player_id.as_deref() != Some(id.as_str()) {
            self.player = None;
        }
        self.player_id = Some(id);
    }

    pub fn close_player(&mut self) {
        self.player_id = None;
        self.player = None;
    }

    pub fn draw_player(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(id) = self.player_id.clone() else {
            self.player = None;
            return;
        };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close_player();
            return;
        }

        let p = self.p;
        let screen = ctx.screen_rect();
        let ppp = ctx.pixels_per_point();
        let width = (screen.width() * 0.72)
            .clamp(420.0, 960.0)
            .min((screen.width() - 32.0).max(320.0));
        let video_h = (width * 9.0 / 16.0).min((screen.height() - 150.0).max(200.0));
        let total = video_h + 68.0;
        let panel = Rect::from_center_size(screen.center(), vec2(width, total));

        ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("player_dim")))
            .rect_filled(
                screen,
                0.0,
                Color32::from_black_alpha(if self.cfg.light_theme { 70 } else { 160 }),
            );

        let mut close = false;
        let mut video_rect = None;
        egui::Area::new(Id::new("player_panel"))
            .order(Order::Foreground)
            .fixed_pos(panel.min)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(p.panel)
                    .stroke(egui::Stroke::new(1.0_f32, p.border))
                    .rounding(14.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .shadow(egui::epaint::Shadow {
                        offset: vec2(0.0, 12.0),
                        blur: 32.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(if self.cfg.light_theme {
                            60
                        } else {
                            160
                        }),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(pick("ВОСПРОИЗВЕДЕНИЕ", "NOW PLAYING"))
                                    .size(10.5)
                                    .strong()
                                    .color(p.muted),
                            );
                            ui.add_space(4.0);
                            ui.label(RichText::new("YouTube").size(11.0).color(p.accent));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if w::close(ui, &p).clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(width - 24.0, video_h), Sense::hover());
                        video_rect = Some(rect);
                    });
            });

        if let Some(rect) = video_rect {
            if self.player.is_none() {
                match Player::new(frame, &id, rect, ppp) {
                    Ok(player) => self.player = Some(player),
                    Err(err) => {
                        self.toast(
                            fill(
                                &pick(
                                    "Встроенный плеер недоступен: {err}",
                                    "The built-in player is unavailable: {err}",
                                ),
                                &[("err", &err)],
                            ),
                            false,
                        );
                        util::open_url(&mods::youtube_watch_url(&id));
                        self.player_id = None;
                    }
                }
            } else if let Some(player) = &mut self.player {
                player.set_bounds(rect, ppp);
            }
        }

        if close {
            self.close_player();
        }
    }
}
