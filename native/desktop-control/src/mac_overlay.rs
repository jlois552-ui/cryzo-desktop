//! The on-screen sign that Cryzo is using the Mac, like Windows' overlay: a glow around every
//! display, a pill naming the step with a Stop button, and an agent cursor that glides to each
//! target and outlines it. AppKit windows belong to a process's main thread, which in Cryzo runs
//! the app or the MCP loop, so a helper process draws them: the same executable started with
//! [`HELPER`], fed [`Signal`]s as JSON lines on stdin and answering `stop` on stdout. Every
//! window is left out of screenshots and screen sharing, and all but the pill let clicks through.

use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        mpsc::{self, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use core_graphics::{
    base::{
        kCGBitmapByteOrder32Little, kCGImageAlphaPremultipliedFirst, kCGRenderingIntentDefault,
    },
    color_space::CGColorSpace,
    data_provider::CGDataProvider,
    image::CGImage,
};
use foreign_types::ForeignType;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::{
    common::{Rect, is_stopped, stop},
    motion::{
        Animator, Frame, Indicator, POINTER, Signal, TIP, glow_strips, outline_pad,
        paint_glow_strip, paint_outline, paint_pointer,
    },
    pill::Painter,
};

/// The argument that makes a Cryzo executable run as the overlay helper.
pub(crate) const HELPER: &str = "--cryzo-desktop-overlay";

/// Starts the overlay helper. None when it could not start, which leaves desktop control
/// working without the overlay.
pub(crate) fn start() -> Option<Indicator> {
    let mut child = Command::new(std::env::current_exe().ok()?)
        .arg(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| eprintln!("cryzo-desktop: could not start the on-screen overlay: {error}"))
        .ok()?;
    let mut input = child.stdin.take()?;
    let output = child.stdout.take()?;
    let (sender, signals) = mpsc::channel::<Signal>();
    std::thread::Builder::new()
        .name("cryzo-overlay".into())
        .spawn(move || {
            let mut told = false;
            loop {
                let signal = match signals.recv_timeout(Duration::from_millis(100)) {
                    Ok(signal) => Some(signal),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                // Ctrl+Option+Esc or the pill's Stop: the pill says so until the next turn.
                let stopped = (is_stopped() && !told).then_some(Signal::Stopped);
                told = is_stopped() && !matches!(signal, Some(Signal::Resume));
                let lines = signal
                    .into_iter()
                    .chain(stopped)
                    .filter_map(|signal| serde_json::to_string(&signal).ok())
                    .map(|line| format!("{line}\n"))
                    .collect::<String>();
                if !lines.is_empty() && input.write_all(lines.as_bytes()).is_err() {
                    break;
                }
            }
            // Closing its input ends the helper.
            drop(input);
            let _ = child.wait();
        })
        .ok()?;
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if line.trim() == "stop" {
                stop();
            }
        }
    });
    Some(Indicator::new(sender))
}

/// The helper process: draws the overlay until its input closes.
pub(crate) fn run() {
    let (sender, signals) = mpsc::channel::<Signal>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Ok(signal) = serde_json::from_str::<Signal>(&line)
                && sender.send(signal).is_err()
            {
                break;
            }
        }
    });
    let app: Retained<AnyObject> = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    // An accessory app has no Dock icon or menu bar, and never takes the focus.
    let _: bool = unsafe { msg_send![&*app, setActivationPolicy: 1isize] };
    let _: () = unsafe { msg_send![&*app, finishLaunching] };
    let mut overlay = Overlay::new();
    let mut animator = Animator::new(Instant::now());
    let mode = NSString::from_str("kCFRunLoopDefaultMode");
    loop {
        let open = autoreleasepool(|_| {
            let wait = if overlay.visible { 0.016 } else { 0.1 };
            let mut until: Retained<AnyObject> =
                unsafe { msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: wait] };
            loop {
                let event: Option<Retained<AnyObject>> = unsafe {
                    msg_send![&*app, nextEventMatchingMask: u64::MAX, untilDate: &*until, inMode: &*mode, dequeue: true]
                };
                let Some(event) = event else {
                    break;
                };
                overlay.event(&event);
                let _: () = unsafe { msg_send![&*app, sendEvent: &*event] };
                until = unsafe { msg_send![class!(NSDate), distantPast] };
            }
            let now = Instant::now();
            loop {
                match signals.try_recv() {
                    Ok(signal) => animator.signal(signal, now),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return false,
                }
            }
            if overlay.stop_clicked {
                overlay.stop_clicked = false;
                animator.signal(Signal::Stopped, now);
                println!("stop");
                let _ = std::io::stdout().flush();
            }
            let frame = animator.frame(now);
            overlay.draw(frame, animator.anchor());
            true
        });
        if !open {
            return;
        }
    }
}

