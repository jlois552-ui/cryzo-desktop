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
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
        },
        Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
        UI::WindowsAndMessaging::GetWindowRect,
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
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    let (printed, rows) = unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());
        // PW_RENDERFULLCONTENT (2) includes GPU-composed content such as Chromium and WebView2.
        let printed = PrintWindow(hwnd, memory, PRINT_WINDOW_FLAGS(2)).as_bool();
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

impl Capture {
    /// Recognized lines, each with the screen point at its centre, for `click_at`.
    pub fn ocr(&self) -> Result<Vec<String>> {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
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
