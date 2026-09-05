use std::{thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
    fn send_inputs(
        &self,
        inputs: &mut [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

        self.begin_desktop_input()?;

        // SAFETY: `inputs` is a valid contiguous array for the duration of the call, and the
        // element size matches the Win32 `INPUT` layout from windows-sys.
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>()
                    as i32,
            )
        };
        if sent != inputs.len() as u32 {
            return Err(format!(
                "SendInput sent {sent} input events out of {}",
                inputs.len()
            ));
        }
        Ok(())
    }

    fn mouse_input_at(
        dx: i32,
        dy: i32,
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
        data: u32,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
        };

        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input(
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
        data: u32,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        Self::mouse_input_at(0, 0, flags, data)
    }

    fn normalize_absolute_mouse_coordinate(
        coordinate: i32,
        virtual_origin: i32,
        virtual_span: i32,
    ) -> Result<i32, String> {
        if virtual_span <= 1 {
            return Err(format!(
                "virtual desktop span must be greater than one pixel, got {virtual_span}"
            ));
        }
        let coordinate = i64::from(coordinate);
        let virtual_origin = i64::from(virtual_origin);
        let virtual_span = i64::from(virtual_span);
        let virtual_end = virtual_origin + virtual_span - 1;
        if coordinate < virtual_origin || coordinate > virtual_end {
            return Err(format!(
                "coordinate {coordinate} is outside virtual desktop range {virtual_origin}..={virtual_end}"
            ));
        }
        Ok(((coordinate - virtual_origin) * 65_535 / (virtual_span - 1)) as i32)
    }

    fn absolute_mouse_move_input(
        x: i32,
        y: i32,
    ) -> Result<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT, String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };

        // SAFETY: GetSystemMetrics is a process-global read with no pointer arguments.
        let (virtual_left, virtual_top, virtual_width, virtual_height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        let normalized_x =
            Self::normalize_absolute_mouse_coordinate(x, virtual_left, virtual_width)?;
        let normalized_y =
            Self::normalize_absolute_mouse_coordinate(y, virtual_top, virtual_height)?;
        Ok(Self::mouse_input_at(
            normalized_x,
            normalized_y,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            0,
        ))
    }

    fn mouse_input_for_wheel(delta: i32) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_WHEEL;

        Self::mouse_input(MOUSEEVENTF_WHEEL, delta as u32)
    }

    pub(super) fn send_mouse_wheel(&self, delta: i32) -> Result<(), String> {
        let mut inputs = [Self::mouse_input_for_wheel(delta)];
        self.send_inputs(&mut inputs)
    }

    pub(super) fn scroll_content_at(
        &self,
        window: PlatformWindowHandle,
        x: i32,
        y: i32,
        delta: i32,
    ) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::RECT,
            UI::{Input::KeyboardAndMouse::MOUSEEVENTF_WHEEL, WindowsAndMessaging::GetWindowRect},
        };
        Self::ensure_window_foreground(window, "content scroll target")?;
        let mut bounds = RECT::default();
        // SAFETY: The HWND belongs to this disposable fixture; output pointers
        // remain valid for the calls. Recheck the captured point before input.
        unsafe {
            if GetWindowRect(window, &mut bounds) == 0 {
                return Err("could not inspect native scroll coordinates".into());
            }
        }
        if x <= bounds.left || x >= bounds.right || y <= bounds.top || y >= bounds.bottom {
            return Err("captured scroll anchor is outside the fixture window".into());
        }
        // Give the GUI a pointer-move frame before the wheel event. Combining
        // both immediately can route the wheel through a tooltip at the old
        // pointer position instead of the intended scroll area.
        self.send_inputs(&mut [Self::absolute_mouse_move_input(x, y)?])?;
        thread::sleep(Duration::from_millis(80));
        let mut input = Self::absolute_mouse_move_input(x, y)?;
        // SAFETY: absolute_mouse_move_input constructs the mouse union member.
        unsafe {
            input.Anonymous.mi.dwFlags |= MOUSEEVENTF_WHEEL;
            input.Anonymous.mi.mouseData = delta as u32;
        }
        let result = self.send_inputs(&mut [input]);
        thread::sleep(Duration::from_millis(160));
        // Keep pointer ownership here until the next transaction. A timed
        // restore can be coalesced with the wheel during an expensive frame,
        // routing the scroll to the old position outside the content area.
        result
    }

    pub(super) fn click_element_center(
        &self,
        window: PlatformWindowHandle,
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        name: &str,
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };

        let rect_context = format!("{name:?}");
        let rect = Self::automation_element_bounding_rect_required(element, &rect_context)?;
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err(format!(
                "UI Automation bounding rectangle for {name:?} was empty"
            ));
        }

        let center_x = (rect.left + rect.right) / 2;
        let center_y = (rect.top + rect.bottom) / 2;
        // Keep physical input physically owned. UIA SetFocus is asynchronous in
        // AccessKit/egui and can race the following pointer transaction, leaving a
        // button focused without delivering its click. Foreground acknowledgement,
        // exact ElementFromPoint validation, and the mouse down/up pair are sufficient.
        Self::ensure_window_foreground(window, &rect_context)?;
        let priming_y = if rect.top >= 4 {
            rect.top - 4
        } else {
            rect.bottom.saturating_add(4)
        };
        let mut prime = [Self::absolute_mouse_move_input(center_x, priming_y)?];
        self.send_inputs(&mut prime)
            .map_err(|error| format!("failed to prime the pointer outside {name:?}: {error}"))?;
        thread::sleep(Duration::from_millis(80));
        let click_result = (|| -> Result<(), String> {
            Self::verify_automation_hit_target(automation, center_x, center_y, name)?;
            let mut down = [
                Self::absolute_mouse_move_input(center_x, center_y)?,
                Self::mouse_input(MOUSEEVENTF_LEFTDOWN, 0),
            ];
            self.send_inputs(&mut down)
                .map_err(|error| format!("failed to press {name:?}: {error}"))?;
            // Deliver down and up in separate frames, but atomically bind each endpoint to the
            // intended absolute coordinate. Unrelated desktop pointer movement cannot redirect
            // either half of the click between the move and button event in one SendInput call.
            thread::sleep(Duration::from_millis(40));
            let mut up = [
                Self::absolute_mouse_move_input(center_x, center_y)?,
                Self::mouse_input(MOUSEEVENTF_LEFTUP, 0),
            ];
            self.send_inputs(&mut up)
                .map_err(|error| format!("failed to release {name:?}: {error}"))
        })();
        // Keep pointer ownership on the target until the next explicit interaction. A timed
        // restore is inherently racy: under a slow frame the restore move can be coalesced with
        // the down/up pair and make egui observe the release outside the widget.
        thread::sleep(Duration::from_millis(120));
        click_result?;
        Ok(())
    }

    fn keyboard_input_for_vk(
        vk: u16,
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        };

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard_input_for_unicode(
        ch: u16,
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_UNICODE,
        };

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: ch,
                    dwFlags: flags | KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub(super) fn send_select_all_backspace_and_type(&self, value: &str) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            KEYEVENTF_KEYUP, VK_BACK, VK_CONTROL,
        };

        let mut controls = [
            Self::keyboard_input_for_vk(VK_CONTROL, 0),
            Self::keyboard_input_for_vk('A' as u16, 0),
            Self::keyboard_input_for_vk('A' as u16, KEYEVENTF_KEYUP),
            Self::keyboard_input_for_vk(VK_CONTROL, KEYEVENTF_KEYUP),
            Self::keyboard_input_for_vk(VK_BACK, 0),
            Self::keyboard_input_for_vk(VK_BACK, KEYEVENTF_KEYUP),
        ];
        self.send_inputs(&mut controls)?;

        if value.is_empty() {
            return Ok(());
        }

        let mut text_inputs = Vec::with_capacity(value.encode_utf16().count() * 2);
        for ch in value.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                if !text_inputs.is_empty() {
                    self.send_inputs(&mut text_inputs)?;
                    text_inputs.clear();
                }
                self.send_enter_key()?;
                continue;
            }
            let mut utf16_buffer = [0u16; 2];
            for code_unit in ch.encode_utf16(&mut utf16_buffer) {
                text_inputs.push(Self::keyboard_input_for_unicode(*code_unit, 0));
                text_inputs.push(Self::keyboard_input_for_unicode(
                    *code_unit,
                    KEYEVENTF_KEYUP,
                ));
            }
        }
        if text_inputs.is_empty() {
            Ok(())
        } else {
            self.send_inputs(&mut text_inputs)
        }
    }

    pub(super) fn send_enter_key(&self) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_RETURN};

        let mut enter_inputs = [
            Self::keyboard_input_for_vk(VK_RETURN, 0),
            Self::keyboard_input_for_vk(VK_RETURN, KEYEVENTF_KEYUP),
        ];
        self.send_inputs(&mut enter_inputs)
    }

    pub(super) fn send_escape_key(&self) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_ESCAPE};

        let mut escape_inputs = [
            Self::keyboard_input_for_vk(VK_ESCAPE, 0),
            Self::keyboard_input_for_vk(VK_ESCAPE, KEYEVENTF_KEYUP),
        ];
        self.send_inputs(&mut escape_inputs)
    }

    pub(super) fn send_page_down_key(&self) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_NEXT};

        let mut page_down_inputs = [
            Self::keyboard_input_for_vk(VK_NEXT, 0),
            Self::keyboard_input_for_vk(VK_NEXT, KEYEVENTF_KEYUP),
        ];
        self.send_inputs(&mut page_down_inputs)
    }

    pub(super) fn send_page_up_key(&self) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_PRIOR};

        let mut page_up_inputs = [
            Self::keyboard_input_for_vk(VK_PRIOR, 0),
            Self::keyboard_input_for_vk(VK_PRIOR, KEYEVENTF_KEYUP),
        ];
        self.send_inputs(&mut page_up_inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::PlatformNativeGuiDriver;

    #[test]
    fn absolute_mouse_coordinates_cover_virtual_desktop_endpoints() {
        assert_eq!(
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(-1_920, -1_920, 3_840)
                .unwrap(),
            0
        );
        assert_eq!(
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(1_919, -1_920, 3_840)
                .unwrap(),
            65_535
        );
        let center =
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(0, -1_920, 3_840).unwrap();
        assert!((32_767..=32_785).contains(&center));
    }

    #[test]
    fn absolute_mouse_coordinates_reject_invalid_span_and_out_of_bounds_points() {
        assert!(
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(0, 0, 1)
                .unwrap_err()
                .contains("greater than one")
        );
        assert!(
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(-1, 0, 1_920)
                .unwrap_err()
                .contains("outside virtual desktop")
        );
        assert!(
            PlatformNativeGuiDriver::normalize_absolute_mouse_coordinate(1_920, 0, 1_920)
                .unwrap_err()
                .contains("outside virtual desktop")
        );
    }
}
