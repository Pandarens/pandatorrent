//! Minimal binding to libmpv's C client API.
//!
//! The library is loaded at runtime rather than linked at build time. That
//! keeps the build free of an mpv SDK, lets the app start and explain itself
//! when the DLL is missing instead of refusing to launch, and makes shipping
//! the DLL beside the binary the only deployment step.
//!
//! Only the handful of entry points the player actually needs are declared.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};

use crate::error::{AppError, AppResult};

/// File names shipped by the official mpv Windows builds, newest first.
const LIBRARY_NAMES: &[&str] = &["libmpv-2.dll", "mpv-2.dll", "libmpv.dll"];

type MpvHandle = *mut c_void;

type FnCreate = unsafe extern "C" fn() -> MpvHandle;
type FnInitialize = unsafe extern "C" fn(MpvHandle) -> c_int;
type FnTerminate = unsafe extern "C" fn(MpvHandle);
type FnSetOption = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnCommand = unsafe extern "C" fn(MpvHandle, *mut *const c_char) -> c_int;
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnGetPropertyString = unsafe extern "C" fn(MpvHandle, *const c_char) -> *mut c_char;
type FnFree = unsafe extern "C" fn(*mut c_void);

/// A loaded libmpv plus one player instance.
///
/// Not `Sync` on its own — callers keep it behind a mutex, which also
/// serialises the "configure then initialize" ordering mpv requires.
pub struct Mpv {
    // Dropped last: the function pointers below borrow from it conceptually,
    // so the library must outlive every call made through them.
    _library: Library,
    ctx: MpvHandle,
    initialized: bool,

    create: FnCreate,
    initialize: FnInitialize,
    terminate: FnTerminate,
    set_option: FnSetOption,
    set_property: FnSetProperty,
    command: FnCommand,
    error_string: FnErrorString,
    get_property_string: FnGetPropertyString,
    free: FnFree,
}

// The mpv client API is documented as thread-safe for a single handle, and the
// handle is only ever reached through a mutex in `super::Player`.
unsafe impl Send for Mpv {}

impl Mpv {
    /// Finds and loads libmpv, then creates an uninitialised player.
    ///
    /// `search_dirs` are tried in order before falling back to the system
    /// loader path.
    pub fn load(search_dirs: &[PathBuf]) -> AppResult<Self> {
        let library = load_library(search_dirs)?;

        // SAFETY: the names below are the documented libmpv C entry points, and
        // the signatures match `client.h`. A missing symbol surfaces as an error
        // rather than undefined behaviour.
        unsafe {
            let create: Symbol<FnCreate> = sym(&library, b"mpv_create\0")?;
            let initialize: Symbol<FnInitialize> = sym(&library, b"mpv_initialize\0")?;
            let terminate: Symbol<FnTerminate> = sym(&library, b"mpv_terminate_destroy\0")?;
            let set_option: Symbol<FnSetOption> = sym(&library, b"mpv_set_option_string\0")?;
            let set_property: Symbol<FnSetProperty> = sym(&library, b"mpv_set_property_string\0")?;
            let command: Symbol<FnCommand> = sym(&library, b"mpv_command\0")?;
            let error_string: FnErrorString = *sym(&library, b"mpv_error_string\0")?;
            let get_property_string: FnGetPropertyString =
                *sym(&library, b"mpv_get_property_string\0")?;
            let free: FnFree = *sym(&library, b"mpv_free\0")?;

            let (create, initialize, terminate, set_option, set_property, command) = (
                *create,
                *initialize,
                *terminate,
                *set_option,
                *set_property,
                *command,
            );

            let ctx = create();
            if ctx.is_null() {
                return Err(AppError::msg("mpv не удалось создать проигрыватель"));
            }

            Ok(Self {
                _library: library,
                ctx,
                initialized: false,
                create,
                initialize,
                terminate,
                set_option,
                set_property,
                command,
                error_string,
                get_property_string,
                free,
            })
        }
    }

