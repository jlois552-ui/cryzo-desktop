//! Window capture for the OCR fallback (Windows.Media.Ocr: built in, local, no model download) and
//! for screenshots, the last resort for genuinely visual questions.

use anyhow::{Result, bail};
use windows::{
    Graphics::Imaging::{BitmapEncoder, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Security::Cryptography::CryptographicBuffer,
    Storage::Streams::{DataReader, InMemoryRandomAccessStream},
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap,
            CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SRCCOPY, SelectObject,
        },
        Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect},
    },
};

/// A window's pixels in screen coordinates, as top-down BGRA.
pub struct Capture {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    bgra: Vec<u8>,
}

/// Captures a window even when other windows cover it.
pub fn capture(hwnd: isize) -> Result<Capture> {
    let hwnd = HWND(hwnd as *mut _);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect)? };
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        bail!("The window has no visible area (is it minimized?)");
    }
    // Some GPU-drawn windows, Cursor's for one, print as a single flat colour or not at all. When
    // the window is in front, the screen itself shows it, so copy that instead.
    let printed = grab(hwnd, rect, false);
    let flat = |bgra: &Vec<u8>| {
        bgra.as_chunks::<4>()
            .0
            .windows(2)
            .all(|pair| pair[0] == pair[1])
    };
    let mut bgra = if printed.as_ref().is_ok_and(|bgra| !flat(bgra))
        || unsafe { GetForegroundWindow() } != hwnd
    {
        printed?
    } else {
        grab(hwnd, rect, true)?
    };
    // GDI leaves alpha at zero, which WinRT would treat as fully transparent.
    bgra.as_chunks_mut::<4>()
        .0
        .iter_mut()
        .for_each(|pixel| pixel[3] = 255);
    Ok(Capture {
        left: rect.left,
        top: rect.top,
        width,
        height,
        bgra,
    })
}

/// The window's pixels, printed by the window itself or copied from the screen.
fn grab(hwnd: HWND, rect: RECT, from_screen: bool) -> Result<Vec<u8>> {
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    let (printed, rows) = unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());
        let printed = if from_screen {
            BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                rect.left,
                rect.top,
                SRCCOPY,
            )
            .is_ok()
        } else {
            // PW_RENDERFULLCONTENT (2) includes GPU-composed content such as Chromium and WebView2.
            PrintWindow(hwnd, memory, PRINT_WINDOW_FLAGS(2)).as_bool()
        };
        SelectObject(memory, previous);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let rows = GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        (printed, rows)
    };
    if !printed || rows == 0 {
        bail!("Windows refused to capture this window");
    }
    Ok(bgra)
}

thread_local! {
    /// Creating the OCR engine took seconds on a cold start, so each thread keeps one.
    static OCR: std::cell::OnceCell<Option<OcrEngine>> = const { std::cell::OnceCell::new() };
}

impl Capture {
    /// Recognized lines, each with the screen point at its centre, for `click_at`.
    pub fn ocr(&self) -> Result<Vec<String>> {
        let engine = OCR
            .with(|engine| {
                engine
                    .get_or_init(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())
                    .clone()
            })
            .ok_or_else(|| {
                anyhow::anyhow!("Windows OCR is not available for this user's languages")
            })?;
        let result = engine.RecognizeAsync(&self.bitmap()?)?.join()?;
        result
            .Lines()?
            .into_iter()
            .map(|line| {
                let boxes = line
                    .Words()?
                    .into_iter()
                    .map(|word| word.BoundingRect())
                    .collect::<windows::core::Result<Vec<_>>>()?;
                let left = boxes.iter().map(|rect| rect.X).fold(f32::MAX, f32::min);
                let top = boxes.iter().map(|rect| rect.Y).fold(f32::MAX, f32::min);
                let right = boxes
                    .iter()
                    .map(|rect| rect.X + rect.Width)
                    .fold(0.0, f32::max);
                let bottom = boxes
                    .iter()
                    .map(|rect| rect.Y + rect.Height)
                    .fold(0.0, f32::max);
                let x = self.left + ((left + right) / 2.0) as i32;
                let y = self.top + ((top + bottom) / 2.0) as i32;
                Ok(format!("{:?} @{x},{y}", line.Text()?.to_string()))
            })
            .collect()
    }

    pub fn jpeg(&self) -> Result<Vec<u8>> {
        let stream = InMemoryRandomAccessStream::new()?;
        let encoder =
            BitmapEncoder::CreateAsync(BitmapEncoder::JpegEncoderId()?, &stream)?.join()?;
        encoder.SetSoftwareBitmap(&self.bitmap()?)?;
        encoder.FlushAsync()?.join()?;
        let size = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.join()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }

    fn bitmap(&self) -> Result<SoftwareBitmap> {
        let buffer = CryptographicBuffer::CreateFromByteArray(&self.bgra)?;
        Ok(SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            self.width,
            self.height,
        )?)
    }
}
