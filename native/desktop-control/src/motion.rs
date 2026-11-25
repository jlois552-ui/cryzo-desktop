//! What the on-screen overlay does, shared by every platform: when the glow and pill show, what
//! the pill says, where the agent cursor is along its glide, and the pixels of the glow, cursor and
//! outline. Each platform only puts these on screen. Windows draws with these helpers in
//! `overlay.rs`; macOS and Linux also run their whole behaviour through [`Animator`].

use std::{
    sync::{Arc, Mutex, mpsc::Sender},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::common::Rect;

/// Cryzo violet.
pub(crate) const ACCENT: (f32, f32, f32) = (124.0, 92.0, 255.0);
/// The agent cursor's travel time grows with distance, between these bounds.
pub(crate) const GLIDE_MIN: Duration = Duration::from_millis(90);
pub(crate) const GLIDE_MAX: Duration = Duration::from_millis(320);
pub(crate) const RIPPLE: Duration = Duration::from_millis(450);
pub(crate) const FADE: Duration = Duration::from_millis(250);
/// The outline leaves this long after the last action. The agent cursor rests where it is, and it,
/// the glow and the pill stay for the whole turn: between calls the pill says "Thinking…", so
/// nothing flickers while the model decides. `IDLE` with no call at all is only a safety net for
/// clients that never end a turn.
pub(crate) const OUTLINE_IDLE: Duration = Duration::from_secs(4);
pub(crate) const IDLE: Duration = Duration::from_secs(45);
/// How long after a call ends the pill switches to "Thinking…".
pub(crate) const THINKING_AFTER: Duration = Duration::from_millis(900);
pub(crate) const STOPPED_LABEL: &str =
    "Stopped. Cryzo won't act again until you send a new message.";

/// Where the arrow's tip sits inside the pointer image, in unscaled pixels; the room around it
/// holds the click ripple.
pub(crate) const TIP: f32 = 30.0;
/// The pointer image's side, in unscaled pixels.
pub(crate) const POINTER: f32 = 96.0;
/// The arrow, tip at the origin, in unscaled pixels.
pub(crate) const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 19.0),
    (4.6, 15.0),
    (7.6, 22.0),
    (10.6, 20.7),
    (7.7, 14.0),
    (13.6, 14.0),
];

/// One change for the overlay. macOS sends these to its overlay process as JSON lines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "signal", rename_all = "snake_case")]
pub(crate) enum Signal {
    /// Shows the glow and the pill with this step, without moving the agent cursor.
    Step { step: String },
    /// Glides the agent cursor to the middle of `rect` and outlines it.
    Point {
        step: String,
        rect: Rect,
        glide_ms: u64,
    },
    /// A ripple under the agent cursor.
    Click,
    /// A tool call started or ended. While one runs the overlay never fades, however long it takes.
    Busy { busy: bool },
    /// The user stopped Cryzo: the pill says so until the next turn.
    Stopped,
    /// The agent's turn ended: hide everything now.
    Done,
    /// A new turn after a Stop: the pill acts normally again.
    Resume,
}

/// Travel time for a glide of `distance` pixels: short hops are quick, long ones stay readable.
pub(crate) fn glide_time(distance: f32) -> Duration {
    let span = (GLIDE_MAX - GLIDE_MIN).as_millis() as f32;
    GLIDE_MIN + Duration::from_millis((span * (distance / 1200.0).clamp(0.0, 1.0).sqrt()) as u64)
}

/// A handle that platforms without their own overlay type give the desktop: signals go to
/// whatever thread or process draws them.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Clone)]
pub struct Indicator {
    sender: Sender<Signal>,
    /// Where the agent cursor was last sent, to time the next glide by its distance.
    cursor: Arc<Mutex<Option<(f32, f32)>>>,
}

#[cfg_attr(windows, allow(dead_code))]
impl Indicator {
    pub(crate) fn new(sender: Sender<Signal>) -> Self {
        Self {
            sender,
            cursor: Default::default(),
        }
    }

    pub(crate) fn step(&self, step: impl Into<String>) {
        self.send(Signal::Step { step: step.into() });
    }

    /// Starts the cursor glide and outline, and returns when the cursor arrives.
    pub(crate) fn preview(&self, step: impl Into<String>, rect: Rect) -> Instant {
        let target = centre(rect);
        // A first glide flies in from about 100 px away.
        let previous = self
            .cursor
            .lock()
            .map(|mut cursor| cursor.replace(target))
            .unwrap_or_default();
        let glide = glide_time(previous.map_or(100.0, |(x, y)| {
            ((x - target.0).powi(2) + (y - target.1).powi(2)).sqrt()
        }));
        self.send(Signal::Point {
            step: step.into(),
            rect,
            glide_ms: glide.as_millis() as u64,
        });
        Instant::now() + glide
    }

