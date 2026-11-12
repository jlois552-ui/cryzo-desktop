//! The on-screen sign that Cryzo is using the computer: a glow around every monitor, a pill that
//! names the current step with a Stop button, and an agent cursor that glides to each target and
//! outlines it. Cryzo acts through UI Automation without moving the real mouse, so without this
//! the user could not see what it is doing. Every window is hidden from screen capture, so the
//! agent's own screenshots and OCR never see it, and all but the pill let clicks through.

use std::{
    cell::RefCell,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::{
            Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
            Gdi::{
                AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
                BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
                CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DIB_RGB_COLORS,
                DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC,
                DeleteObject, DrawTextW, Ellipse, EndPaint, EnumDisplayMonitors, FillRect, GetDC,
                GetStockObject, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, HMONITOR,
                InvalidateRect, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC, RoundRect,
                SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForSystem,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
                GetSystemMetrics, HWND_TOPMOST, IDC_ARROW, IDC_HAND, KillTimer, LoadCursorW,
                MA_NOACTIVATE, MSG, PostMessageW, RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN,
                SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE,
                SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer,
                SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA,
                UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WINDOW_EX_STYLE, WM_APP,
                WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_PAINT, WM_TIMER, WNDCLASSW,
                WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
    core::{BOOL, PCWSTR, w},
};

/// Cryzo violet.
const ACCENT: (f32, f32, f32) = (124.0, 92.0, 255.0);
const GLIDE: Duration = Duration::from_millis(220);
const RIPPLE: Duration = Duration::from_millis(450);
const FADE: Duration = Duration::from_millis(250);
/// The outline leaves this long after the last action. The agent cursor rests where it is, and it,
/// the glow and the pill stay until `IDLE` passes with no call at all, so nothing flickers while
/// the model thinks.
const OUTLINE_IDLE: Duration = Duration::from_secs(4);
const IDLE: Duration = Duration::from_secs(10);
const FRAME_MS: u32 = 16;

enum Signal {
    Step(String),
    Point { step: String, rect: RECT },
    Click,
}

/// A handle to the overlay thread. Cloning it is cheap; every clone drives the same windows.
#[derive(Clone)]
pub(crate) struct Overlay {
    sender: Sender<Signal>,
    /// The glow window, which also receives the wake-up messages.
    window: isize,
}

impl Overlay {
    /// Starts the overlay thread with its windows hidden. None when windows could not be made,
    /// which leaves desktop control working without the overlay.
    pub(crate) fn start() -> Option<Self> {
        let (sender, signals) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        std::thread::Builder::new()
            .name("cryzo-overlay".into())
            .spawn(move || match Windows::create() {
                Ok(windows) => {
                    let _ = ready.send(Some(windows.glow.0 as isize));
                    run(windows, signals);
                }
                Err(error) => {
                    eprintln!("cryzo-desktop: could not create the on-screen overlay: {error}");
                    let _ = ready.send(None);
                }
            })
            .ok()?;
        let window = started
            .recv_timeout(Duration::from_secs(3))
            .ok()
            .flatten()?;
        Some(Self { sender, window })
    }

    /// Shows the glow and the pill with this step, without moving the agent cursor.
    pub(crate) fn step(&self, step: impl Into<String>) {
        self.send(Signal::Step(step.into()));
    }

    /// Glides the agent cursor to the middle of `rect` and outlines it, then waits for the glide,
    /// so the user sees the target before the action lands.
    pub(crate) fn point(&self, step: impl Into<String>, rect: RECT) {
        self.send(Signal::Point {
            step: step.into(),
            rect,
        });
        std::thread::sleep(GLIDE);
    }

    /// A ripple under the agent cursor.
    pub(crate) fn click(&self) {
        self.send(Signal::Click);
    }

    fn send(&self, signal: Signal) {
        if self.sender.send(signal).is_ok() {
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(self.window as *mut _)),
                    WM_APP,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
    }
}

struct Windows {
    glow: HWND,
    outline: HWND,
    pointer: HWND,
    pill: HWND,
    scale: f32,
}

impl Windows {
    fn create() -> windows::core::Result<Self> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
        let scale = unsafe { GetDpiForSystem() } as f32 / 96.0;
        let passive =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let windows = Self {
            glow: window(instance, w!("CryzoOverlayGlow"), passive, false)?,
            outline: window(instance, w!("CryzoOverlayOutline"), passive, false)?,
            pointer: window(instance, w!("CryzoOverlayPointer"), passive, false)?,
            pill: window(
                instance,
                w!("CryzoOverlayPill"),
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                true,
            )?,
            scale,
        };
        // CRYZO_OVERLAY_CAPTURE=1 keeps the overlay in screen captures, for demo recordings.
        if std::env::var_os("CRYZO_OVERLAY_CAPTURE").is_none() {
            for hwnd in windows.all() {
                let _ = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
            }
        }
        // Windows 11 rounds the pill's corners with antialiasing; older systems keep it square.
        let _ = unsafe {
            DwmSetWindowAttribute(
                windows.pill,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_ROUND as *const _ as *const _,
                size_of_val(&DWMWCP_ROUND) as u32,
            )
        };
        let font = unsafe {
            CreateFontW(
                (-14.0 * scale) as i32,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        PILL.with_borrow_mut(|pill| {
            pill.font = font;
            pill.scale = scale;
        });
        Ok(windows)
    }

    fn all(&self) -> [HWND; 4] {
        [self.glow, self.outline, self.pointer, self.pill]
    }
}

fn window(
    instance: HINSTANCE,
    class: PCWSTR,
    style: WINDOW_EX_STYLE,
    pill: bool,
) -> windows::core::Result<HWND> {
    let cursor = unsafe { LoadCursorW(None, if pill { IDC_HAND } else { IDC_ARROW })? };
    let info = WNDCLASSW {
        lpfnWndProc: Some(if pill { pill_proc } else { plain_proc }),
        hInstance: instance,
        lpszClassName: class,
        hCursor: cursor,
        ..Default::default()
    };
    // Fails harmlessly when a previous overlay in this process registered the class.
    unsafe { RegisterClassW(&info) };
    unsafe {
        CreateWindowExW(
            style,
            class,
            class,
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )
    }
}

unsafe extern "system" fn plain_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// What the pill shows. It lives on the overlay thread, where the pill's window procedure runs.
struct Pill {
    text: Vec<u16>,
    stopped: bool,
    font: HFONT,
    scale: f32,
}

thread_local! {
    static PILL: RefCell<Pill> = RefCell::new(Pill {
        text: Vec::new(),
        stopped: false,
        font: HFONT::default(),
        scale: 1.0,
    });
}

/// Pill layout in unscaled pixels: height, side padding, status dot, gaps and the Stop button.
const PILL_HEIGHT: f32 = 40.0;
const PILL_PAD: f32 = 14.0;
const PILL_DOT: f32 = 8.0;
const PILL_GAP: f32 = 10.0;
const STOP_WIDTH: f32 = 56.0;
const STOP_HEIGHT: f32 = 26.0;

unsafe extern "system" fn pill_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Clicking Stop must not take the keyboard focus away from the app being driven.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            PILL.with_borrow(|pill| paint_pill(hwnd, pill));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let mut client = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            let stop = PILL.with_borrow(|pill| {
                !pill.stopped && x >= client.right - ((PILL_PAD + STOP_WIDTH) * pill.scale) as i32
            });
            // The next frame notices the stop and relabels the pill, as for Ctrl+Alt+Esc.
            if stop {
                crate::uia::stop();
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint_pill(hwnd: HWND, pill: &Pill) {
    let scaled = |value: f32| (value * pill.scale) as i32;
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let middle = client.bottom / 2;
    unsafe {
        let background = CreateSolidBrush(rgb(32, 31, 36));
        FillRect(dc, &client, background);
        let _ = DeleteObject(background.into());
        SelectObject(dc, GetStockObject(NULL_PEN));
        let dot = CreateSolidBrush(if pill.stopped {
            rgb(230, 80, 80)
        } else {
            rgb(ACCENT.0 as u8, ACCENT.1 as u8, ACCENT.2 as u8)
        });
        let previous = SelectObject(dc, dot.into());
        let left = scaled(PILL_PAD);
        let _ = Ellipse(
            dc,
            left,
            middle - scaled(PILL_DOT / 2.0),
            left + scaled(PILL_DOT),
            middle + scaled(PILL_DOT / 2.0),
        );
        SelectObject(dc, previous);
        let _ = DeleteObject(dot.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(240, 240, 245));
        SelectObject(dc, pill.font.into());
        let stop_left = client.right - scaled(PILL_PAD + STOP_WIDTH);
        let mut text_box = RECT {
            left: left + scaled(PILL_DOT + PILL_GAP),
            top: 0,
            right: if pill.stopped {
                client.right - scaled(PILL_PAD)
            } else {
                stop_left - scaled(PILL_GAP)
            },
            bottom: client.bottom,
        };
        let mut text = pill.text.clone();
        DrawTextW(
            dc,
            &mut text,
            &mut text_box,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        if !pill.stopped {
            let button = CreateSolidBrush(rgb(64, 62, 72));
            let previous = SelectObject(dc, button.into());
            let top = middle - scaled(STOP_HEIGHT / 2.0);
            let _ = RoundRect(
                dc,
                stop_left,
                top,
                stop_left + scaled(STOP_WIDTH),
                top + scaled(STOP_HEIGHT),
                scaled(12.0),
                scaled(12.0),
            );
            SelectObject(dc, previous);
            let _ = DeleteObject(button.into());
            let mut label = "Stop".encode_utf16().collect::<Vec<_>>();
            let mut label_box = RECT {
                left: stop_left,
                top,
                right: stop_left + scaled(STOP_WIDTH),
                bottom: top + scaled(STOP_HEIGHT),
            };
            DrawTextW(
                dc,
                &mut label,
                &mut label_box,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
            );
        }
        let _ = EndPaint(hwnd, &paint);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn run(windows: Windows, signals: Receiver<Signal>) {
    let mut state = State::new(windows);
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        if matches!(message.message, WM_APP | WM_TIMER) && message.hwnd == state.windows.glow {
            for signal in signals.try_iter() {
                state.signal(signal);
            }
            state.frame();
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

struct State {
    windows: Windows,
    glow: Option<Layer>,
    pointer: Option<Layer>,
    outline: Option<Layer>,
    shown: Option<Instant>,
    last: Instant,
    last_point: Instant,
    pointing: bool,
    from: (f32, f32),
    to: (f32, f32),
    glide: Instant,
    ripple: Option<Instant>,
    /// The pointer image needs drawing again (a ripple ran or just ended).
    redraw: bool,
}

impl State {
    fn new(windows: Windows) -> Self {
        let size = (96.0 * windows.scale) as i32;
        let pointer = Layer::new(windows.pointer, size, size);
        let now = Instant::now();
        let mut state = Self {
            windows,
            glow: None,
            pointer,
            outline: None,
            shown: None,
            last: now,
            last_point: now,
            pointing: false,
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            glide: now,
            ripple: None,
            redraw: true,
        };
        state.draw_pointer(None);
        state
    }

    fn signal(&mut self, signal: Signal) {
        let now = Instant::now();
        self.last = now;
        if self.shown.is_none() {
            self.show();
        }
        // Once stopped, the pill keeps saying so; reads may still arrive.
        let stopped = PILL.with_borrow(|pill| pill.stopped);
        match signal {
            Signal::Step(step) if !stopped => self.label(&step),
            Signal::Step(_) => {}
            Signal::Point { step, rect } => {
                if !stopped {
                    self.label(&step);
                }
                let target = (
                    (rect.left + rect.right) as f32 / 2.0,
                    (rect.top + rect.bottom) as f32 / 2.0,
                );
                // A fresh cursor flies in from up and to the left; a visible one continues
                // from wherever it is.
                self.from = if self.pointing {
                    self.position(now)
                } else {
                    let offset = 70.0 * self.windows.scale;
                    (target.0 - offset, target.1 - offset)
                };
                self.to = target;
                self.glide = now;
                self.last_point = now;
                self.pointing = true;
                self.draw_outline(rect);
                unsafe {
                    let _ = ShowWindow(self.windows.pointer, SW_SHOWNOACTIVATE);
                }
            }
            Signal::Click => {
                self.ripple = Some(now);
                self.last_point = now;
            }
        }
    }

    fn show(&mut self) {
        self.shown = Some(Instant::now());
        // Drawn on every show, so a monitor added or rearranged since is covered.
        self.glow = draw_glow(self.windows.glow, self.windows.scale);
        if let Some(glow) = &self.glow {
            glow.push(glow.origin.0, glow.origin.1, 0);
        }
        unsafe {
            let _ = ShowWindow(self.windows.glow, SW_SHOWNOACTIVATE);
            SetTimer(Some(self.windows.glow), 1, FRAME_MS, None);
        }
    }

    fn hide(&mut self) {
        self.shown = None;
        self.pointing = false;
        self.glow = None;
        self.outline = None;
        unsafe {
            let _ = KillTimer(Some(self.windows.glow), 1);
            for hwnd in self.windows.all() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    fn frame(&mut self) {
        let Some(shown) = self.shown else {
            return;
        };
        let now = Instant::now();
        if crate::uia::is_stopped() && !PILL.with_borrow(|pill| pill.stopped) {
            PILL.with_borrow_mut(|pill| pill.stopped = true);
            self.label("Stopped. Cryzo won't act until desktop control restarts.");
        }
        let quiet = now.duration_since(self.last);
        let fade_out = quiet
            .checked_sub(IDLE)
            .map(|late| 1.0 - late.as_secs_f32() / FADE.as_secs_f32());
        if fade_out.is_some_and(|alpha| alpha <= 0.0) {
            self.hide();
            return;
        }
        let fade_in = (now.duration_since(shown).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        let breathing = 0.8 + 0.2 * (now.duration_since(shown).as_secs_f32() * 3.0).sin();
        let alpha = fade_in * breathing * fade_out.unwrap_or(1.0);
        if let Some(glow) = &self.glow {
            glow.fade((alpha * 255.0) as u8);
        }
        if self.outline.is_some() && now.duration_since(self.last_point) > OUTLINE_IDLE {
            self.outline = None;
            let _ = unsafe { ShowWindow(self.windows.outline, SW_HIDE) };
        }
        if !self.pointing {
            return;
        }
        let ripple = self
            .ripple
            .map(|start| now.duration_since(start).as_secs_f32() / RIPPLE.as_secs_f32())
            .filter(|progress| *progress < 1.0);
        if ripple.is_none() {
            self.ripple = None;
        }
        if ripple.is_some() || self.redraw {
            self.draw_pointer(ripple);
            self.redraw = ripple.is_some();
        }
        let (x, y) = self.position(now);
        let tip = TIP * self.windows.scale;
        if let Some(pointer) = &self.pointer {
            pointer.push((x - tip) as i32, (y - tip) as i32, 255);
        }
    }

    /// Where the agent cursor is now, easing out along its glide.
    fn position(&self, now: Instant) -> (f32, f32) {
        let t = (now.duration_since(self.glide).as_secs_f32() / GLIDE.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        (
            self.from.0 + (self.to.0 - self.from.0) * eased,
            self.from.1 + (self.to.1 - self.from.1) * eased,
        )
    }

    fn label(&mut self, step: &str) {
        let text = step.encode_utf16().collect::<Vec<_>>();
        let scale = self.windows.scale;
        let (width, stopped) = PILL.with_borrow_mut(|pill| {
            pill.text = text.clone();
            let dc = unsafe { GetDC(Some(self.windows.pill)) };
            let previous = unsafe { SelectObject(dc, pill.font.into()) };
            let mut size = SIZE::default();
            let _ = unsafe { GetTextExtentPoint32W(dc, &text, &mut size) };
            unsafe {
                SelectObject(dc, previous);
                ReleaseDC(Some(self.windows.pill), dc);
            }
            (size.cx.min((520.0 * scale) as i32), pill.stopped)
        });
        let button = if stopped { 0.0 } else { PILL_GAP + STOP_WIDTH };
        let chrome = ((PILL_PAD * 2.0 + PILL_DOT + PILL_GAP + button) * scale) as i32;
        let height = (PILL_HEIGHT * scale) as i32;
        let total = width + chrome;
        let screen = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        unsafe {
            let _ = SetWindowPos(
                self.windows.pill,
                Some(HWND_TOPMOST),
                (screen - total) / 2,
                (12.0 * scale) as i32,
                total,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(self.windows.pill), None, false);
        }
    }

    fn draw_outline(&mut self, rect: RECT) {
        let scale = self.windows.scale;
        let margin = 6.0 * scale;
        let glow = 10.0 * scale;
        let pad = (margin + glow) as i32;
        let (left, top) = (rect.left - pad, rect.top - pad);
        let width = rect.right - rect.left + pad * 2;
        let height = rect.bottom - rect.top + pad * 2;
        if rect.right <= rect.left || rect.bottom <= rect.top || width * height > 16_000_000 {
            self.outline = None;
            let _ = unsafe { ShowWindow(self.windows.outline, SW_HIDE) };
            return;
        }
        let Some(mut layer) = Layer::new(self.windows.outline, width, height) else {
            return;
        };
        let centre = (width as f32 / 2.0, height as f32 / 2.0);
        let half = (
            (rect.right - rect.left) as f32 / 2.0 + margin,
            (rect.bottom - rect.top) as f32 / 2.0 + margin,
        );
        let radius = (8.0 * scale).min(half.0).min(half.1);
        let stroke = 2.0 * scale;
        let pixels = layer.pixels();
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
        layer.push(left, top, 255);
        let _ = unsafe { ShowWindow(self.windows.outline, SW_SHOWNOACTIVATE) };
        self.outline = Some(layer);
    }

    fn draw_pointer(&mut self, ripple: Option<f32>) {
        let scale = self.windows.scale;
        let Some(layer) = &mut self.pointer else {
            return;
        };
        let size = layer.width;
        let tip = TIP * scale;
        let arrow = ARROW
            .iter()
            .map(|(x, y)| (tip + x * scale, tip + y * scale))
            .collect::<Vec<_>>();
        let shadow = arrow
            .iter()
            .map(|(x, y)| (x + 1.2 * scale, y + 2.0 * scale))
            .collect::<Vec<_>>();
        let pixels = layer.pixels();
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
}

/// Where the arrow's tip sits inside the pointer window, in unscaled pixels; the room around it
/// holds the click ripple.
const TIP: f32 = 30.0;
/// The arrow, tip at the origin, in unscaled pixels.
const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 19.0),
    (4.6, 15.0),
    (7.6, 22.0),
    (10.6, 20.7),
    (7.7, 14.0),
    (13.6, 14.0),
];

/// The glow around every monitor, drawn into one window that spans the virtual screen.
fn draw_glow(hwnd: HWND, scale: f32) -> Option<Layer> {
    let origin = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        )
    };
    let (width, height) = unsafe {
        (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let mut layer = Layer::new(hwnd, width, height)?;
    layer.origin = origin;
    let mut monitors = Vec::<RECT>::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    let band = (12.0 * scale).max(4.0) as i32;
    let pixels = layer.pixels();
    for monitor in monitors {
        let (left, top) = (monitor.left - origin.0, monitor.top - origin.1);
        let (right, bottom) = (monitor.right - origin.0, monitor.bottom - origin.1);
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
    Some(layer)
}

unsafe extern "system" fn collect_monitor(
    _monitor: HMONITOR,
    _dc: HDC,
    rect: *mut RECT,
    monitors: LPARAM,
) -> BOOL {
    unsafe {
        (*(monitors.0 as *mut Vec<RECT>)).push(*rect);
    }
    BOOL(1)
}

/// A per-pixel-alpha window's image: a premultiplied BGRA bitmap selected into a memory DC.
struct Layer {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u32,
    width: i32,
    height: i32,
    origin: (i32, i32),
}

impl Layer {
    fn new(hwnd: HWND, width: i32, height: i32) -> Option<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height: rows run top to bottom.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let mut bits = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return None;
            };
            SelectObject(dc, bitmap.into());
            Some(Self {
                hwnd,
                dc,
                bitmap,
                bits: bits.cast(),
                width,
                height,
                origin: (0, 0),
            })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }

    /// Shows the image at a screen position with an overall opacity.
    fn push(&self, x: i32, y: i32, alpha: u8) {
        unsafe {
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&POINT { x, y }),
                Some(&SIZE {
                    cx: self.width,
                    cy: self.height,
                }),
                Some(self.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend(alpha)),
                ULW_ALPHA,
            );
        }
    }

    /// Changes only the overall opacity, which costs far less than pushing the image again.
    fn fade(&self, alpha: u8) {
        unsafe {
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                None,
                None,
                None,
                COLORREF(0),
                Some(&blend(alpha)),
                ULW_ALPHA,
            );
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

fn blend(alpha: u8) -> BLENDFUNCTION {
    BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    }
}

/// Paints `color` at opacity `alpha` over a premultiplied BGRA pixel.
fn over(pixel: &mut u32, color: (f32, f32, f32), alpha: f32) {
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
fn rounded_rect(p: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let q = (p.0.abs() - (half.0 - radius), p.1.abs() - (half.1 - radius));
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - radius
}

/// Signed distance from `p` to a polygon: negative inside.
fn polygon(p: (f32, f32), points: &[(f32, f32)]) -> f32 {
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
    use super::{ARROW, over, polygon, rounded_rect};

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
}
