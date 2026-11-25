//! The on-screen sign that Cryzo is using a Linux desktop, drawn with X11 like Windows' overlay: a
//! glow around every monitor, a pill naming the step with a Stop button, and an agent cursor that
//! glides to each target and outlines it. The windows are override-redirect ARGB windows that let
//! clicks through (all but the pill), so they need a compositing window manager; without one
//! there is no overlay, since transparent windows would paint black.

use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        Event,
        randr::ConnectionExt as _,
        shape,
        xfixes::ConnectionExt as _,
        xproto::{
            AtomEnum, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
            CreateWindowAux, EventMask, Gcontext, ImageFormat, PropMode, StackMode, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{
    common::{Rect, is_stopped, stop},
    motion::{
        Animator, Frame, Indicator, POINTER, Signal, TIP, glow_strips, outline_pad,
        paint_glow_strip, paint_outline, paint_pointer,
    },
    pill::Painter,
};

const FRAME: Duration = Duration::from_millis(16);

/// Starts the overlay thread. None when there is no X11 display or no compositor.
pub(crate) fn start() -> Option<Indicator> {
    let (sender, signals) = mpsc::channel();
    let (ready, started) = mpsc::channel();
    std::thread::Builder::new()
        .name("cryzo-overlay".into())
        .spawn(move || match Screen::open() {
            Ok(screen) => {
                let _ = ready.send(true);
                run(screen, signals);
            }
            Err(error) => {
                eprintln!("cryzo-desktop: no on-screen overlay: {error:#}");
                let _ = ready.send(false);
            }
        })
        .ok()?;
    started
        .recv_timeout(Duration::from_secs(3))
        .ok()
        .filter(|ok| *ok)?;
    Some(Indicator::new(sender))
}

/// One overlay window and what it shows now.
struct Surface {
    window: u32,
    gc: Gcontext,
    mapped: bool,
    area: Rect,
}

struct Screen {
    connection: RustConnection,
    root: u32,
    visual: u32,
    colormap: u32,
    opacity: u32,
    scale: f32,
    monitors: Vec<Rect>,
    glow: Vec<Surface>,
    pointer: Surface,
    outline: Surface,
    pill: Surface,
    painter: Painter,
}

impl Screen {
    fn open() -> Result<Self> {
        let (connection, number) = x11rb::connect(None).context("no X11 display")?;
        let screen = &connection.setup().roots[number];
        let root = screen.root;
        let compositor = connection
            .intern_atom(false, format!("_NET_WM_CM_S{number}").as_bytes())?
            .reply()?
            .atom;
        if connection.get_selection_owner(compositor)?.reply()?.owner == 0 {
            bail!("no compositing window manager is running");
        }
        let visual = screen
            .allowed_depths
            .iter()
            .filter(|depth| depth.depth == 32)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.class == x11rb::protocol::xproto::VisualClass::TRUE_COLOR)
            .map(|visual| visual.visual_id)
            .context("the display has no 32-bit visual for transparent windows")?;
        let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
        let colormap = connection.generate_id()?;
        connection.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;
        connection.xfixes_query_version(5, 0)?.reply()?;
        let opacity = connection
            .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?
            .reply()?
            .atom;
        let monitors = connection
            .randr_get_monitors(root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| {
                reply
                    .monitors
                    .iter()
                    .map(|monitor| Rect {
                        left: monitor.x.into(),
                        top: monitor.y.into(),
                        right: i32::from(monitor.x) + i32::from(monitor.width),
                        bottom: i32::from(monitor.y) + i32::from(monitor.height),
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|monitors| !monitors.is_empty())
            .unwrap_or_else(|| {
                vec![Rect {
                    left: 0,
                    top: 0,
                    right: width.into(),
                    bottom: height.into(),
                }]
            });
        // X11 has no system scale; desktops that scale say so to toolkits through these.
        let scale = ["GDK_SCALE", "QT_SCALE_FACTOR"]
            .iter()
            .find_map(|name| std::env::var(name).ok()?.parse::<f32>().ok())
            .filter(|scale| *scale >= 1.0)
            .unwrap_or(1.0);
        let mut screen = Self {
            connection,
            root,
            visual,
            colormap,
            opacity,
            scale,
            monitors,
            glow: Vec::new(),
            pointer: Surface::empty(),
            outline: Surface::empty(),
            pill: Surface::empty(),
            painter: Painter::load(),
        };
        let strips = screen
            .monitors
            .clone()
            .into_iter()
            .flat_map(|monitor| glow_strips(monitor, scale))
            .collect::<Vec<_>>();
        for strip in strips {
            let mut surface = screen.surface(true)?;
            surface.area = strip;
            screen.glow.push(surface);
        }
        screen.pointer = screen.surface(true)?;
        screen.outline = screen.surface(true)?;
        screen.pill = screen.surface(false)?;
        screen.connection.flush()?;
        Ok(screen)
    }

    /// A hidden override-redirect ARGB window. `passive` ones let every click through.
    fn surface(&self, passive: bool) -> Result<Surface> {
        let window = self.connection.generate_id()?;
        self.connection.create_window(
            32,
            window,
            self.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            self.visual,
            &CreateWindowAux::new()
                .background_pixel(0)
                .border_pixel(0)
                .colormap(self.colormap)
                .override_redirect(1)
                .event_mask(if passive {
                    EventMask::NO_EVENT
                } else {
                    EventMask::BUTTON_PRESS
                }),
        )?;
        if passive {
            let region = self.connection.generate_id()?;
            self.connection.xfixes_create_region(region, &[])?;
            self.connection.xfixes_set_window_shape_region(
                window,
                shape::SK::INPUT,
                0,
                0,
                region,
            )?;
            self.connection.xfixes_destroy_region(region)?;
        }
        let gc = self.connection.generate_id()?;
        self.connection.create_gc(gc, window, &CreateGCAux::new())?;
        Ok(Surface {
            window,
            gc,
            mapped: false,
            area: Rect::default(),
        })
    }

    /// The monitor that holds a point, else the first.
    fn monitor(&self, point: Option<(f32, f32)>) -> Rect {
        point
            .and_then(|(x, y)| {
                self.monitors.iter().copied().find(|monitor| {
                    (monitor.left..monitor.right).contains(&(x as i32))
                        && (monitor.top..monitor.bottom).contains(&(y as i32))
                })
            })
            .unwrap_or(self.monitors[0])
    }
}

impl Surface {
    fn empty() -> Self {
        Self {
            window: 0,
            gc: 0,
            mapped: false,
            area: Rect::default(),
        }
    }

    /// Moves the window to `area`, shows it on top, and paints `pixels` into it when given.
    fn place(&mut self, x: &RustConnection, area: Rect, pixels: Option<&[u32]>) -> Result<()> {
        let (width, height) = (area.right - area.left, area.bottom - area.top);
        if width <= 0 || height <= 0 {
            return self.hide(x);
        }
        if self.area != area || !self.mapped {
            x.configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .x(area.left)
                    .y(area.top)
                    .width(width as u32)
                    .height(height as u32)
                    .stack_mode(StackMode::ABOVE),
            )?;
            self.area = area;
        }
        if !self.mapped {
            x.map_window(self.window)?;
            self.mapped = true;
        }
        if let Some(pixels) = pixels {
            self.paint(x, width, pixels)?;
        }
        Ok(())
    }

    /// Uploads an image in strips that fit the server's request size.
    fn paint(&self, x: &RustConnection, width: i32, pixels: &[u32]) -> Result<()> {
        let bytes = pixels
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect::<Vec<_>>();
        let row = (width * 4) as usize;
        let rows = ((x.maximum_request_bytes() - 64) / row).max(1);
        for (index, chunk) in bytes.chunks(row * rows).enumerate() {
            x.put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                width as u16,
                (chunk.len() / row) as u16,
                0,
                (index * rows) as i16,
                0,
                32,
                chunk,
            )?;
        }
        Ok(())
    }

    fn hide(&mut self, x: &RustConnection) -> Result<()> {
        if self.mapped {
            x.unmap_window(self.window)?;
            self.mapped = false;
        }
        Ok(())
    }
}

/// What was last put on screen, so each frame only sends what changed.
#[derive(Default)]
struct Shown {
    visible: bool,
    glow: u8,
    pill: Option<(String, bool, Rect)>,
    stop_from: Option<i32>,
    outline: Option<Rect>,
    ripple: bool,
}

fn run(mut screen: Screen, signals: Receiver<Signal>) {
    let mut animator = Animator::new(Instant::now());
    let mut shown = Shown::default();
    let mut told_stopped = false;
    loop {
        let first = if shown.visible {
            match signals.recv_timeout(FRAME) {
                Ok(signal) => Some(signal),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match signals.recv() {
                Ok(signal) => Some(signal),
                Err(_) => return,
            }
        };
        let now = Instant::now();
        for signal in first.into_iter().chain(signals.try_iter()) {
            if matches!(signal, Signal::Resume) {
                told_stopped = false;
            }
            animator.signal(signal, now);
        }
        // Ctrl+Alt+Esc or the pill's Stop button: the pill says so until the next turn.
        if is_stopped() && !told_stopped {
            told_stopped = true;
            animator.signal(Signal::Stopped, now);
        }
        while let Ok(Some(event)) = screen.connection.poll_for_event() {
            if let Event::ButtonPress(press) = event
                && press.event == screen.pill.window
                && shown
                    .stop_from
                    .is_some_and(|from| i32::from(press.event_x) >= from)
            {
                stop();
            }
        }
        let frame = animator.frame(now);
        if draw(&mut screen, &mut shown, frame, animator.anchor()).is_err()
            || screen.connection.flush().is_err()
        {
            return;
        }
    }
}

fn draw(
    screen: &mut Screen,
    shown: &mut Shown,
    frame: Option<Frame>,
    anchor: Option<(f32, f32)>,
) -> Result<()> {
    let x = &screen.connection;
    let Some(frame) = frame else {
        if shown.visible {
            for surface in screen.glow.iter_mut().chain([
                &mut screen.pointer,
                &mut screen.outline,
                &mut screen.pill,
            ]) {
                surface.hide(x)?;
            }
            *shown = Shown::default();
        }
        return Ok(());
    };
    let scale = screen.scale;
    if !shown.visible {
        for (index, surface) in screen.glow.iter_mut().enumerate() {
            let strip = surface.area;
            let pixels = paint_glow_strip(strip, screen.monitors[index / 4], scale);
            surface.place(x, strip, Some(&pixels))?;
        }
        shown.visible = true;
    }
    // The compositor fades the glow; only visible steps are sent.
    let glow = (frame.glow * 255.0) as u8;
    if glow.abs_diff(shown.glow) >= 3 {
        let value = (f64::from(u32::MAX) * f64::from(glow) / 255.0) as u32;
        for surface in &screen.glow {
            x.change_property32(
                PropMode::REPLACE,
                surface.window,
                screen.opacity,
                AtomEnum::CARDINAL,
                &[value],
            )?;
        }
        shown.glow = glow;
    }
    let monitor = screen.monitor(anchor);
    if shown.pill.as_ref().is_none_or(|(label, stopped, area)| {
        *label != frame.label
            || *stopped != frame.stopped
            || area.left < monitor.left
            || area.right > monitor.right
    }) {
        let image = screen.painter.paint(&frame.label, frame.stopped, scale);
        let left = monitor.left + (monitor.right - monitor.left - image.width) / 2;
        let top = monitor.top + (12.0 * scale) as i32;
        let area = Rect {
            left,
            top,
            right: left + image.width,
            bottom: top + image.height,
        };
        screen.pill.place(x, area, Some(&image.pixels))?;
        shown.pill = Some((frame.label.clone(), frame.stopped, area));
        shown.stop_from = image.stop_from;
    }
    if frame.outline != shown.outline {
        let pad = outline_pad(scale);
        let painted = frame.outline.and_then(|rect| {
            let area = Rect {
                left: rect.left - pad,
                top: rect.top - pad,
                right: rect.right + pad,
                bottom: rect.bottom + pad,
            };
            paint_outline(area.right - area.left, area.bottom - area.top, scale)
                .map(|pixels| (area, pixels))
        });
        match painted {
            Some((area, pixels)) => screen.outline.place(x, area, Some(&pixels))?,
            None => screen.outline.hide(x)?,
        }
        shown.outline = frame.outline;
    }
    let Some((left, top)) = frame.pointer else {
        return screen.pointer.hide(x);
    };
    let size = (POINTER * scale) as i32;
    let tip = (TIP * scale) as i32;
    let area = Rect {
        left: left as i32 - tip,
        top: top as i32 - tip,
        right: left as i32 - tip + size,
        bottom: top as i32 - tip + size,
    };
    // The image changes only while a ripple runs, and once more when it ends.
    let repaint = !screen.pointer.mapped || frame.ripple.is_some() || shown.ripple;
    shown.ripple = frame.ripple.is_some();
    let pixels = repaint.then(|| {
        let mut pixels = vec![0u32; (size * size) as usize];
        paint_pointer(&mut pixels, size, scale, frame.ripple);
        pixels
    });
    screen.pointer.place(x, area, pixels.as_deref())
}