    /// Shows the target before delivering coordinate-based input.
    pub(crate) fn point(&self, step: impl Into<String>, rect: Rect) {
        let arrives = self.preview(step, rect);
        std::thread::sleep(arrives.saturating_duration_since(Instant::now()));
    }

    pub(crate) fn click(&self) {
        self.send(Signal::Click);
    }

    pub(crate) fn busy(&self) -> Busy {
        self.send(Signal::Busy { busy: true });
        Busy(self.clone())
    }

    pub(crate) fn done(&self) {
        if let Ok(mut cursor) = self.cursor.lock() {
            *cursor = None;
        }
        self.send(Signal::Done);
    }

    pub(crate) fn resume(&self) {
        self.send(Signal::Resume);
    }

    fn send(&self, signal: Signal) {
        let _ = self.sender.send(signal);
    }
}

/// Marks a tool call as running until dropped.
#[cfg_attr(windows, allow(dead_code))]
pub struct Busy(Indicator);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.send(Signal::Busy { busy: false });
    }
}

fn centre(rect: Rect) -> (f32, f32) {
    (
        (rect.left + rect.right) as f32 / 2.0,
        (rect.top + rect.bottom) as f32 / 2.0,
    )
}

/// What the overlay shows at one moment.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    /// The glow's opacity, 0 to 1.
    pub glow: f32,
    pub label: String,
    pub stopped: bool,
    /// Where the agent cursor's tip is, when it is on screen.
    pub pointer: Option<(f32, f32)>,
    /// How far the click ripple has spread, 0 to 1.
    pub ripple: Option<f32>,
    pub outline: Option<Rect>,
}

/// The overlay's behaviour over time, fed signals and asked for a frame about 60 times a second.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) struct Animator {
    shown: Option<Instant>,
    last: Instant,
    last_point: Instant,
    busy: bool,
    thinking: bool,
    stopped: bool,
    label: String,
    pointing: bool,
    from: (f32, f32),
    to: (f32, f32),
    glide: Instant,
    glide_time: Duration,
    ripple: Option<Instant>,
    outline: Option<Rect>,
}