/// The overlay's windows. Rectangles are in Accessibility coordinates: points from the top left
/// of the main display, as element frames come.
struct Overlay {
    scale: f32,
    /// The main display's height, for turning those into AppKit's bottom-left coordinates.
    height: f64,
    monitors: Vec<Rect>,
    glow: Vec<(Retained<AnyObject>, Rect, Rect)>,
    pointer: Retained<AnyObject>,
    outline: Retained<AnyObject>,
    pill: Retained<AnyObject>,
    painter: Painter,
    visible: bool,
    glow_alpha: u8,
    shown_pill: Option<(String, bool, Rect)>,
    /// Where Stop starts, in points from the pill's left edge.
    stop_from: Option<f64>,
    stop_clicked: bool,
    shown_outline: Option<Rect>,
    pointer_shown: bool,
    ripple: bool,
}

impl Overlay {
    fn new() -> Self {
        let screens: Retained<AnyObject> = unsafe { msg_send![class!(NSScreen), screens] };
        let count: usize = unsafe { msg_send![&*screens, count] };
        let frames = (0..count)
            .map(|index| {
                let screen: Retained<AnyObject> =
                    unsafe { msg_send![&*screens, objectAtIndex: index] };
                let frame: NSRect = unsafe { msg_send![&*screen, frame] };
                frame
            })
            .collect::<Vec<_>>();
        let main: Option<Retained<AnyObject>> = unsafe { msg_send![class!(NSScreen), mainScreen] };
        let scale = main.map_or(2.0, |screen| {
            let factor: f64 = unsafe { msg_send![&*screen, backingScaleFactor] };
            factor
        }) as f32;
        let height = frames.first().map_or(0.0, |frame| frame.size.height);
        let monitors = frames
            .iter()
            .map(|frame| Rect {
                left: frame.origin.x as i32,
                top: (height - frame.origin.y - frame.size.height) as i32,
                right: (frame.origin.x + frame.size.width) as i32,
                bottom: (height - frame.origin.y) as i32,
            })
            .collect::<Vec<_>>();
        let glow = monitors
            .iter()
            .flat_map(|monitor| {
                glow_strips(*monitor, 1.0)
                    .into_iter()
                    .map(|strip| (panel(true), strip, *monitor))
            })
            .collect();
        Self {
            scale,
            height,
            monitors,
            glow,
            pointer: panel(true),
            outline: panel(true),
            pill: panel(false),
            painter: Painter::load(),
            visible: false,
            glow_alpha: 0,
            shown_pill: None,
            stop_from: None,
            stop_clicked: false,
            shown_outline: None,
            pointer_shown: false,
            ripple: false,
        }
    }

    /// A click on the pill's Stop button.
    fn event(&mut self, event: &AnyObject) {
        // NSEventTypeLeftMouseDown
        let kind: usize = unsafe { msg_send![event, type] };
        if kind != 1 {
            return;
        }
        let window: Option<Retained<AnyObject>> = unsafe { msg_send![event, window] };
        if window.is_none_or(|window| !std::ptr::eq(&*window, &*self.pill)) {
            return;
        }
        let point: NSPoint = unsafe { msg_send![event, locationInWindow] };
        if self.stop_from.is_some_and(|from| point.x >= from) {
            self.stop_clicked = true;
            stop();
        }
    }

