#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod association;
#[cfg(target_os = "windows")]
mod config;
#[cfg(target_os = "windows")]
mod dwrite;
#[cfg(target_os = "windows")]
mod instance;
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
    {
        // The installer's two requests, made of the installed program itself.
        let flag = std::env::args_os().nth(1);
        match flag.as_deref().and_then(|flag| flag.to_str()) {
            Some("--unregister") => unsafe { association::unregister() },
            Some("--close") => unsafe { instance::close_others() },
            _ => viewer::run(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    eprintln!("MDLite is a native Windows application.");
}