#[cfg_attr(windows, allow(dead_code))]
impl Animator {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            shown: None,
            last: now,
            last_point: now,
            busy: false,
            thinking: false,
            stopped: false,
            label: String::new(),
            pointing: false,
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            glide: now,
            glide_time: GLIDE_MIN,
            ripple: None,
            outline: None,
        }
    }

    /// Where the pill belongs: near the cursor's target once there is one.
    pub(crate) fn anchor(&self) -> Option<(f32, f32)> {
        (self.to != (0.0, 0.0)).then_some(self.to)
    }

    pub(crate) fn signal(&mut self, signal: Signal, now: Instant) {
        match signal {
            Signal::Done => {
                self.hide();
                return;
            }
            Signal::Resume => {
                self.stopped = false;
                return;
            }
            Signal::Stopped => {
                self.stopped = true;
                self.label = STOPPED_LABEL.into();
                return;
            }
            Signal::Busy { busy } => {
                self.last = now;
                self.busy = busy;
                return;
            }
            _ => {}
        }
        self.last = now;
        self.thinking = false;
        if self.shown.is_none() {
            self.shown = Some(now);
        }
        match signal {
            Signal::Step { step } if !self.stopped => self.label = step,
            Signal::Point {
                step,
                rect,
                glide_ms,
            } => {
                if !self.stopped {
                    self.label = step;
                }
                let target = centre(rect);
                // A fresh cursor flies in from up and to the left; a visible one continues from
                // wherever it is.
                self.from = if self.pointing {
                    self.position(now)
                } else {
                    (target.0 - 70.0, target.1 - 70.0)
                };
                self.to = target;
                self.glide = now;
                self.glide_time = Duration::from_millis(glide_ms);
                self.ripple = None;
                self.last_point = now;
                self.pointing = true;
                self.outline = Some(rect);
            }
            Signal::Click => {
                self.ripple = Some(now.max(self.glide + self.glide_time));
                self.last_point = now;
            }
            _ => {}
        }
    }

    /// The overlay now, or None when it is hidden.
    pub(crate) fn frame(&mut self, now: Instant) -> Option<Frame> {
        let shown = self.shown?;
        if !self.busy
            && !self.thinking
            && !self.stopped
            && now.duration_since(self.last) >= THINKING_AFTER
        {
            self.thinking = true;
            self.label = "Thinking…".into();
        }
        let quiet = if self.busy {
            Duration::ZERO
        } else {
            now.duration_since(self.last)
        };
        let fade_out = quiet
            .checked_sub(IDLE)
            .map(|late| 1.0 - late.as_secs_f32() / FADE.as_secs_f32());
        if fade_out.is_some_and(|alpha| alpha <= 0.0) {
            self.hide();
            return None;
        }
        let fade_in = (now.duration_since(shown).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        let breathing = 0.8 + 0.2 * (now.duration_since(shown).as_secs_f32() * 3.0).sin();
        if self.outline.is_some() && now.duration_since(self.last_point) > OUTLINE_IDLE {
            self.outline = None;
        }
        let ripple = self
            .ripple
            .and_then(|start| now.checked_duration_since(start))
            .map(|elapsed| elapsed.as_secs_f32() / RIPPLE.as_secs_f32())
            .filter(|progress| *progress < 1.0);
        if self.ripple.is_some_and(|start| now >= start + RIPPLE) {
            self.ripple = None;
        }
        Some(Frame {
            glow: fade_in * breathing * fade_out.unwrap_or(1.0),
            label: self.label.clone(),
            stopped: self.stopped,
            pointer: self.pointing.then(|| self.position(now)),
            ripple,
            outline: self.outline,
        })
    }

    fn hide(&mut self) {
        self.shown = None;
        self.pointing = false;
        self.busy = false;
        self.thinking = false;
        self.outline = None;
        self.to = (0.0, 0.0);
    }

    /// Where the agent cursor is now, easing out along a slightly bowed path, as a hand moves.
    fn position(&self, now: Instant) -> (f32, f32) {
        glide_position(
            self.from,
            self.to,
            now.duration_since(self.glide).as_secs_f32() / self.glide_time.as_secs_f32(),
        )
    }
}

/// A point `t` (0 to 1) of the way along a glide, eased out and bowed slightly to one side.
pub(crate) fn glide_position(from: (f32, f32), to: (f32, f32), t: f32) -> (f32, f32) {
    let eased = 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let bow = 0.32 * eased * (1.0 - eased);
    (
        from.0 + dx * eased - dy * bow,
        from.1 + dy * eased + dx * bow,
    )
}

/// Paints the agent cursor into a square `size` × `size` image, tip at (`TIP`, `TIP`) scaled, with
/// the click ripple when given.
pub(crate) fn paint_pointer(pixels: &mut [u32], size: i32, scale: f32, ripple: Option<f32>) {
    let tip = TIP * scale;
    let arrow = ARROW
        .iter()
        .map(|(x, y)| (tip + x * scale, tip + y * scale))
        .collect::<Vec<_>>();
    let shadow = arrow
        .iter()
        .map(|(x, y)| (x + 1.2 * scale, y + 2.0 * scale))
        .collect::<Vec<_>>();
    pixels.fill(0);
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let pixel = &mut pixels[(y * size + x) as usize];
            if let Some(t) = ripple {
                let distance = ((p.0 - tip).powi(2) + (p.1 - tip).powi(2)).sqrt();
                let radius = (6.0 + 22.0 * t) * scale;
                let ring = (1.5 * scale + 0.5 - (distance - radius).abs()).clamp(0.0, 1.0);
                over(pixel, ACCENT, ring * (1.0 - t) * 0.9);
            }
            let shade = polygon(p, &shadow);
            over(
                pixel,
                (0.0, 0.0, 0.0),
                0.3 * ((2.5 * scale - shade) / (2.5 * scale)).clamp(0.0, 1.0),
            );
            let d = polygon(p, &arrow);
            over(
                pixel,
                (255.0, 255.0, 255.0),
                (1.6 * scale + 0.5 - d).clamp(0.0, 1.0),
            );
            over(pixel, ACCENT, (0.5 - d).clamp(0.0, 1.0));
        }
    }
}

/// The room an outline image keeps around its element, in physical pixels.
pub(crate) fn outline_pad(scale: f32) -> i32 {
    (16.0 * scale) as i32
}

