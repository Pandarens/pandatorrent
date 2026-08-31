//! Turning the computer off when there is nothing left to wait for.
//!
//! Reached only from a countdown the user could have cancelled, and only when
//! they switched it on themselves. Nothing here ever runs by default.

use std::sync::Arc;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Seconds Windows itself waits, on top of the countdown already shown.
///
/// A last chance at the operating-system level, and enough time for the engine
/// to finish closing its files.
const OS_GRACE: &str = "5";

/// Shuts the machine down, after stopping playback and the torrent engine.
#[tauri::command]
pub async fn system_shutdown(state: State<'_, Arc<AppState>>) -> AppResult<()> {
    tracing::info!("выключение компьютера по завершении");

    // Close our own files first: the engine is writing real data, and letting
    // Windows pull the floor out mid-write is how a download gets corrupted.
    state.player.stop();
    state.engine.shutdown().await;

    let mut command = std::process::Command::new("shutdown");
    command.args(["/s", "/t", OS_GRACE, "/c", "Panda Torrent: работа завершена"]);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this a console window flashes up over whatever is on screen.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .map_err(|e| AppError::msg(format!("не удалось выключить компьютер: {e}")))?;
    Ok(())
}

/// Calls off a shutdown Windows has already been told about.
#[tauri::command]
pub async fn system_shutdown_cancel() -> AppResult<()> {
    let mut command = std::process::Command::new("shutdown");
    command.arg("/a");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    // A failure here usually means there was nothing to call off, which is
    // the same outcome the caller wanted.
    let _ = command.spawn();
    Ok(())
}