    fn draw(&mut self, frame: Option<Frame>, anchor: Option<(f32, f32)>) {
        let Some(frame) = frame else {
            if self.visible {
                for (window, ..) in &self.glow {
                    hide(window);
                }
                for window in [&self.pointer, &self.outline, &self.pill] {
                    hide(window);
                }
                self.visible = false;
                self.shown_pill = None;
                self.shown_outline = None;
                self.pointer_shown = false;
            }
            return;
        };
        let scale = self.scale;
        if !self.visible {
            for (window, strip, monitor) in &self.glow {
                let pixels =
                    paint_glow_strip(scaled(*strip, scale), scaled(*monitor, scale), scale);
                self.place(window, *strip, Some(&pixels));
            }
            self.visible = true;
        }
        let alpha = (frame.glow * 255.0) as u8;
        if alpha.abs_diff(self.glow_alpha) >= 3 {
            for (window, ..) in &self.glow {
                let _: () =
                    unsafe { msg_send![&**window, setAlphaValue: f64::from(alpha) / 255.0] };
            }
            self.glow_alpha = alpha;
        }
        let monitor = anchor
            .and_then(|(x, y)| {
                self.monitors.iter().copied().find(|monitor| {
                    (monitor.left..monitor.right).contains(&(x as i32))
                        && (monitor.top..monitor.bottom).contains(&(y as i32))
                })
            })
            .or(self.monitors.first().copied())
            .unwrap_or_default();
        if self
            .shown_pill
            .as_ref()
            .is_none_or(|(label, stopped, area)| {
                *label != frame.label
                    || *stopped != frame.stopped
                    || area.left < monitor.left
                    || area.right > monitor.right
            })
        {
            let image = self.painter.paint(&frame.label, frame.stopped, scale);
            let (width, height) = (
                (image.width as f32 / scale) as i32,
                (image.height as f32 / scale) as i32,
            );
            let left = monitor.left + (monitor.right - monitor.left - width) / 2;
            let top = monitor.top + 12;
            let area = Rect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            };
            self.place(&self.pill, area, Some(&image.pixels));
            self.stop_from = image.stop_from.map(|from| f64::from(from as f32 / scale));
            self.shown_pill = Some((frame.label.clone(), frame.stopped, area));
        }
        if frame.outline != self.shown_outline {
            let pad = (outline_pad(scale) as f32 / scale) as i32;
            let painted = frame.outline.and_then(|rect| {
                let area = Rect {
                    left: rect.left - pad,
                    top: rect.top - pad,
                    right: rect.right + pad,
                    bottom: rect.bottom + pad,
                };
                let pixels = scaled(area, scale);
                paint_outline(
                    pixels.right - pixels.left,
                    pixels.bottom - pixels.top,
                    scale,
                )
                .map(|image| (area, image))
            });
            match painted {
                Some((area, image)) => self.place(&self.outline, area, Some(&image)),
                None => hide(&self.outline),
            }
            self.shown_outline = frame.outline;
        }
        let Some((x, y)) = frame.pointer else {
            if self.pointer_shown {
                hide(&self.pointer);
                self.pointer_shown = false;
            }
            return;
        };
        let size = POINTER as i32;
        let area = Rect {
            left: (x - TIP) as i32,
            top: (y - TIP) as i32,
            right: (x - TIP) as i32 + size,
            bottom: (y - TIP) as i32 + size,
        };
        // The image changes only while a ripple runs, and once more when it ends.
        let repaint = !self.pointer_shown || frame.ripple.is_some() || self.ripple;
        self.ripple = frame.ripple.is_some();
        let pixels = repaint.then(|| {
            let side = (POINTER * scale) as i32;
            let mut pixels = vec![0u32; (side * side) as usize];
            paint_pointer(&mut pixels, side, scale, frame.ripple);
            pixels
        });
        self.place(&self.pointer, area, pixels.as_deref());
        self.pointer_shown = true;
    }

    /// Moves a window to `area`, shows it on top, and sets its image when given.
    fn place(&self, window: &AnyObject, area: Rect, pixels: Option<&[u32]>) {
        let frame = NSRect::new(
            NSPoint::new(f64::from(area.left), self.height - f64::from(area.bottom)),
            NSSize::new(
                f64::from(area.right - area.left),
                f64::from(area.bottom - area.top),
            ),
        );
        let _: () = unsafe { msg_send![window, setFrame: frame, display: false] };
        if let Some(pixels) = pixels {
            let pixel_area = scaled(area, self.scale);
            let image = image(
                (pixel_area.right - pixel_area.left) as usize,
                (pixel_area.bottom - pixel_area.top) as usize,
                pixels,
            );
            let view: Retained<AnyObject> = unsafe { msg_send![window, contentView] };
            let layer: Option<Retained<AnyObject>> = unsafe { msg_send![&*view, layer] };
            if let (Some(layer), Some(image)) = (layer, image) {
                let contents = image.as_ptr() as *const AnyObject;
                let _: () = unsafe { msg_send![&*layer, setContents: contents] };
                let _: () = unsafe { msg_send![&*layer, setContentsScale: f64::from(self.scale)] };
            }
        }
        let _: () = unsafe { msg_send![window, orderFrontRegardless] };
    }
}

