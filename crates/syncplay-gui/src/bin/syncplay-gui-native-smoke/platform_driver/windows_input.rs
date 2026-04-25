use std::{thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
    fn send_keyboard_inputs(
        inputs: &mut [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

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
                "SendInput sent {sent} keyboard events out of {}",
                inputs.len()
            ));
        }
        Ok(())
    }

    fn mouse_input(
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
                    dx: 0,
                    dy: 0,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input_for_wheel(delta: i32) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_WHEEL;

        Self::mouse_input(MOUSEEVENTF_WHEEL, delta as u32)
    }

    pub(super) fn send_mouse_wheel(delta: i32) -> Result<(), String> {
        let mut inputs = [Self::mouse_input_for_wheel(delta)];
        Self::send_keyboard_inputs(&mut inputs)
    }

    pub(super) fn click_element_center(
        window: PlatformWindowHandle,
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        name: &str,
    ) -> Result<(), String> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };

        unsafe extern "system" {
            fn ScreenToClient(hwnd: PlatformWindowHandle, point: *mut POINT) -> i32;
        }

        let rect_context = format!("{name:?}");
        let rect = Self::automation_element_bounding_rect_required(element, &rect_context)?;
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err(format!(
                "UI Automation bounding rectangle for {name:?} was empty"
            ));
        }

        let center_x = (rect.left + rect.right) / 2;
        let center_y = (rect.top + rect.bottom) / 2;
        let mut client_point = POINT {
            x: center_x,
            y: center_y,
        };
        Self::focus_window_element(window, element, &rect_context)?;
        // SAFETY: `client_point` is a valid mutable POINT and `window` is the HWND under test;
        // failure is converted into a driver error.
        unsafe {
            if ScreenToClient(window, &mut client_point) == 0 {
                return Err(format!(
                    "failed to convert {name:?} center ({center_x}, {center_y}) to client coordinates"
                ));
            }
        }
        thread::sleep(Duration::from_millis(80));
        // SAFETY: The lparam encodes client coordinates for the current HWND. Messages are sent
        // synchronously to the GUI window under test to emulate a click in the smoke driver.
        unsafe {
            let lparam =
                ((client_point.y as u32) << 16 | (client_point.x as u32 & 0xffff)) as isize;
            SendMessageW(window, WM_MOUSEMOVE, 0, lparam);
            SendMessageW(window, WM_LBUTTONDOWN, 0x0001, lparam);
            SendMessageW(window, WM_LBUTTONUP, 0, lparam);
        }
        thread::sleep(Duration::from_millis(120));
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

    pub(super) fn send_select_all_backspace_and_type(value: &str) -> Result<(), String> {
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
        Self::send_keyboard_inputs(&mut controls)?;

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
                    Self::send_keyboard_inputs(&mut text_inputs)?;
                    text_inputs.clear();
                }
                Self::send_enter_key()?;
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
            Self::send_keyboard_inputs(&mut text_inputs)
        }
    }

    pub(super) fn send_enter_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_RETURN};

        let mut enter_inputs = [
            Self::keyboard_input_for_vk(VK_RETURN, 0),
            Self::keyboard_input_for_vk(VK_RETURN, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut enter_inputs)
    }

    pub(super) fn send_page_down_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_NEXT};

        let mut page_down_inputs = [
            Self::keyboard_input_for_vk(VK_NEXT, 0),
            Self::keyboard_input_for_vk(VK_NEXT, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut page_down_inputs)
    }

    pub(super) fn send_page_up_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_PRIOR};

        let mut page_up_inputs = [
            Self::keyboard_input_for_vk(VK_PRIOR, 0),
            Self::keyboard_input_for_vk(VK_PRIOR, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut page_up_inputs)
    }
}
