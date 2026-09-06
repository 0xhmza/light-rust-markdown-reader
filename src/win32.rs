//! The small Win32 ABI surface used by the viewer. No generated bindings or
//! crates, and no unused declarations: the dead-code lint is left on.
#![allow(non_snake_case)]
use std::ffi::c_void;
pub type Handle = *mut c_void;
pub type Hwnd = Handle;
pub type Hdc = Handle;
pub type Lresult = isize;
pub type Wparam = usize;
pub type Lparam = isize;
pub type Wndproc = unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult;
pub const NULL: Handle = std::ptr::null_mut();

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[repr(C)]
#[derive(Default)]
pub struct Size {
    pub cx: i32,
    pub cy: i32,
}
#[repr(C)]
pub struct WndClass {
    pub style: u32,
    pub proc: Option<Wndproc>,
    pub class_extra: i32,
    pub window_extra: i32,
    pub instance: Handle,
    pub icon: Handle,
    pub cursor: Handle,
    pub background: Handle,
    pub menu_name: *const u16,
    pub class_name: *const u16,
}
#[repr(C)]
#[derive(Default)]
pub struct Msg {
    pub hwnd: Hwnd,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub point: Point,
    pub private: u32,
}
#[repr(C)]
pub struct PaintStruct {
    pub dc: Hdc,
    pub erase: i32,
    pub rect: Rect,
    pub restore: i32,
    pub inc_update: i32,
    pub reserved: [u8; 32],
}
#[repr(C)]
pub struct ScrollInfo {
    pub size: u32,
    pub mask: u32,
    pub min: i32,
    pub max: i32,
    pub page: u32,
    pub pos: i32,
    pub track: i32,
}
impl ScrollInfo {
    pub fn new(mask: u32) -> Self {
        Self {
            size: std::mem::size_of::<Self>() as u32,
            mask,
            min: 0,
            max: 0,
            page: 0,
            pos: 0,
            track: 0,
        }
    }
}
#[repr(C)]
pub struct OpenFileName {
    pub size: u32,
    pub owner: Hwnd,
    pub instance: Handle,
    pub filter: *const u16,
    pub custom_filter: *mut u16,
    pub max_custom_filter: u32,
    pub filter_index: u32,
    pub file: *mut u16,
    pub max_file: u32,
    pub file_title: *mut u16,
    pub max_file_title: u32,
    pub initial_dir: *const u16,
    pub title: *const u16,
    pub flags: u32,
    pub file_offset: u16,
    pub file_extension: u16,
    pub default_extension: *const u16,
    pub custom_data: isize,
    pub hook: *const c_void,
    pub template_name: *const u16,
    pub reserved: *mut c_void,
    pub reserved2: u32,
    pub flags_ex: u32,
}
#[repr(C)]
#[derive(Default)]
pub struct TextMetric {
    pub height: i32,
    pub ascent: i32,
    pub descent: i32,
    pub internal_leading: i32,
    pub external_leading: i32,
    pub average_width: i32,
    pub max_width: i32,
    pub weight: i32,
    pub overhang: i32,
    pub digitized_aspect_x: i32,
    pub digitized_aspect_y: i32,
    pub first: u16,
    pub last: u16,
    pub default_char: u16,
    pub break_char: u16,
    pub italic: u8,
    pub underlined: u8,
    pub struck_out: u8,
    pub pitch_family: u8,
    pub charset: u8,
}

/// Owner-draw payloads. Menus and the settings swatches both use them.
#[repr(C)]
pub struct DrawItem {
    pub kind: u32,
    pub id: u32,
    pub item: u32,
    pub action: u32,
    pub state: u32,
    pub window: Hwnd,
    pub dc: Hdc,
    pub rect: Rect,
    pub data: usize,
}
#[repr(C)]
pub struct MeasureItem {
    pub kind: u32,
    pub id: u32,
    pub item: u32,
    pub width: u32,
    pub height: u32,
    pub data: usize,
}
#[repr(C)]
#[derive(Default)]
pub struct MenuInfo {
    pub size: u32,
    pub mask: u32,
    pub style: u32,
    pub max_height: u32,
    pub back: Handle,
    pub context_help: u32,
    pub menu_data: usize,
}
#[repr(C)]
pub struct TrackMouse {
    pub size: u32,
    pub flags: u32,
    pub window: Hwnd,
    pub time: u32,
}
#[repr(C)]
pub struct ChooseColor {
    pub size: u32,
    pub owner: Hwnd,
    pub instance: Handle,
    pub result: u32,
    pub custom: *mut u32,
    pub flags: u32,
    pub data: isize,
    pub hook: *const c_void,
    pub template_name: *const u16,
}