fn hide(window: &AnyObject) {
    let _: () = unsafe { msg_send![window, orderOut: std::ptr::null::<AnyObject>()] };
}

fn scaled(rect: Rect, scale: f32) -> Rect {
    let at = |value: i32| (value as f32 * scale).round() as i32;
    Rect {
        left: at(rect.left),
        top: at(rect.top),
        right: at(rect.right),
        bottom: at(rect.bottom),
    }
}

/// Premultiplied ARGB pixels as an image for a layer. None when the size and pixels disagree.
fn image(width: usize, height: usize, pixels: &[u32]) -> Option<CGImage> {
    if width == 0 || height == 0 || pixels.len() != width * height {
        return None;
    }
    let bytes = pixels
        .iter()
        .flat_map(|pixel| pixel.to_le_bytes())
        .collect::<Vec<_>>();
    Some(CGImage::new(
        width,
        height,
        8,
        32,
        width * 4,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little,
        &CGDataProvider::from_buffer(Arc::new(bytes)),
        false,
        kCGRenderingIntentDefault,
    ))
}

/// A borderless, transparent panel above everything, on every Space, left out of screenshots.
/// `passive` ones let every click through.
fn panel(passive: bool) -> Retained<AnyObject> {
    let empty = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    // NSWindowStyleMaskNonactivatingPanel: clicking Stop doesn't take the focus from the app
    // being driven.
    let style = 1usize << 7;
    let window: Allocated<AnyObject> = unsafe { msg_send![class!(NSPanel), alloc] };
    // NSBackingStoreBuffered
    let window: Retained<AnyObject> = unsafe {
        msg_send![window, initWithContentRect: empty, styleMask: style, backing: 2usize, defer: false]
    };
    let clear: Retained<AnyObject> = unsafe { msg_send![class!(NSColor), clearColor] };
    unsafe {
        let _: () = msg_send![&*window, setOpaque: false];
        let _: () = msg_send![&*window, setBackgroundColor: &*clear];
        let _: () = msg_send![&*window, setHasShadow: false];
        // NSScreenSaverWindowLevel: above other apps' windows and the menu bar.
        let _: () = msg_send![&*window, setLevel: 1000isize];
        let _: () = msg_send![&*window, setIgnoresMouseEvents: passive];
        // NSWindowSharingNone: screenshots, OCR and screen sharing never see it.
        let _: () = msg_send![&*window, setSharingType: 0usize];
        // Every Space, unaffected by Mission Control, beside full-screen apps.
        let _: () = msg_send![&*window, setCollectionBehavior: 1usize | 1 << 4 | 1 << 8];
        let _: () = msg_send![&*window, setReleasedWhenClosed: false];
        let _: () = msg_send![&*window, setHidesOnDeactivate: false];
        let _: () = msg_send![&*window, setBecomesKeyOnlyIfNeeded: true];
        let view: Retained<AnyObject> = msg_send![&*window, contentView];
        let _: () = msg_send![&*view, setWantsLayer: true];
    }
    window
}
