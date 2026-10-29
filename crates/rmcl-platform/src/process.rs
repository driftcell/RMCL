//! Child process configuration for launcher-owned Java processes.

/// Suppresses the console window that Windows allocates for a console-subsystem
/// child process.
///
/// RMCL keeps the Windows console subsystem so `rmcl mcp --stdio` can use
/// inherited stdio, and GUI mode releases that console with `FreeConsole`.
/// A Java child spawned afterwards has no console to inherit, so Windows opens
/// a new console window unless this flag is set. The window does not close
/// until the child exits, which makes it look like the game opened a terminal.
///
/// The method is a no-op on non-Windows platforms.
pub trait HideConsoleWindow {
    fn hide_console_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
impl HideConsoleWindow for std::process::Command {
    fn hide_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt as _;

        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(windows)]
impl HideConsoleWindow for tokio::process::Command {
    fn hide_console_window(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl HideConsoleWindow for std::process::Command {
    fn hide_console_window(&mut self) -> &mut Self {
        self
    }
}

#[cfg(not(windows))]
impl HideConsoleWindow for tokio::process::Command {
    fn hide_console_window(&mut self) -> &mut Self {
        self
    }
}
