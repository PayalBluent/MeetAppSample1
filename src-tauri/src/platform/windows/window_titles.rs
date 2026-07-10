use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
};

/// Return the titles of all visible top-level windows.
pub fn visible_window_titles() -> Vec<String> {
    let mut titles: Vec<String> = Vec::new();
    unsafe {
        // EnumWindows returns Err if the callback stops early; we always continue,
        // so ignore the result.
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut titles as *mut _ as isize));
    }
    titles
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let titles = &mut *(lparam.0 as *mut Vec<String>);

    if IsWindowVisible(hwnd).as_bool() {
        let len = GetWindowTextLengthW(hwnd);
        if len > 0 {
            let mut buf = vec![0u16; (len + 1) as usize];
            let read = GetWindowTextW(hwnd, &mut buf);
            if read > 0 {
                titles.push(String::from_utf16_lossy(&buf[..read as usize]));
            }
        }
    }
    TRUE
}
