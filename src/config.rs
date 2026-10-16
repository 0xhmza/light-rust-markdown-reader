//! Where the settings file lives. Beside the program when it is carried
//! around, in the user's profile when it is installed: Program Files cannot be
//! written by the person using the program, and settings must be.
use std::path::{Path, PathBuf};

/// The file shipped with an install, copied into the profile on first run.
const TEMPLATE: &str = "mdlite.default.ini";

/// The INI to read and, from the settings window, write.
pub fn path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    // Tests must never read or seed the profile of whoever runs them.
    if cfg!(test) {
        return exe.with_file_name("mdlite.ini");
    }
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let path = resolve(&exe, appdata.as_deref());
    prepare(&path, &exe.with_file_name(TEMPLATE));
    path
}

/// An `mdlite.ini` beside the program means the reader keeps it portable and it
/// is used as it is. Otherwise settings belong to the user: `%APPDATA%\MDLite`.
fn resolve(exe: &Path, appdata: Option<&Path>) -> PathBuf {
    let beside = exe.with_file_name("mdlite.ini");
    match appdata {
        Some(_) if beside.is_file() => beside,
        Some(appdata) => appdata.join("MDLite").join("mdlite.ini"),
        None => beside,
    }
}

/// Make the folder, and on first run start from the shipped defaults so a new
/// install has every palette. Failure is harmless: the program runs without a
/// file, and says so when a setting cannot be saved.
fn prepare(path: &Path, template: &Path) {
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if !path.exists() {
        let _ = std::fs::copy(template, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("mdlite-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }
    #[test]
    fn an_ini_beside_the_program_keeps_it_portable() {
        let folder = scratch("portable");
        let exe = folder.join("mdlite.exe");
        std::fs::write(folder.join("mdlite.ini"), "[viewer]\n").unwrap();
        let profile = folder.join("profile");
        assert_eq!(resolve(&exe, Some(&profile)), folder.join("mdlite.ini"));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn an_installed_program_keeps_its_settings_in_the_profile() {
        let folder = scratch("installed");
        let exe = folder.join("mdlite.exe");
        let profile = folder.join("profile");
        let path = resolve(&exe, Some(&profile));
        assert_eq!(path, profile.join("MDLite").join("mdlite.ini"));
        // Only the template is beside the program, and it is never the live file.
        std::fs::write(folder.join(TEMPLATE), "[viewer]\ntheme=Nord\n").unwrap();
        assert_eq!(resolve(&exe, Some(&profile)), path);
        prepare(&path, &folder.join(TEMPLATE));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[viewer]\ntheme=Nord\n"
        );
        // Settings the reader has since saved are not overwritten by a new run.
        std::fs::write(&path, "[viewer]\ntheme=Light\n").unwrap();
        prepare(&path, &folder.join(TEMPLATE));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[viewer]\ntheme=Light\n"
        );
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn a_missing_template_or_profile_is_not_an_error() {
        let folder = scratch("missing");
        let exe = folder.join("mdlite.exe");
        // No profile to speak of: fall back to beside the program.
        assert_eq!(resolve(&exe, None), folder.join("mdlite.ini"));
        // No template: the folder is made and the program starts from built-in defaults.
        let path = folder.join("profile").join("MDLite").join("mdlite.ini");
        prepare(&path, &folder.join(TEMPLATE));
        assert!(path.parent().unwrap().is_dir() && !path.exists());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
