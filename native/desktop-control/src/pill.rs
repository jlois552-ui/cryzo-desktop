//! The overlay's pill on macOS and Linux: the current step and a Stop button, painted into
//! premultiplied ARGB pixels with a system font so both platforms look like Windows' pill.

use crate::motion::{ACCENT, over, rounded_rect};

/// Pill layout in unscaled pixels, as on Windows: height, side padding, status dot, gaps and the
/// Stop button.
const HEIGHT: f32 = 40.0;
const PAD: f32 = 14.0;
const DOT: f32 = 8.0;
const GAP: f32 = 10.0;
const STOP_WIDTH: f32 = 56.0;
const STOP_HEIGHT: f32 = 26.0;
const TEXT: f32 = 14.0;
const MAX_TEXT: f32 = 520.0;

/// A painted pill.
pub(crate) struct Image {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
    /// Where a click means Stop, from the pill's left edge, in the image's pixels. None once
    /// stopped.
    pub stop_from: Option<i32>,
}

pub(crate) struct Painter {
    font: Option<fontdue::Font>,
}

impl Painter {
    /// Loads a sans-serif system font. Without one the pill still shows its dot and Stop button.
    pub(crate) fn load() -> Self {
        let font = font_files().into_iter().find_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
        });
        Self { font }
    }

    pub(crate) fn paint(&self, text: &str, stopped: bool, scale: f32) -> Image {
        let size = TEXT * scale;
        let text = self.fit(text, MAX_TEXT * scale, size);
        let text_width = self.measure(&text, size);
        let button = if stopped { 0.0 } else { GAP + STOP_WIDTH };
        let width = (text_width + (PAD * 2.0 + DOT + GAP + button) * scale).ceil() as i32;
        let height = (HEIGHT * scale) as i32;
        let mut pixels = vec![0u32; (width * height) as usize];
        let middle = height as f32 / 2.0;
        let stop_left = width as f32 - (PAD + STOP_WIDTH) * scale;
        for y in 0..height {
            for x in 0..width {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let pixel = &mut pixels[(y * width + x) as usize];
                let shape = rounded_rect(
                    (p.0 - width as f32 / 2.0, p.1 - middle),
                    (width as f32 / 2.0, middle),
                    middle,
                );
                over(pixel, (32.0, 31.0, 36.0), (0.5 - shape).clamp(0.0, 1.0));
                let dot_centre = ((PAD + DOT / 2.0) * scale, middle);
                let dot = ((p.0 - dot_centre.0).powi(2) + (p.1 - dot_centre.1).powi(2)).sqrt()
                    - DOT / 2.0 * scale;
                let colour = if stopped { (230.0, 80.0, 80.0) } else { ACCENT };
                over(pixel, colour, (0.5 - dot).clamp(0.0, 1.0));
                if !stopped {
                    let half = (STOP_WIDTH / 2.0 * scale, STOP_HEIGHT / 2.0 * scale);
                    let centre = (stop_left + half.0, middle);
                    let button = rounded_rect((p.0 - centre.0, p.1 - centre.1), half, 12.0 * scale);
                    over(pixel, (64.0, 62.0, 72.0), (0.5 - button).clamp(0.0, 1.0));
                }
            }
        }
        let text_left = (PAD + DOT + GAP) * scale;
        self.draw(&mut pixels, width, &text, text_left, middle, size);
        if !stopped {
            let label = self.measure("Stop", size);
            let left = stop_left + (STOP_WIDTH * scale - label) / 2.0;
            self.draw(&mut pixels, width, "Stop", left, middle, size);
        }
        Image {
            width,
            height,
            pixels,
            stop_from: (!stopped).then_some(stop_left as i32),
        }
    }

    fn measure(&self, text: &str, size: f32) -> f32 {
        let Some(font) = &self.font else {
            return 0.0;
        };
        text.chars()
            .map(|character| font.metrics(character, size).advance_width)
            .sum()
    }

    /// The text, ending in an ellipsis where it would run wider than `limit`.
    fn fit(&self, text: &str, limit: f32, size: f32) -> String {
        if self.measure(text, size) <= limit {
            return text.to_owned();
        }
        let mut fitted = text.chars().collect::<Vec<_>>();
        while !fitted.is_empty()
            && self.measure(&format!("{}…", fitted.iter().collect::<String>()), size) > limit
        {
            fitted.pop();
        }
        format!("{}…", fitted.iter().collect::<String>().trim_end())
    }

    /// Draws text in the pill's light grey, vertically centred on `middle`.
    fn draw(&self, pixels: &mut [u32], width: i32, text: &str, left: f32, middle: f32, size: f32) {
        let Some(font) = &self.font else {
            return;
        };
        let height = pixels.len() as i32 / width;
        let line = font.horizontal_line_metrics(size);
        let baseline = line.map_or(middle + size * 0.35, |line| {
            middle + (line.ascent + line.descent) / 2.0
        });
        let mut pen = left;
        for character in text.chars() {
            let (metrics, coverage) = font.rasterize(character, size);
            let origin_x = (pen + metrics.xmin as f32).round() as i32;
            let origin_y = (baseline - metrics.height as f32 - metrics.ymin as f32).round() as i32;
            for row in 0..metrics.height {
                for column in 0..metrics.width {
                    let (x, y) = (origin_x + column as i32, origin_y + row as i32);
                    if x < 0 || y < 0 || x >= width || y >= height {
                        continue;
                    }
                    let alpha = coverage[row * metrics.width + column] as f32 / 255.0;
                    over(
                        &mut pixels[(y * width + x) as usize],
                        (240.0, 240.0, 245.0),
                        alpha,
                    );
                }
            }
            pen += metrics.advance_width;
        }
    }
}

/// Font files to try, most likely first.
fn font_files() -> Vec<String> {
    #[cfg(target_os = "macos")]
    let mut files = vec![
        "/System/Library/Fonts/SFNS.ttf".to_owned(),
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf".to_owned(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".to_owned(),
        "/Library/Fonts/Arial.ttf".to_owned(),
    ];
    #[cfg(not(target_os = "macos"))]
    let mut files = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "sans:bold"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .filter(|file| !file.is_empty())
        .into_iter()
        .collect::<Vec<_>>();
    files.extend(
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-M.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        ]
        .map(str::to_owned),
    );
    files
}
