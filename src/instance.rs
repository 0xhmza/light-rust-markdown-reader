//! Other running copies of this very program. An installer cannot replace or
//! delete a file that is open, so it asks them to close first: `mdlite.exe --close`.
use crate::win32::*;
use std::path::PathBuf;

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(Hwnd, isize) -> i32, param: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, process: *mut u32) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: i32, process: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

/// The window class `viewer::run` registers for the main window.
const CLASS: &str = "MDLite.NativeReader";
const QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;

unsafe extern "system" fn collect(hwnd: Hwnd, found: isize) -> i32 {
    let mut name = [0u16; 64];
    let length = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32).max(0) as usize;
    if String::from_utf16_lossy(&name[..length]) == CLASS {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, &mut process);
        (*(found as *mut Vec<(usize, u32)>)).push((hwnd as usize, process));
    }
    1
}

/// The file a process was started from, if this account may ask.
unsafe fn image_of(process: Handle) -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    (QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) != 0)
        .then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Ask every other MDLite started from this same file to close, and give each a
/// few seconds to do it. Copies from anywhere else are left alone.
pub unsafe fn close_others() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut windows: Vec<(usize, u32)> = vec![];
    EnumWindows(collect, &mut windows as *mut _ as isize);
    let me = GetCurrentProcessId();
    for (hwnd, process) in windows {
        if process == me {
            continue;
        }
        let handle = OpenProcess(QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, process);
        if handle.is_null() {
            continue;
        }
        if image_of(handle).is_some_and(|image| same_file(&image, &exe)) {
            PostMessageW(hwnd as Hwnd, WM_CLOSE, 0, 0);
            WaitForSingleObject(handle, 5000);
        }
        CloseHandle(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_process_knows_the_file_it_was_started_from() {
        unsafe {
            let handle = OpenProcess(QUERY_LIMITED_INFORMATION, 0, GetCurrentProcessId());
            assert!(!handle.is_null());
            let image = image_of(handle).expect("the running program has an image");
            CloseHandle(handle);
            assert!(same_file(&image, &std::env::current_exe().unwrap()));
            assert!(!same_file(
                &image,
                std::path::Path::new(r"C:\elsewhere\mdlite.exe")
            ));
        }
    }
    #[test]
    fn closing_others_leaves_this_process_alone() {
        // No MDLite window exists in a test run; the call must simply return.
        unsafe { close_others() };
    }
}
