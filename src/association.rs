//! Per-user registration of MDLite as a Markdown reader. Windows keeps the final
//! say over which program is the default: this makes MDLite a choice it can
//! offer, and the default outright only for an extension nobody has claimed.
//! The installer writes the same keys, so the two always agree.
use crate::win32::{wide, Handle, Hwnd, NULL};
use std::{io, path::Path, ptr};

/// The extensions the open dialog already offers, minus plain text.
const EXTENSIONS: [&str; 4] = [".md", ".markdown", ".mdown", ".mkd"];
const PROG_ID: &str = "MDLite.Markdown";

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        root: Handle,
        path: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        access: u32,
        security: *const u8,
        key: *mut Handle,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        key: Handle,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        bytes: u32,
    ) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
    fn RegDeleteTreeW(root: Handle, path: *const u16) -> i32;
    fn RegDeleteKeyValueW(root: Handle, path: *const u16, name: *const u16) -> i32;
    fn RegGetValueW(
        root: Handle,
        path: *const u16,
        name: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u16,
        bytes: *mut u32,
    ) -> i32;
}
#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        owner: Hwnd,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> Handle;
    fn SHChangeNotify(event: i32, flags: u32, item1: *const u8, item2: *const u8);
}

const HKEY_CLASSES_ROOT: Handle = 0x8000_0000u32 as i32 as isize as Handle;
const HKEY_CURRENT_USER: Handle = 0x8000_0001u32 as i32 as isize as Handle;
const NOT_FOUND: i32 = 2;

/// `"C:\path\mdlite.exe" "%1"`, the way Windows wants a shell command.
fn command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

fn failed(status: i32) -> io::Error {
    io::Error::from_raw_os_error(status)
}

/// Set one string value under HKEY_CURRENT_USER, making the key if need be. An
/// empty `name` is the key's default value.
unsafe fn write(path: &str, name: &str, value: &str) -> io::Result<()> {
    let mut key = NULL;
    let status = RegCreateKeyExW(
        HKEY_CURRENT_USER,
        wide(path).as_ptr(),
        0,
        ptr::null_mut(),
        0,
        0x2, // KEY_SET_VALUE.
        ptr::null(),
        &mut key,
        ptr::null_mut(),
    );
    if status != 0 {
        return Err(failed(status));
    }
    let data = wide(value);
    let status = RegSetValueExW(
        key,
        wide(name).as_ptr(),
        0,
        1, // REG_SZ.
        data.as_ptr().cast(),
        std::mem::size_of_val(&data[..]) as u32,
    );
    RegCloseKey(key);
    if status == 0 {
        Ok(())
    } else {
        Err(failed(status))
    }
}

/// The default value of a key, or `None` when it is missing or empty.
unsafe fn read_default(root: Handle, path: &str) -> Option<String> {
    let mut buffer = [0u16; 260];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    let status = RegGetValueW(
        root,
        wide(path).as_ptr(),
        ptr::null(),
        0x2, // RRF_RT_REG_SZ.
        ptr::null_mut(),
        buffer.as_mut_ptr(),
        &mut bytes,
    );
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(0);
    (status == 0 && length > 0).then(|| String::from_utf16_lossy(&buffer[..length]))
}

/// Delete a tree or a value, treating "it was not there" as success.
fn gone(status: i32) -> io::Result<()> {
    if matches!(status, 0 | NOT_FOUND) {
        Ok(())
    } else {
        Err(failed(status))
    }
}

/// Register MDLite under `{software}` in HKEY_CURRENT_USER, which is
/// `Software` for real and something else in the tests. `claim` also makes MDLite
/// the default for any extension that has no program at all, which is the only
/// case Windows lets a program decide for itself.
unsafe fn register(software: &str, exe: &Path, claim: bool) -> io::Result<()> {
    let classes = format!("{software}\\Classes");
    let capabilities = format!("{software}\\MDLite\\Capabilities");
    let program = format!("{classes}\\{PROG_ID}");
    write(&program, "", "Markdown document")?;
    write(
        &format!("{program}\\DefaultIcon"),
        "",
        &format!("\"{}\",0", exe.display()),
    )?;
    write(
        &format!("{program}\\shell\\open\\command"),
        "",
        &command(exe),
    )?;
    // What "Open with" lists for any file, whatever has been registered above.
    let application = format!("{classes}\\Applications\\mdlite.exe");
    write(&application, "FriendlyAppName", "MDLite")?;
    write(
        &format!("{application}\\shell\\open\\command"),
        "",
        &command(exe),
    )?;
    write(&capabilities, "ApplicationName", "MDLite")?;
    write(
        &capabilities,
        "ApplicationDescription",
        "Lightweight Markdown reader",
    )?;
    for extension in EXTENSIONS {
        write(
            &format!("{capabilities}\\FileAssociations"),
            extension,
            PROG_ID,
        )?;
        write(&format!("{application}\\SupportedTypes"), extension, "")?;
        // An empty value offers MDLite in "Open with" without claiming the type.
        write(
            &format!("{classes}\\{extension}\\OpenWithProgids"),
            PROG_ID,
            "",
        )?;
        if claim && read_default(HKEY_CLASSES_ROOT, extension).is_none() {
            write(&format!("{classes}\\{extension}"), "", PROG_ID)?;
        }
    }
    write(
        &format!("{software}\\RegisteredApplications"),
        "MDLite",
        &capabilities,
    )
}