pub const WM_DESTROY: u32 = 0x2;
pub const WM_SIZE: u32 = 0x5;
pub const WM_PAINT: u32 = 0xf;
pub const WM_ERASEBKGND: u32 = 0x14;
pub const WM_KEYDOWN: u32 = 0x100;
pub const WM_COMMAND: u32 = 0x111;
pub const WM_MOUSEWHEEL: u32 = 0x20a;
pub const WM_DROPFILES: u32 = 0x233;
pub const WM_DPICHANGED: u32 = 0x2e0;
pub const WM_CLOSE: u32 = 0x10;
pub const WM_SETICON: u32 = 0x80;
pub const WM_SETCURSOR: u32 = 0x20;
pub const WM_DRAWITEM: u32 = 0x2b;
pub const WM_MEASUREITEM: u32 = 0x2c;
pub const WM_TIMER: u32 = 0x113;
pub const WM_MOUSEMOVE: u32 = 0x200;
pub const WM_LBUTTONDOWN: u32 = 0x201;
pub const WM_LBUTTONUP: u32 = 0x202;
pub const WM_APP_LAYOUT: u32 = 0x8001;

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassW(class: *const WndClass) -> u16;
    pub fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: Handle,
        instance: Handle,
        param: *mut c_void,
    ) -> Hwnd;
    pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
    pub fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    pub fn TranslateMessage(msg: *const Msg) -> i32;
    pub fn DispatchMessageW(msg: *const Msg) -> isize;
    pub fn PostQuitMessage(code: i32);
    pub fn PostMessageW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> i32;
    pub fn ShowWindow(hwnd: Hwnd, show: i32) -> i32;
    pub fn UpdateWindow(hwnd: Hwnd) -> i32;
    pub fn DestroyWindow(hwnd: Hwnd) -> i32;
    pub fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    pub fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
    pub fn LoadImageW(
        instance: Handle,
        name: *const u16,
        kind: u32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> Handle;
    pub fn GetSystemMetrics(index: i32) -> i32;
    pub fn TrackMouseEvent(track: *mut TrackMouse) -> i32;
    pub fn GetParent(hwnd: Hwnd) -> Hwnd;
    pub fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    pub fn GetDC(hwnd: Hwnd) -> Hdc;
    pub fn ReleaseDC(hwnd: Hwnd, dc: Hdc) -> i32;
    pub fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Hdc;
    pub fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
    pub fn FillRect(dc: Hdc, rect: *const Rect, brush: Handle) -> i32;
    pub fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
    pub fn SetScrollInfo(hwnd: Hwnd, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
    #[cfg(test)]
    pub fn GetScrollInfo(hwnd: Hwnd, bar: i32, info: *mut ScrollInfo) -> i32;
    pub fn ScrollWindowEx(
        hwnd: Hwnd,
        dx: i32,
        dy: i32,
        scroll: *const Rect,
        clip: *const Rect,
        region: Handle,
        update: *mut Rect,
        flags: u32,
    ) -> i32;
    pub fn GetKeyState(key: i32) -> i16;
    pub fn SystemParametersInfoW(action: u32, param: u32, value: *mut c_void, flags: u32) -> i32;
    pub fn SetWindowTextW(hwnd: Hwnd, title: *const u16) -> i32;
    pub fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    pub fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    pub fn SetWindowPos(
        hwnd: Hwnd,
        after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    pub fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
    pub fn CreateMenu() -> Handle;
    pub fn CreatePopupMenu() -> Handle;
    pub fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
    pub fn CheckMenuRadioItem(menu: Handle, first: u32, last: u32, check: u32, flags: u32) -> i32;
    pub fn SetMenu(hwnd: Hwnd, menu: Handle) -> i32;
    pub fn SetProcessDpiAwarenessContext(context: Handle) -> i32;
    pub fn SendMessageW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
    pub fn EnableWindow(hwnd: Hwnd, enable: i32) -> i32;
    pub fn IsWindow(hwnd: Hwnd) -> i32;
    pub fn IsDialogMessageW(hwnd: Hwnd, msg: *const Msg) -> i32;
    pub fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    pub fn AdjustWindowRect(rect: *mut Rect, style: u32, menu: i32) -> i32;
    pub fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    pub fn GetDlgItem(hwnd: Hwnd, id: i32) -> Hwnd;
    pub fn GetMenu(hwnd: Hwnd) -> Handle;
    pub fn SetMenuInfo(menu: Handle, info: *const MenuInfo) -> i32;
    pub fn DrawMenuBar(hwnd: Hwnd) -> i32;
    pub fn DestroyMenu(menu: Handle) -> i32;
    pub fn DrawTextW(dc: Hdc, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
    pub fn FrameRect(dc: Hdc, rect: *const Rect, brush: Handle) -> i32;
    pub fn SetCursor(cursor: Handle) -> Handle;
    pub fn GetCursorPos(point: *mut Point) -> i32;
    pub fn ScreenToClient(hwnd: Hwnd, point: *mut Point) -> i32;
    pub fn SetCapture(hwnd: Hwnd) -> Hwnd;
    pub fn ShowScrollBar(hwnd: Hwnd, bar: i32, show: i32) -> i32;
    pub fn ReleaseCapture() -> i32;
    pub fn SetTimer(hwnd: Hwnd, id: usize, period: u32, proc: *const c_void) -> usize;
    pub fn KillTimer(hwnd: Hwnd, id: usize) -> i32;
    pub fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, count: i32) -> i32;
    #[cfg(test)]
    pub fn PrintWindow(hwnd: Hwnd, dc: Hdc, flags: u32) -> i32;
    pub fn GetDpiForWindow(hwnd: Hwnd) -> u32;
}
#[link(name = "gdi32")]
extern "system" {
    pub fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        output: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> Handle;
    pub fn DeleteObject(object: Handle) -> i32;
    pub fn SelectObject(dc: Hdc, object: Handle) -> Handle;
    pub fn GetTextMetricsW(dc: Hdc, metrics: *mut TextMetric) -> i32;
    pub fn GetTextExtentPoint32W(dc: Hdc, text: *const u16, count: i32, size: *mut Size) -> i32;
    pub fn GetTextExtentExPointW(
        dc: Hdc,
        text: *const u16,
        count: i32,
        max: i32,
        fit: *mut i32,
        advances: *mut i32,
        size: *mut Size,
    ) -> i32;
    pub fn SetBkMode(dc: Hdc, mode: i32) -> i32;
    pub fn SetTextColor(dc: Hdc, color: u32) -> u32;
    pub fn TextOutW(dc: Hdc, x: i32, y: i32, text: *const u16, len: i32) -> i32;
    pub fn CreateSolidBrush(color: u32) -> Handle;
    pub fn GetStockObject(object: i32) -> Handle;
    pub fn SetDCBrushColor(dc: Hdc, color: u32) -> u32;
    pub fn CreateCompatibleDC(dc: Hdc) -> Hdc;
    pub fn DeleteDC(dc: Hdc) -> i32;
    pub fn CreateDIBSection(
        dc: Hdc,
        header: *const u32,
        usage: u32,
        bits: *mut *mut u32,
        section: Handle,
        offset: u32,
    ) -> Handle;
    pub fn BitBlt(
        dest: Hdc,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        source: Hdc,
        sx: i32,
        sy: i32,
        operation: u32,
    ) -> i32;
    #[cfg(test)]
    pub fn GetPixel(dc: Hdc, x: i32, y: i32) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    pub fn GetStringTypeW(kind: u32, text: *const u16, count: i32, properties: *mut u16) -> i32;
    pub fn GetModuleHandleW(name: *const u16) -> Handle;
    pub fn WritePrivateProfileStringW(
        section: *const u16,
        key: *const u16,
        value: *const u16,
        file: *const u16,
    ) -> i32;
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(
        module: Handle,
        name: *const u8,
    ) -> Option<unsafe extern "system" fn() -> isize>;
}
#[link(name = "comdlg32")]
extern "system" {
    pub fn GetOpenFileNameW(info: *mut OpenFileName) -> i32;
    pub fn ChooseColorW(info: *mut ChooseColor) -> i32;
}
#[link(name = "shell32")]
extern "system" {
    pub fn DragAcceptFiles(hwnd: Hwnd, accept: i32);
    pub fn DragQueryFileW(drop: Handle, index: u32, file: *mut u16, length: u32) -> u32;
    pub fn DragFinish(drop: Handle);
}
pub unsafe fn DwmSetWindowAttribute(
    hwnd: Hwnd,
    attribute: u32,
    value: *const c_void,
    size: u32,
) -> i32 {
    type SetAttribute = unsafe extern "system" fn(Hwnd, u32, *const c_void, u32) -> i32;
    static FUNCTION: std::sync::OnceLock<Option<SetAttribute>> = std::sync::OnceLock::new();
    // Caption theming is optional. Load only the system DLL and retain it for
    // the process lifetime, avoiding a DWM import-library build dependency.
    let function = FUNCTION.get_or_init(|| {
        let module = LoadLibraryExW(wide("dwmapi.dll").as_ptr(), NULL, 0x800);
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, c"DwmSetWindowAttribute".as_ptr().cast()).map(|function| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, SetAttribute>(function)
        })
    });
    function.map_or(0x80004001u32 as i32, |function| {
        function(hwnd, attribute, value, size)
    })
}

