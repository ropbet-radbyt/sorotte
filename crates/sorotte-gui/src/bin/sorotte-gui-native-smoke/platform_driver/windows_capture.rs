use std::{mem::size_of, path::Path, ptr, slice, thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle, png::write_bgrx_png};

// DWM's extended frame includes a thin transparent compositing fringe above and below the
// hardware-rendered window on Windows 11. BitBlt resolves that fringe against whatever happens
// to be behind the app, so capture a slightly taller frame and trim only those transparent rows.
const DWM_CAPTURE_TOP_INSET: i32 = 2;
const DWM_CAPTURE_BOTTOM_INSET: i32 = 3;

fn raw_window_bounds(
    window: PlatformWindowHandle,
) -> Result<windows_sys::Win32::Foundation::RECT, String> {
    use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};

    let mut bounds = RECT::default();
    // SAFETY: `window` is the HWND discovered for the GUI process under test and `bounds` is
    // valid writable storage for the duration of the call.
    if unsafe { GetWindowRect(window, &mut bounds) } == 0 {
        return Err("failed to read native smoke window bounds".to_owned());
    }
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Err(format!(
            "native smoke window bounds were empty: left={}, top={}, right={}, bottom={}",
            bounds.left, bounds.top, bounds.right, bounds.bottom
        ));
    }
    Ok(bounds)
}

fn visible_window_bounds(
    window: PlatformWindowHandle,
) -> Result<windows_sys::Win32::Foundation::RECT, String> {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
    };

    let mut bounds = RECT::default();
    // SAFETY: `window` is the HWND under test. `bounds` is valid writable RECT storage and the
    // byte count exactly matches that storage. DWM returns the visible frame without the
    // invisible resize border reported by GetWindowRect.
    let status = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut bounds as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if status < 0 {
        return Err(format!(
            "failed to read visible native smoke window bounds (DWM HRESULT 0x{:08x})",
            status as u32
        ));
    }
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Err(format!(
            "visible native smoke window bounds were empty: left={}, top={}, right={}, bottom={}",
            bounds.left, bounds.top, bounds.right, bounds.bottom
        ));
    }
    Ok(bounds)
}

fn set_outer_window_bounds(
    window: PlatformWindowHandle,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOZORDER, SetWindowPos};

    // SAFETY: `window` is the GUI HWND under test. The dimensions were derived from valid DWM
    // and GetWindowRect measurements, and z-order is deliberately preserved.
    if unsafe {
        SetWindowPos(
            window,
            ptr::null_mut(),
            left,
            top,
            width,
            height,
            SWP_NOZORDER,
        )
    } == 0
    {
        return Err("failed to set native smoke window bounds".to_owned());
    }
    Ok(())
}

struct CaptureTopmostScope {
    window: PlatformWindowHandle,
}

impl CaptureTopmostScope {
    fn activate(window: PlatformWindowHandle) -> Result<Self, String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };

        // SAFETY: `window` is the GUI HWND under test. The flags preserve its configured bounds
        // while temporarily making it topmost so a desktop pixel copy cannot capture an
        // unrelated occluding window.
        if unsafe {
            SetWindowPos(
                window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            )
        } == 0
        {
            return Err("failed to bring native smoke window above desktop occluders".to_owned());
        }
        Ok(Self { window })
    }
}

impl Drop for CaptureTopmostScope {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };

        // SAFETY: The scope is created only after this HWND was successfully made topmost. The
        // flags restore ordinary z-order without changing the deterministic test bounds.
        unsafe {
            SetWindowPos(
                self.window,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }
    }
}