/// Paints a soft rounded outline that fills a `width` × `height` image, the element sitting
/// `outline_pad` inside its edges. None when the element is empty or too large to be worth it.
pub(crate) fn paint_outline(width: i32, height: i32, scale: f32) -> Option<Vec<u32>> {
    let pad = outline_pad(scale);
    if width <= pad * 2 || height <= pad * 2 || width * height > 16_000_000 {
        return None;
    }
    let margin = 6.0 * scale;
    let mut pixels = vec![0u32; (width * height) as usize];
    let centre = (width as f32 / 2.0, height as f32 / 2.0);
    let half = (
        (width - pad * 2) as f32 / 2.0 + margin,
        (height - pad * 2) as f32 / 2.0 + margin,
    );
    let radius = (8.0 * scale).min(half.0).min(half.1);
    let stroke = 2.0 * scale;
    for y in 0..height {
        for x in 0..width {
            let p = (x as f32 + 0.5 - centre.0, y as f32 + 0.5 - centre.1);
            let d = rounded_rect(p, half, radius);
            let pixel = &mut pixels[(y * width + x) as usize];
            if d > 0.0 {
                over(pixel, ACCENT, 0.25 * (-d / (4.0 * scale)).exp());
            } else {
                over(pixel, ACCENT, 0.06);
            }
            over(
                pixel,
                ACCENT,
                (stroke / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0),
            );
        }
    }
    Some(pixels)
}

/// Paints the glow band inside each monitor, given in the image's own coordinates.
pub(crate) fn paint_glow(
    pixels: &mut [u32],
    width: i32,
    height: i32,
    monitors: &[Rect],
    scale: f32,
) {
    let band = (12.0 * scale).max(4.0) as i32;
    for monitor in monitors {
        let (left, top, right, bottom) = (monitor.left, monitor.top, monitor.right, monitor.bottom);
        for y in top.max(0)..bottom.min(height) {
            let edge_row = y - top < band || bottom - 1 - y < band;
            // Edge rows run the full width; other rows only touch the left and right bands.
            let columns = if edge_row {
                [left..right, 0..0]
            } else {
                [left..left + band, right - band..right]
            };
            for x in columns.into_iter().flatten() {
                if x < 0 || x >= width {
                    continue;
                }
                let edge = (x - left)
                    .min(right - 1 - x)
                    .min(y - top)
                    .min(bottom - 1 - y);
                let strength = (band - edge) as f32 / band as f32;
                over(
                    &mut pixels[(y * width + x) as usize],
                    ACCENT,
                    strength * strength * 0.85,
                );
            }
        }
    }
}

/// The four edge strips of a monitor's glow (top, bottom, left, right), so a platform can show
/// the glow as small windows instead of one transparent window over the whole screen.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn glow_strips(monitor: Rect, scale: f32) -> [Rect; 4] {
    let band = (12.0 * scale).max(4.0) as i32;
    let Rect {
        left,
        top,
        right,
        bottom,
    } = monitor;
    [
        Rect {
            left,
            top,
            right,
            bottom: top + band,
        },
        Rect {
            left,
            top: bottom - band,
            right,
            bottom,
        },
        Rect {
            left,
            top: top + band,
            right: left + band,
            bottom: bottom - band,
        },
        Rect {
            left: right - band,
            top: top + band,
            right,
            bottom: bottom - band,
        },
    ]
}

/// One glow strip's pixels: the monitor's glow as seen through that strip.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn paint_glow_strip(strip: Rect, monitor: Rect, scale: f32) -> Vec<u32> {
    let (width, height) = (strip.right - strip.left, strip.bottom - strip.top);
    let mut pixels = vec![0u32; (width.max(0) * height.max(0)) as usize];
    let relative = Rect {
        left: monitor.left - strip.left,
        top: monitor.top - strip.top,
        right: monitor.right - strip.left,
        bottom: monitor.bottom - strip.top,
    };
    paint_glow(&mut pixels, width, height, &[relative], scale);
    pixels
}