/// Ask Windows for a finer timer tick while an animation runs, and give it
/// back afterwards. Unavailable multimedia timers simply leave the default.
pub unsafe fn frame_clock(fast: bool) {
    type Period = unsafe extern "system" fn(u32) -> u32;
    static FUNCTIONS: std::sync::OnceLock<Option<(Period, Period)>> = std::sync::OnceLock::new();
    let functions = FUNCTIONS.get_or_init(|| {
        let load = |name: &std::ffi::CStr| {
            system_function("winmm.dll", name).map(|function| {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Period>(function)
            })
        };
        Some((load(c"timeBeginPeriod")?, load(c"timeEndPeriod")?))
    });
    if let Some((begin, end)) = functions {
        if fast {
            begin(1);
        } else {
            end(1);
        }
    }
}

/// An off-screen 32-bit surface. The viewer composes each frame in one so
/// nothing is ever seen half-painted, and the tests draw into one to read the
/// pixels back.
pub struct Buffer {
    pub dc: Hdc,
    pub width: i32,
    pub height: i32,
    bitmap: Handle,
    previous: Handle,
    /// Read back by the tests through `pixels`.
    #[cfg_attr(not(test), allow(dead_code))]
    bits: *mut u32,
}
impl Buffer {
    /// `compatible` may be NULL for a surface that matches the screen.
    pub unsafe fn new(compatible: Hdc, width: i32, height: i32) -> Option<Buffer> {
        let dc = CreateCompatibleDC(compatible);
        if dc.is_null() {
            return None;
        }
        // Top-down, 32 bits, no compression: what Direct2D binds to most
        // happily, and what lets `pixels` be a plain slice.
        let header = [
            40u32,
            width as u32,
            (-height) as u32,
            1 | (32 << 16),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, header.as_ptr(), 0, &mut bits, NULL, 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            return None;
        }
        Some(Buffer {
            previous: SelectObject(dc, bitmap),
            dc,
            bitmap,
            bits,
            width,
            height,
        })
    }
    /// The surface as `width * height` pixels, top row first, `0x00RRGGBB`.
    #[cfg(test)]
    pub unsafe fn pixels(&self) -> &[u32] {
        std::slice::from_raw_parts(self.bits, (self.width * self.height).max(0) as usize)
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// Load from System32 only. Modules remain loaded until process exit.
pub unsafe fn system_function(
    library: &str,
    name: &std::ffi::CStr,
) -> Option<unsafe extern "system" fn() -> isize> {
    let module = LoadLibraryExW(wide(library).as_ptr(), NULL, 0x800);
    if module.is_null() {
        None
    } else {
        GetProcAddress(module, name.as_ptr().cast())
    }
}