impl PlatformNativeGuiDriver {
    pub(super) fn prepare_visible_window_bounds(
        window: PlatformWindowHandle,
        target_left: i32,
        target_top: i32,
        target_width: i32,
        target_height: i32,
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        if target_width <= 0 || target_height <= 0 {
            return Err("native smoke visible window dimensions must be positive".to_owned());
        }
        // SAFETY: `window` is the GUI HWND under test; foreground activation is best effort and
        // resizing below remains authoritative.
        unsafe {
            SetForegroundWindow(window);
        }

        for _ in 0..3 {
            let outer = raw_window_bounds(window)?;
            let visible = visible_window_bounds(window)?;
            let visible_width = visible.right - visible.left;
            let visible_height = visible.bottom - visible.top;
            if visible.left == target_left
                && visible.top == target_top
                && visible_width == target_width
                && visible_height == target_height
            {
                return Ok(());
            }

            let horizontal_decoration = (outer.right - outer.left).saturating_sub(visible_width);
            let vertical_decoration = (outer.bottom - outer.top).saturating_sub(visible_height);
            let visible_left_inset = visible.left - outer.left;
            let visible_top_inset = visible.top - outer.top;
            set_outer_window_bounds(
                window,
                target_left - visible_left_inset,
                target_top - visible_top_inset,
                target_width + horizontal_decoration,
                target_height + vertical_decoration,
            )?;
            thread::sleep(Duration::from_millis(120));
        }

        let visible = visible_window_bounds(window)?;
        Err(format!(
            "visible native smoke window bounds did not settle at {target_left},{target_top} {target_width}x{target_height}; got {},{} {}x{}",
            visible.left,
            visible.top,
            visible.right - visible.left,
            visible.bottom - visible.top
        ))
    }

    pub(super) fn capture_window_png_internal(
        window: PlatformWindowHandle,
        output_path: &Path,
    ) -> Result<(), String> {
        use windows_sys::Win32::{
            Graphics::Gdi::{
                BI_RGB, BITMAPINFO, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
                DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
            },
            UI::WindowsAndMessaging::SetForegroundWindow,
        };

        let requested_bounds = visible_window_bounds(window)?;
        let requested_width = requested_bounds.right - requested_bounds.left;
        let requested_height = requested_bounds.bottom - requested_bounds.top;
        Self::prepare_visible_window_bounds(
            window,
            requested_bounds.left,
            requested_bounds.top,
            requested_width,
            requested_height + DWM_CAPTURE_TOP_INSET + DWM_CAPTURE_BOTTOM_INSET,
        )?;
        let visible_bounds = visible_window_bounds(window)?;
        let bounds = trimmed_capture_bounds(visible_bounds)?;
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        if width <= 0 || height <= 0 {
            return Err(format!(
                "native smoke window bounds were empty for screenshot: left={}, top={}, right={}, bottom={}",
                bounds.left, bounds.top, bounds.right, bounds.bottom
            ));
        }

        // Bring the tested window above all desktop occluders before copying pixels. This captures
        // the actual hardware-rendered egui frame, which PrintWindow does not reliably provide.
        let _topmost_scope = CaptureTopmostScope::activate(window)?;
        // SAFETY: `window` is the GUI HWND under test; failure to change foreground ownership is
        // harmless because the topmost scope already guarantees visibility for BitBlt.
        unsafe {
            SetForegroundWindow(window);
        }
        thread::sleep(Duration::from_millis(200));

        // SAFETY: A null HWND requests the desktop DC. Every acquired GDI object is checked and
        // released below on all subsequent paths.
        let screen_dc = unsafe { GetDC(ptr::null_mut()) };
        if screen_dc.is_null() {
            return Err("failed to acquire desktop device context for screenshot".to_owned());
        }
        // SAFETY: `screen_dc` is a live desktop device context.
        let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
        if memory_dc.is_null() {
            // SAFETY: `screen_dc` came from GetDC with a null HWND.
            unsafe {
                ReleaseDC(ptr::null_mut(), screen_dc);
            }
            return Err("failed to create compatible screenshot device context".to_owned());
        }

        let mut bitmap_info = BITMAPINFO::default();
        bitmap_info.bmiHeader.biSize =
            size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        bitmap_info.bmiHeader.biWidth = width;
        bitmap_info.bmiHeader.biHeight = -height;
        bitmap_info.bmiHeader.biPlanes = 1;
        bitmap_info.bmiHeader.biBitCount = 32;
        bitmap_info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = ptr::null_mut();
        // SAFETY: `bitmap_info` describes a top-down 32-bit DIB and `pixels` is a valid
        // out-parameter. A null section handle requests process-owned memory.
        let bitmap = unsafe {
            CreateDIBSection(
                screen_dc,
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut pixels,
                ptr::null_mut(),
                0,
            )
        };
        if bitmap.is_null() || pixels.is_null() {
            // SAFETY: Both DCs were acquired successfully above.
            unsafe {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(memory_dc);
                ReleaseDC(ptr::null_mut(), screen_dc);
            }
            return Err("failed to allocate screenshot bitmap".to_owned());
        }

        // SAFETY: `memory_dc` and `bitmap` are live GDI objects; the returned prior selection is
        // restored before the bitmap is deleted.
        let previous_object = unsafe { SelectObject(memory_dc, bitmap) };
        let copied = if previous_object.is_null() {
            false
        } else {
            // SAFETY: Both DCs and the selected bitmap are live. Coordinates and dimensions come
            // directly from GetWindowRect and fit i32 by definition.
            unsafe {
                BitBlt(
                    memory_dc,
                    0,
                    0,
                    width,
                    height,
                    screen_dc,
                    bounds.left,
                    bounds.top,
                    SRCCOPY | CAPTUREBLT,
                ) != 0
            }
        };

        let pixel_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| "native screenshot buffer length overflowed".to_owned());
        let encoded = match (copied, pixel_len) {
            (true, Ok(pixel_len)) => {
                // SAFETY: CreateDIBSection allocated at least width * height * 4 bytes for the
                // top-down 32-bit bitmap, which remains selected and live during this copy.
                let pixels = unsafe { slice::from_raw_parts(pixels.cast::<u8>(), pixel_len) };
                if bgrx_has_color_variation(pixels) {
                    write_bgrx_png(output_path, width as u32, height as u32, pixels)
                } else {
                    Err("captured native smoke window was uniformly blank".to_owned())
                }
            }
            (false, _) => Err("failed to copy native smoke window pixels".to_owned()),
            (_, Err(error)) => Err(error),
        };