/// Take back everything `register` wrote, and nothing it did not. An extension
/// is left alone unless MDLite is what it opens with.
unsafe fn unregister_from(software: &str) -> io::Result<()> {
    let classes = format!("{software}\\Classes");
    for tree in [
        format!("{classes}\\{PROG_ID}"),
        format!("{classes}\\Applications\\mdlite.exe"),
        format!("{software}\\MDLite"),
    ] {
        gone(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&tree).as_ptr()))?;
    }
    let value = |path: &str, name: &str| {
        gone(RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            wide(path).as_ptr(),
            wide(name).as_ptr(),
        ))
    };
    value(&format!("{software}\\RegisteredApplications"), "MDLite")?;
    for extension in EXTENSIONS {
        let key = format!("{classes}\\{extension}");
        value(&format!("{key}\\OpenWithProgids"), PROG_ID)?;
        if read_default(HKEY_CURRENT_USER, &key).as_deref() == Some(PROG_ID) {
            value(&key, "")?;
        }
    }
    Ok(())
}

/// Remove MDLite's registration for the current user. `mdlite.exe --unregister`,
/// which the uninstaller runs.
pub unsafe fn unregister() {
    let _ = unregister_from("Software");
    SHChangeNotify(0x0800_0000, 0, ptr::null(), ptr::null());
}

/// Only called by the explicit settings button; never during startup or tests.
pub unsafe fn choose_default(owner: Hwnd) -> io::Result<()> {
    register("Software", &std::env::current_exe()?, true)?;
    // SHCNE_ASSOCCHANGED: tell Explorer to look again.
    SHChangeNotify(0x0800_0000, 0, ptr::null(), ptr::null());
    let result = ShellExecuteW(
        owner,
        wide("open").as_ptr(),
        wide("ms-settings:defaultapps?registeredAppUser=MDLite").as_ptr(),
        ptr::null(),
        ptr::null(),
        1,
    ) as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(io::Error::other(
            "MDLite is registered. Open Windows Settings > Apps > Default apps and choose MDLite for .md.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[test]
    fn open_command_quotes_both_paths() {
        assert_eq!(
            command(Path::new(r"C:\My Apps\日本語\mdlite.exe")),
            "\"C:\\My Apps\\日本語\\mdlite.exe\" \"%1\""
        );
        // The wide path that goes to the registry keeps non-ASCII intact.
        let exe = Path::new(r"C:\Apps\日本語\mdlite.exe");
        assert_eq!(
            exe.as_os_str().encode_wide().count(),
            exe.to_string_lossy().encode_utf16().count()
        );
    }
    /// Register into, and take back out of, a registry root made for the test, so
    /// the machine's real file associations are never touched.
    #[test]
    fn registering_and_unregistering_leave_nothing_behind() {
        unsafe {
            let root = format!("Software\\MDLiteTest-{}", std::process::id());
            let exe = Path::new(r"C:\Program Files\MDLite\mdlite.exe");
            register(&root, exe, false).unwrap();
            let classes = format!("{root}\\Classes");
            let open = read_default(
                HKEY_CURRENT_USER,
                &format!("{classes}\\{PROG_ID}\\shell\\open\\command"),
            );
            assert_eq!(open.as_deref(), Some(command(exe).as_str()));
            let icon = read_default(
                HKEY_CURRENT_USER,
                &format!("{classes}\\{PROG_ID}\\DefaultIcon"),
            );
            assert_eq!(
                icon.as_deref(),
                Some("\"C:\\Program Files\\MDLite\\mdlite.exe\",0")
            );
            // Registering is not claiming: no extension is made to open with MDLite.
            assert_eq!(
                read_default(HKEY_CURRENT_USER, &format!("{classes}\\.md")),
                None
            );
            // An extension that opens with something else is left to it.
            write(&format!("{classes}\\.mkd"), "", "SomethingElse.mkd").unwrap();
            // One MDLite owns is let go of.
            write(&format!("{classes}\\.md"), "", PROG_ID).unwrap();
            unregister_from(&root).unwrap();
            assert_eq!(
                read_default(HKEY_CURRENT_USER, &format!("{classes}\\.md")),
                None
            );
            assert_eq!(
                read_default(HKEY_CURRENT_USER, &format!("{classes}\\.mkd")).as_deref(),
                Some("SomethingElse.mkd")
            );
            assert_eq!(
                read_default(HKEY_CURRENT_USER, &format!("{classes}\\{PROG_ID}")),
                None
            );
            // Doing it twice is fine, which is what an uninstaller needs.
            unregister_from(&root).unwrap();
            gone(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&root).as_ptr())).unwrap();
        }
    }
}