/// Paints `color` at opacity `alpha` over a premultiplied ARGB pixel.
pub(crate) fn over(pixel: &mut u32, color: (f32, f32, f32), alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let alpha = alpha.min(1.0);
    let old = *pixel;
    let mix = |value: f32, shift: u32| {
        let under = ((old >> shift) & 0xff) as f32;
        (value * alpha + under * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u32
    };
    *pixel = mix(255.0, 24) << 24 | mix(color.0, 16) << 16 | mix(color.1, 8) << 8 | mix(color.2, 0);
}

/// Signed distance from `p` to a rounded rectangle centred on the origin: negative inside.
pub(crate) fn rounded_rect(p: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let q = (p.0.abs() - (half.0 - radius), p.1.abs() - (half.1 - radius));
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - radius
}

/// Signed distance from `p` to a polygon: negative inside.
pub(crate) fn polygon(p: (f32, f32), points: &[(f32, f32)]) -> f32 {
    let mut distance = f32::MAX;
    let mut inside = false;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        let edge = (b.0 - a.0, b.1 - a.1);
        let to_p = (p.0 - a.0, p.1 - a.1);
        let along = ((to_p.0 * edge.0 + to_p.1 * edge.1) / (edge.0 * edge.0 + edge.1 * edge.1))
            .clamp(0.0, 1.0);
        let gap = (to_p.0 - edge.0 * along, to_p.1 - edge.1 * along);
        distance = distance.min((gap.0 * gap.0 + gap.1 * gap.1).sqrt());
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
    }
    if inside { -distance } else { distance }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_measure_inside_as_negative() {
        assert!(polygon((2.0, 8.0), &ARROW) < 0.0);
        assert!(polygon((12.0, 2.0), &ARROW) > 0.0);
        assert!(rounded_rect((0.0, 0.0), (10.0, 5.0), 2.0) < 0.0);
        assert!((rounded_rect((12.0, 0.0), (10.0, 5.0), 2.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn painting_premultiplies() {
        let mut pixel = 0;
        over(&mut pixel, (200.0, 100.0, 0.0), 0.5);
        assert_eq!(pixel, 128 << 24 | 100 << 16 | 50 << 8);
    }

    #[test]
    fn longer_trips_glide_longer() {
        assert_eq!(glide_time(0.0), GLIDE_MIN);
        assert!(glide_time(100.0) < glide_time(800.0));
        assert_eq!(glide_time(5000.0), GLIDE_MAX);
    }

    #[test]
    fn signals_round_trip_as_json() {
        let signal = Signal::Point {
            step: "Clicking Send".into(),
            rect: Rect {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            },
            glide_ms: 120,
        };
        let line = serde_json::to_string(&signal).unwrap();
        assert_eq!(serde_json::from_str::<Signal>(&line).unwrap(), signal);
    }

    #[test]
    fn the_overlay_stays_through_long_calls_and_thinks_between_them() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut animator = Animator::new(start);
        assert_eq!(animator.frame(start), None);
        animator.signal(Signal::Busy { busy: true }, at(0));
        animator.signal(
            Signal::Step {
                step: "Opening Cursor".into(),
            },
            at(0),
        );
        // A call far longer than the idle timeout keeps everything up and labelled.
        let during = animator.frame(at(120_000)).unwrap();
        assert_eq!(during.label, "Opening Cursor");
        assert!(during.glow > 0.0);
        animator.signal(Signal::Busy { busy: false }, at(120_000));
        assert_eq!(animator.frame(at(120_500)).unwrap().label, "Opening Cursor");
        assert_eq!(animator.frame(at(121_000)).unwrap().label, "Thinking…");
        // A long think still fades out eventually, for clients that never end the turn.
        assert_eq!(animator.frame(at(120_000 + 46_000)), None);
    }

    #[test]
    fn pointing_glides_and_a_stop_sticks_until_resume() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut animator = Animator::new(start);
        let rect = Rect {
            left: 100,
            top: 100,
            right: 200,
            bottom: 140,
        };
        animator.signal(
            Signal::Point {
                step: "Clicking Send".into(),
                rect,
                glide_ms: 200,
            },
            at(0),
        );
        let start_frame = animator.frame(at(0)).unwrap();
        assert_eq!(start_frame.pointer, Some((80.0, 50.0)));
        assert_eq!(start_frame.outline, Some(rect));
        assert_eq!(
            animator.frame(at(250)).unwrap().pointer,
            Some((150.0, 120.0))
        );
        animator.signal(Signal::Stopped, at(300));
        animator.signal(
            Signal::Step {
                step: "Typing".into(),
            },
            at(310),
        );
        let stopped = animator.frame(at(2_000)).unwrap();
        assert!(stopped.stopped);
        assert_eq!(stopped.label, STOPPED_LABEL);
        animator.signal(Signal::Done, at(2_100));
        assert_eq!(animator.frame(at(2_200)), None);
        animator.signal(Signal::Resume, at(3_000));
        animator.signal(
            Signal::Step {
                step: "Typing".into(),
            },
            at(3_000),
        );
        assert_eq!(animator.frame(at(3_100)).unwrap().label, "Typing");
    }
}