    /// Sets an option. Must happen before [`Mpv::init`] for options mpv only
    /// reads at startup, such as `wid`.
    pub fn set_option(&self, name: &str, value: &str) -> AppResult<()> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: both pointers are valid NUL-terminated strings for the call.
        let code = unsafe { (self.set_option)(self.ctx, n.as_ptr(), v.as_ptr()) };
        self.check(code, &format!("установка параметра {name}"))
    }

    /// Changes a property on a running player, e.g. volume or an audio filter.
    pub fn set_property(&self, name: &str, value: &str) -> AppResult<()> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: as above.
        let code = unsafe { (self.set_property)(self.ctx, n.as_ptr(), v.as_ptr()) };
        self.check(code, &format!("установка свойства {name}"))
    }

    pub fn init(&mut self) -> AppResult<()> {
        // SAFETY: `ctx` came from `mpv_create` and has not been destroyed.
        let code = unsafe { (self.initialize)(self.ctx) };
        self.check(code, "инициализация mpv")?;
        self.initialized = true;
        Ok(())
    }

    /// Runs an mpv command, e.g. `["loadfile", url, "replace"]`.
    ///
    /// Passing the arguments as an array avoids the quoting problems of the
    /// string command form — stream URLs carry `?` and `&`.
    pub fn command(&self, args: &[&str]) -> AppResult<()> {
        let owned: Vec<CString> = args
            .iter()
            .map(|a| cstr(a))
            .collect::<AppResult<Vec<_>>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());

        // SAFETY: `ptrs` is a NULL-terminated array of valid C strings that
        // outlives the call, which is exactly what mpv_command expects.
        let code = unsafe { (self.command)(self.ctx, ptrs.as_mut_ptr()) };
        self.check(code, &format!("команда mpv {:?}", args.first().unwrap_or(&"")))
    }

    /// Reads a property as text, e.g. `time-pos` or `pause`.
    ///
    /// Returns `None` when mpv has no value for it — which is normal for
    /// playback properties before a file is loaded.
    pub fn get_property(&self, name: &str) -> Option<String> {
        let n = cstr(name).ok()?;
        // SAFETY: mpv allocates the returned string; it must be released with
        // mpv_free, which is what the guard below does.
        unsafe {
            let raw = (self.get_property_string)(self.ctx, n.as_ptr());
            if raw.is_null() {
                return None;
            }
            let value = std::ffi::CStr::from_ptr(raw).to_string_lossy().into_owned();
            (self.free)(raw as *mut c_void);
            Some(value)
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn check(&self, code: c_int, what: &str) -> AppResult<()> {
        if code >= 0 {
            return Ok(());
        }
        // SAFETY: mpv_error_string returns a static NUL-terminated string.
        let text = unsafe {
            let ptr = (self.error_string)(code);
            if ptr.is_null() {
                String::from("неизвестная ошибка")
            } else {
                std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        Err(AppError::msg(format!("{what}: {text}")))
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            // SAFETY: destroying a handle created by mpv_create, exactly once.
            unsafe { (self.terminate)(self.ctx) };
            self.ctx = std::ptr::null_mut();
        }
        // Silences "field is never read" for the constructor pointer, which is
        // kept so the struct documents the full set of entry points it uses.
        let _ = self.create;
    }
}

unsafe fn sym<'a, T>(library: &'a Library, name: &[u8]) -> AppResult<Symbol<'a, T>> {
    unsafe { library.get(name) }.map_err(|e| {
        AppError::msg(format!(
            "в libmpv нет функции {}: {e}",
            String::from_utf8_lossy(name).trim_end_matches('\0')
        ))
    })
}

/// The first library file that actually exists in one of `dirs`.
///
/// Split out from loading so it can be tested without depending on what
/// happens to be installed on the machine running the tests.
pub fn find_in_dirs(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|dir| LIBRARY_NAMES.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.exists())
}

fn missing_library_error(tried: &[String]) -> AppError {
    AppError::msg(format!(
        "не найдена библиотека mpv ({}). Положите её рядом с исполняемым файлом.{}",
        LIBRARY_NAMES.join(" / "),
        if tried.is_empty() {
            String::new()
        } else {
            format!(" Попытки: {}", tried.join("; "))
        }
    ))
}

fn load_library(search_dirs: &[PathBuf]) -> AppResult<Library> {
    let mut tried = Vec::new();

    if let Some(candidate) = find_in_dirs(search_dirs) {
        // SAFETY: loading a DLL runs its initialisation code; this is the mpv
        // library the app ships or the user installed.
        match unsafe { Library::new(&candidate) } {
            Ok(lib) => return Ok(lib),
            Err(e) => tried.push(format!("{}: {e}", candidate.display())),
        }
    }

    // Last resort: whatever the OS loader can find on PATH.
    for name in LIBRARY_NAMES {
        // SAFETY: as above.
        if let Ok(lib) = unsafe { Library::new(name) } {
            return Ok(lib);
        }
    }

    Err(missing_library_error(&tried))
}

fn cstr(s: &str) -> AppResult<CString> {
    CString::new(s).map_err(|_| AppError::msg("недопустимый параметр для mpv (нулевой байт)"))
}

/// Where to look for the library, in priority order.
pub fn search_dirs(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = resource_dir {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("mpv"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
            dirs.push(parent.join("mpv"));
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_found_in_an_empty_directory() {
        // Deliberately does not call `load_library`: that falls back to the OS
        // loader, so its result depends on whether mpv happens to be installed
        // on the machine running the tests.
        assert!(find_in_dirs(&[PathBuf::from("Z:/definitely/not/here")]).is_none());
    }

    #[test]
    fn the_missing_library_message_names_the_file() {
        let err = missing_library_error(&[]).to_string();
        assert!(
            err.contains("libmpv-2.dll"),
            "the user needs to know which file to supply: {err}"
        );
    }

    #[test]
    fn search_dirs_include_the_executable_folder() {
        let dirs = search_dirs(None);
        assert!(!dirs.is_empty(), "the exe folder should always be searched");
    }

    #[test]
    fn nul_bytes_are_rejected_rather_than_truncating() {
        assert!(cstr("a\0b").is_err());
    }
}
