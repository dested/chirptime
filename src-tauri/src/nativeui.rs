//! The three bits of Windows shell the output-folder UI needs: a folder
//! picker, "open this in Explorer", and how much room is left on the volume.
//!
//! None of this touches ffmpeg, so it stays out of `capture.rs`.

use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Windows dislikes the `\\?\` prefix `canonicalize` produces, and Explorer
/// dislikes forward slashes. Both show up in paths that came from a config file.
pub fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('/', "\\");
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// Free and total bytes on the volume holding `path`.
///
/// Walks up to the nearest ancestor that exists, so the number is right for a
/// folder the user has chosen but nothing has created yet — which is exactly
/// the state the first-run panel shows it in.
#[cfg(target_os = "windows")]
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut probe: Option<&Path> = Some(path);
    while let Some(p) = probe {
        if p.exists() {
            let wide = HSTRING::from(p.as_os_str());
            let mut free: u64 = 0;
            let mut total: u64 = 0;
            let ok = unsafe {
                GetDiskFreeSpaceExW(&wide, Some(&mut free), Some(&mut total), None).is_ok()
            };
            return ok.then_some((free, total));
        }
        probe = p.parent();
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// Open a folder in Explorer. Explorer routinely exits non-zero even when it
/// opened the window, so the exit status is deliberately not checked — only a
/// failure to launch it at all is an error.
#[cfg(target_os = "windows")]
pub fn open_in_explorer(path: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("explorer");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg(display_path(path));
    cmd.spawn().map(|_| ()).map_err(|e| format!("open Explorer: {e}"))
}

#[cfg(not(target_os = "windows"))]
pub fn open_in_explorer(_path: &Path) -> Result<(), String> {
    Err("Opening a folder is only implemented on Windows".to_string())
}

/// Native "choose a folder" dialog (`IFileOpenDialog` + `FOS_PICKFOLDERS`).
///
/// `Ok(None)` means the user cancelled — that is a normal outcome, not an error.
/// `parent_hwnd` should be the widget's window handle: without an owner the
/// dialog can open *behind* an always-on-top widget, which reads as the button
/// doing nothing.
///
/// Blocking and modal. Call it from a blocking thread, never the async runtime.
#[cfg(target_os = "windows")]
pub fn pick_folder(start_at: Option<&Path>, parent_hwnd: Option<isize>) -> Result<Option<String>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    /// See `sysaudio::ComApartment` — `CoUninitialize` must run after every
    /// interface pointer in the scope is released, so this is declared first and
    /// dropped last.
    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    // S_FALSE means this thread was already initialised, which is fine.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        return Err(format!("CoInitializeEx failed: {hr:?}"));
    }
    let _com = ComApartment;

    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("folder dialog unavailable: {e}"))?;

        let opts = dialog.GetOptions().map_err(|e| e.to_string())?;
        dialog
            .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .map_err(|e| e.to_string())?;

        // Start where the recordings currently go, if that exists. A missing
        // start folder is not worth failing over — the dialog just opens at its
        // own default.
        if let Some(start) = start_at.filter(|p| p.exists()) {
            let wide = windows::core::HSTRING::from(start.as_os_str());
            if let Ok(item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None)
            {
                let _ = dialog.SetFolder(&item);
            }
        }

        let owner = HWND(parent_hwnd.unwrap_or(0) as *mut core::ffi::c_void);
        if let Err(e) = dialog.Show(Some(owner)) {
            // HRESULT_FROM_WIN32(ERROR_CANCELLED): the user closed the dialog.
            if e.code().0 as u32 == 0x800704C7 {
                return Ok(None);
            }
            return Err(format!("folder dialog failed: {e}"));
        }

        let item = dialog.GetResult().map_err(|e| e.to_string())?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| e.to_string())?;
        let picked = raw.to_string().map_err(|e| e.to_string())?;
        CoTaskMemFree(Some(raw.0 as *const core::ffi::c_void));
        Ok(Some(picked))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn pick_folder(_start_at: Option<&Path>, _parent_hwnd: Option<isize>) -> Result<Option<String>, String> {
    Err("Choosing a folder is only implemented on Windows".to_string())
}

/// Absolute form of a configured output dir, without requiring it to exist.
pub fn absolute(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_normalizes_separators_and_prefix() {
        assert_eq!(display_path(Path::new("G:/code/screener/output")), r"G:\code\screener\output");
        assert_eq!(display_path(Path::new(r"\\?\C:\Users\x")), r"C:\Users\x");
    }

    #[test]
    fn disk_space_works_for_a_folder_that_does_not_exist_yet() {
        let missing = std::env::temp_dir().join("chirptime-does-not-exist").join("nested");
        let space = disk_space(&missing);
        #[cfg(target_os = "windows")]
        {
            let (free, total) = space.expect("temp volume should report space");
            assert!(total > 0 && free <= total);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = space;
    }
}
