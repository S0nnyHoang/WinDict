//! Lấy chữ đang được chọn ở ứng dụng khác: giả lập Ctrl+C, đọc clipboard,
//! rồi trả lại nội dung clipboard cũ cho người dùng.

use std::mem::{size_of, zeroed};
use std::ptr::{copy_nonoverlapping, null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MsgWaitForMultipleObjects, PeekMessageW, MSG};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
};

use crate::util::trace;

const CF_UNICODETEXT: u32 = 13;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_C: u16 = 0x43;
/// Mã phím không gán chức năng; chen vào để việc nhả Alt/Win không bật menu hay Start.
const VK_MASK: u16 = 0xE8;

pub struct Capture {
    pub text: Option<String>,
    saved: Option<Vec<(u32, Vec<u8>)>>,
}

impl Capture {
    /// Trả lại clipboard cũ. Gọi sau khi popup đã hiện để không làm chậm việc hiển thị.
    pub unsafe fn restore(self, owner: HWND) {
        if let Some(saved) = self.saved {
            restore_clipboard(owner, &saved);
            trace("clipboard restored");
        }
    }
}

/// Mặc định Windows chỉ "tỉnh dậy" mỗi 15,6 ms; tăng độ phân giải lên 1 ms
/// chỉ trong lúc chờ clipboard để phản hồi nhanh, xong thì trả lại ngay.
struct FineTimer;

impl FineTimer {
    fn new() -> Self {
        unsafe { timeBeginPeriod(1) };
        FineTimer
    }
}

impl Drop for FineTimer {
    fn drop(&mut self) {
        unsafe { timeEndPeriod(1) };
    }
}

/// Chờ khoảng 1 ms nhưng vẫn xử lý các thông điệp được gửi đồng bộ từ luồng khác
/// (ví dụ báo mất focus khi người dùng chuyển ứng dụng). Nếu chỉ Sleep, ứng dụng
/// nguồn có thể bị treo chờ ta trả lời và xử lý Ctrl+C rất chậm.
unsafe fn wait_1ms() {
    const QS_SENDMESSAGE: u32 = 0x40;
    MsgWaitForMultipleObjects(0, null(), 0, 1, QS_SENDMESSAGE);
    let mut msg: MSG = zeroed();
    PeekMessageW(&mut msg, null_mut(), 0, 0, QS_SENDMESSAGE << 16); // PM_NOREMOVE | PM_QS_SENDMESSAGE
}

pub unsafe fn capture_selection(owner: HWND) -> Capture {
    let _timer = FineTimer::new();
    let saved = save_clipboard(owner);
    trace("clipboard saved");
    let seq = GetClipboardSequenceNumber();

    // Không chờ người dùng nhả phím tắt: nhả hộ Alt/Shift/Win đang giữ
    // để Ctrl+C giả lập không bị biến thành Ctrl+Alt+C.
    let mut keys = vec![(VK_MASK, false), (VK_MASK, true)];
    for k in [VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
        if GetAsyncKeyState(k as i32) as u16 & 0x8000 != 0 {
            keys.push((k, true));
        }
    }
    keys.extend([(VK_CONTROL, false), (VK_C, false), (VK_C, true), (VK_CONTROL, true)]);
    send_keys(&keys);
    trace("ctrl+c sent");

    let start = Instant::now();
    while GetClipboardSequenceNumber() == seq {
        if start.elapsed() > Duration::from_millis(500) {
            // Không có gì được chọn; clipboard chưa bị đụng tới.
            return Capture { text: None, saved: None };
        }
        wait_1ms();
    }
    trace("clipboard changed");
    let text = read_text(owner).filter(|t| !t.trim().is_empty());
    trace("text read");
    Capture { text, saved }
}

unsafe fn send_keys(keys: &[(u16, bool)]) {
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&(vk, up)| {
            let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
            if matches!(vk, VK_LWIN | VK_RWIN) {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            let mut i: INPUT = zeroed();
            i.r#type = INPUT_KEYBOARD;
            i.Anonymous.ki = KEYBDINPUT {
                wVk: vk,
                wScan: MapVirtualKeyW(vk as u32, 0) as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            };
            i
        })
        .collect();
    SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
}

unsafe fn open_clipboard(owner: HWND) -> bool {
    for _ in 0..200 {
        if OpenClipboard(owner) != 0 {
            return true;
        }
        wait_1ms();
    }
    false
}

/// Định dạng không phải HGLOBAL (bitmap, metafile, palette...) hoặc thuộc riêng
/// ứng dụng khác thì không sao chép được.
fn skip_format(f: u32) -> bool {
    matches!(f, 2 | 3 | 9 | 14) || (0x80..=0x8F).contains(&f) || (0x200..=0x3FF).contains(&f)
}

unsafe fn save_clipboard(owner: HWND) -> Option<Vec<(u32, Vec<u8>)>> {
    if !open_clipboard(owner) {
        return None;
    }
    let mut saved = Vec::new();
    let mut f = 0;
    loop {
        f = EnumClipboardFormats(f);
        if f == 0 {
            break;
        }
        if skip_format(f) {
            continue;
        }
        let h = GetClipboardData(f);
        if h.is_null() {
            continue;
        }
        let size = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            continue;
        }
        if size > 0 {
            saved.push((f, std::slice::from_raw_parts(p, size).to_vec()));
        }
        GlobalUnlock(h);
    }
    CloseClipboard();
    Some(saved)
}

unsafe fn restore_clipboard(owner: HWND, saved: &[(u32, Vec<u8>)]) {
    if !open_clipboard(owner) {
        return;
    }
    EmptyClipboard();
    for (f, data) in saved {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
        if h.is_null() {
            continue;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            continue;
        }
        copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        if SetClipboardData(*f, h).is_null() {
            GlobalFree(h);
        }
    }
    CloseClipboard();
}

/// Vài ứng dụng ghi clipboard thành nhiều lượt; thử lại ngắn nếu chưa có chữ.
unsafe fn read_text(owner: HWND) -> Option<String> {
    let start = Instant::now();
    loop {
        if !open_clipboard(owner) {
            return None;
        }
        let text = read_open_text();
        CloseClipboard();
        if text.is_some() || start.elapsed() > Duration::from_millis(100) {
            return text;
        }
        wait_1ms();
    }
}

unsafe fn read_open_text() -> Option<String> {
    let mut text = None;
    let h = GetClipboardData(CF_UNICODETEXT);
    if !h.is_null() {
        let p = GlobalLock(h) as *const u16;
        if !p.is_null() {
            let max = GlobalSize(h) / 2;
            let s = std::slice::from_raw_parts(p, max);
            let len = s.iter().position(|&c| c == 0).unwrap_or(max);
            text = Some(String::from_utf16_lossy(&s[..len]));
            GlobalUnlock(h);
        }
    }
    text
}