        // SAFETY: Restore the prior GDI selection before deleting the bitmap; then release every
        // object acquired above exactly once.
        unsafe {
            if !previous_object.is_null() {
                SelectObject(memory_dc, previous_object);
            }
            DeleteObject(bitmap);
            DeleteDC(memory_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
        }
        encoded
    }
}

fn bgrx_has_color_variation(pixels: &[u8]) -> bool {
    let Some(first) = pixels.get(..3) else {
        return false;
    };
    pixels
        .chunks_exact(4)
        .skip(1)
        .any(|pixel| &pixel[..3] != first)
}

fn trimmed_capture_bounds(
    mut bounds: windows_sys::Win32::Foundation::RECT,
) -> Result<windows_sys::Win32::Foundation::RECT, String> {
    bounds.top += DWM_CAPTURE_TOP_INSET;
    bounds.bottom -= DWM_CAPTURE_BOTTOM_INSET;
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Err("native screenshot bounds were empty after trimming the DWM fringe".to_owned());
    }
    Ok(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_capture_rejects_an_invalid_window_handle() {
        let output = std::env::temp_dir().join("sorotte-invalid-window-capture.png");
        let _ = std::fs::remove_file(&output);
        let error =
            PlatformNativeGuiDriver::capture_window_png_internal(std::ptr::null_mut(), &output)
                .expect_err("a null HWND must not produce a screenshot");
        assert!(error.contains("window bounds"));
        assert!(!output.exists());
    }

    #[test]
    fn blank_capture_detection_ignores_unused_alpha_channel() {
        assert!(!bgrx_has_color_variation(&[1, 2, 3, 0, 1, 2, 3, 255]));
        assert!(bgrx_has_color_variation(&[1, 2, 3, 0, 1, 2, 4, 0]));
    }

    #[test]
    fn dwm_fringe_trim_restores_the_requested_capture_height() {
        let bounds = windows_sys::Win32::Foundation::RECT {
            left: 32,
            top: 32,
            right: 1732,
            bottom: 1137,
        };
        let trimmed = trimmed_capture_bounds(bounds).expect("valid padded frame");
        assert_eq!(trimmed.right - trimmed.left, 1700);
        assert_eq!(trimmed.bottom - trimmed.top, 1100);
    }
}
