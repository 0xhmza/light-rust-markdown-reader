#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod dwrite;
mod markdown;
#[cfg(target_os = "windows")]
mod render;
mod text;
mod theme;
#[cfg(target_os = "windows")]
mod viewer;
#[cfg(target_os = "windows")]
mod win32;

fn main() {
    #[cfg(target_os = "windows")]
    viewer::run();
    #[cfg(not(target_os = "windows"))]
    eprintln!("MDLite is a native Windows application.");
}
