//! Windows notification-area control and close-to-tray policy.
//!
//! The tray is deliberately a platform edge. It turns native menu callbacks into bounded local
//! actions, but never reaches an application service, a database, or an egui viewport directly.
//! OM-299 owns the final command routing and the viewport-specific hide/restore operation.

use std::collections::VecDeque;

const WINDOWS_TRAY_ACTION_CAPACITY: usize = 16;

/// A local action emitted by the Windows tray or activation pipe.
///
/// These values intentionally do not claim a public application command contract. The primary
/// composition task maps them to accepted application commands and acknowledges their result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsPlatformAction {
    /// Ask the host to restore and activate the main window when Windows permits it.
    Open,
    /// Ask the host to pause tracking.
    PauseTracking,
    /// Ask the host to resume tracking.
    ResumeTracking,
    /// Ask the host to begin a focus session using its configured/default duration.
    StartFocusSession,
    /// Ask the host to begin coordinated explicit shutdown.
    Quit,
}

/// The host-side result required after a main-window close request.
///
/// Hiding is intentionally distinct from [`WindowsPlatformAction::Quit`]: close-to-tray keeps
/// tracking and the control loop alive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseToTrayDisposition {
    /// Cancel the viewport close and hide it while services continue running.
    HideToTray {
        /// Whether to show the one-time explanation that the application remains active.
        show_one_time_notice: bool,
    },
    /// Begin the host's coordinated explicit Quit lifecycle.
    BeginCoordinatedQuit,
}

/// Deterministic state shared by a native tray adapter and controlled fakes.
#[derive(Debug)]
pub struct WindowsTrayController {
    close_to_tray_enabled: bool,
    close_notice_shown: bool,
    pending_actions: VecDeque<WindowsPlatformAction>,
    dropped_action_count: u32,
}

impl Default for WindowsTrayController {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsTrayController {
    /// Creates the default Windows policy: close hides to the tray until the user changes it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            close_to_tray_enabled: true,
            close_notice_shown: false,
            pending_actions: VecDeque::with_capacity(WINDOWS_TRAY_ACTION_CAPACITY),
            dropped_action_count: 0,
        }
    }

    /// Sets the persisted close-to-tray preference supplied by the future settings composition.
    pub fn set_close_to_tray_enabled(&mut self, enabled: bool) {
        self.close_to_tray_enabled = enabled;
    }

    /// Returns the current close-to-tray preference.
    #[must_use]
    pub const fn close_to_tray_enabled(&self) -> bool {
        self.close_to_tray_enabled
    }

    /// Records a normal main-window close request.
    ///
    /// The background-tracking argument is owned by the host because this platform crate must
    /// not reach the tracking service. When tracking is active and close-to-tray is enabled, no
    /// quit action is queued and the one-time notice state advances exactly once.
    #[must_use]
    pub fn on_main_window_close(
        &mut self,
        background_tracking_enabled: bool,
    ) -> CloseToTrayDisposition {
        if self.close_to_tray_enabled && background_tracking_enabled {
            let show_one_time_notice = !self.close_notice_shown;
            self.close_notice_shown = true;
            return CloseToTrayDisposition::HideToTray {
                show_one_time_notice,
            };
        }

        CloseToTrayDisposition::BeginCoordinatedQuit
    }

    /// Records a user choosing an item from the notification-area menu.
    ///
    /// This is intentionally a queue rather than a direct service invocation so a Win32 callback
    /// cannot block on application state or storage.
    pub fn on_menu_action(&mut self, action: WindowsPlatformAction) {
        let queue_is_full = self.pending_actions.len() == WINDOWS_TRAY_ACTION_CAPACITY;
        let replaced_non_quit =
            queue_is_full && action == WindowsPlatformAction::Quit && self.remove_non_quit();
        if !queue_is_full || replaced_non_quit {
            self.pending_actions.push_back(action);
            return;
        }
        self.dropped_action_count = self.dropped_action_count.saturating_add(1);
    }

    /// Returns the next typed action in native callback order.
    #[must_use]
    pub fn take_next_action(&mut self) -> Option<WindowsPlatformAction> {
        self.pending_actions.pop_front()
    }

    /// Returns whether the first close-to-tray explanation was already requested.
    #[must_use]
    pub const fn close_notice_shown(&self) -> bool {
        self.close_notice_shown
    }

    /// Returns how many local menu actions could not be retained in the bounded queue.
    #[must_use]
    pub const fn dropped_action_count(&self) -> u32 {
        self.dropped_action_count
    }

    fn remove_non_quit(&mut self) -> bool {
        let Some(index) = self
            .pending_actions
            .iter()
            .position(|existing| *existing != WindowsPlatformAction::Quit)
        else {
            return false;
        };
        let _ = self.pending_actions.remove(index);
        true
    }
}

#[cfg(windows)]
mod native {
    use std::{error::Error, fmt, mem::size_of};

    use windows::{
        Win32::{
            Foundation::{HWND, POINT},
            UI::{
                Shell::{
                    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
                    NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, IDI_APPLICATION,
                    LoadIconW, MF_STRING, RegisterWindowMessageW, SetForegroundWindow,
                    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
                    WM_COMMAND, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
                },
            },
        },
        core::w,
    };

    use super::{CloseToTrayDisposition, WindowsPlatformAction, WindowsTrayController};

    const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 41;
    const TRAY_ICON_ID: u32 = 1;
    const MENU_OPEN: usize = 1;
    const MENU_PAUSE: usize = 2;
    const MENU_RESUME: usize = 3;
    const MENU_START_FOCUS: usize = 4;
    const MENU_QUIT: usize = 5;

    /// A native tray installation failure with no raw OS error text.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum WindowsTrayError {
        /// Windows could not add or restore the notification-area icon.
        AddIcon,
        /// Windows could not modify the notification-area icon.
        ModifyIcon,
        /// Windows could not create or show the tray popup menu.
        Menu,
    }

    impl fmt::Display for WindowsTrayError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let message = match self {
                Self::AddIcon => "Windows could not add the OpenManic tray icon",
                Self::ModifyIcon => "Windows could not update the OpenManic tray icon",
                Self::Menu => "Windows could not show the OpenManic tray menu",
            };
            formatter.write_str(message)
        }
    }

    impl Error for WindowsTrayError {}

    /// Native notification-area icon connected to the hidden control window.
    ///
    /// It must be driven from that window's normal message loop. Native callbacks only append a
    /// [`WindowsPlatformAction`] to the controller's local queue.
    #[derive(Debug)]
    pub struct WindowsTray {
        control_window: HWND,
        taskbar_created_message: u32,
        controller: WindowsTrayController,
    }

    impl WindowsTray {
        /// Adds an icon whose callback message is delivered to the hidden control window.
        ///
        /// # Errors
        ///
        /// Returns [`WindowsTrayError::AddIcon`] when Windows rejects the notification-area
        /// registration.
        pub(crate) fn install(control_window: HWND) -> Result<Self, WindowsTrayError> {
            // SAFETY: TaskbarCreated is a process-wide registered message name and Windows
            // retains no caller memory.
            let taskbar_created_message = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
            if taskbar_created_message == 0 {
                return Err(WindowsTrayError::AddIcon);
            }
            let mut tray = Self {
                control_window,
                taskbar_created_message,
                controller: WindowsTrayController::new(),
            };
            tray.add_icon()?;
            Ok(tray)
        }

        /// Returns the deterministic action/close policy owned by this tray.
        #[must_use]
        pub fn controller(&self) -> &WindowsTrayController {
            &self.controller
        }

        /// Returns mutable deterministic action/close policy for host-owned settings restore.
        pub fn controller_mut(&mut self) -> &mut WindowsTrayController {
            &mut self.controller
        }

        /// Records a normal viewport close and, on the first hide, asks Windows to show a notice.
        ///
        /// The returned disposition tells the host whether to cancel the close or begin its
        /// coordinated explicit Quit path. This method never stops tracking itself.
        #[must_use]
        pub fn on_main_window_close(
            &mut self,
            background_tracking_enabled: bool,
        ) -> CloseToTrayDisposition {
            let disposition = self
                .controller
                .on_main_window_close(background_tracking_enabled);
            if matches!(
                disposition,
                CloseToTrayDisposition::HideToTray {
                    show_one_time_notice: true
                }
            ) {
                let _ = self.show_close_to_tray_notice();
            }
            disposition
        }

        /// Shows a best-effort native notice after a durable focus completion.
        ///
        /// # Errors
        ///
        /// Returns [`WindowsTrayError::ModifyIcon`] when the notification-area icon cannot show
        /// the notice. The already-persisted focus completion is never affected.
        pub fn show_focus_completion_notice(&self) -> Result<(), WindowsTrayError> {
            let mut data = self.icon_data(NIF_INFO);
            write_wide(&mut data.szInfo, "Your focus session is complete.");
            write_wide(&mut data.szInfoTitle, "Focus session complete");
            // SAFETY: The shell copies the fixed-size notification structure synchronously.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const data) }.as_bool() {
                Ok(())
            } else {
                Err(WindowsTrayError::ModifyIcon)
            }
        }

        /// Removes and returns the next local native action without invoking application code.
        #[must_use]
        pub fn take_next_action(&mut self) -> Option<WindowsPlatformAction> {
            self.controller.take_next_action()
        }

        /// Handles a message delivered to the hidden control window.
        ///
        /// Returns `true` when the message belonged to the tray. The caller still dispatches the
        /// message normally so the control window remains a conventional Win32 message target.
        ///
        /// # Errors
        ///
        /// Returns an error only when Windows cannot construct the context menu or restore the
        /// notification icon after Explorer recreation.
        pub(crate) fn handle_control_message(
            &mut self,
            message: u32,
            wparam: usize,
            lparam: isize,
        ) -> Result<bool, WindowsTrayError> {
            if message == self.taskbar_created_message {
                self.add_icon()?;
                return Ok(true);
            }
            if message == TRAY_CALLBACK_MESSAGE {
                let notification = u32::try_from(lparam).map_err(|_| WindowsTrayError::Menu)?;
                self.handle_icon_callback(notification)?;
                return Ok(true);
            }
            if message != WM_COMMAND {
                return Ok(false);
            }
            let Some(action) = action_for_menu_id(wparam & 0xffff) else {
                return Ok(false);
            };
            self.controller.on_menu_action(action);
            Ok(true)
        }

        pub(crate) fn remove_icon(&mut self) {
            let data = self.icon_data(NIF_MESSAGE);
            // SAFETY: Removing an already-missing shell icon is harmless. The control loop calls
            // this before its hidden HWND is destroyed during normal coordinated shutdown.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &raw const data) };
        }

        fn add_icon(&mut self) -> Result<(), WindowsTrayError> {
            let data = self.icon_data(NIF_MESSAGE | NIF_ICON | NIF_TIP);
            // SAFETY: NOTIFYICONDATAW is fully initialized for the duration of the call and
            // Windows copies it synchronously.
            if !unsafe { Shell_NotifyIconW(NIM_ADD, &raw const data) }.as_bool() {
                return Err(WindowsTrayError::AddIcon);
            }
            let version = self.icon_data(NIF_MESSAGE);
            // SAFETY: The icon is owned by this process; setting the negotiated callback version
            // only changes Shell behavior and retains no pointer.
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &raw const version) };
            Ok(())
        }

        fn show_close_to_tray_notice(&self) -> Result<(), WindowsTrayError> {
            let mut data = self.icon_data(NIF_INFO);
            write_wide(&mut data.szInfo, "OpenManic is still tracking in the tray.");
            write_wide(&mut data.szInfoTitle, "OpenManic is still running");
            // SAFETY: The shell copies the fixed-size notification structure synchronously.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const data) }.as_bool() {
                Ok(())
            } else {
                Err(WindowsTrayError::ModifyIcon)
            }
        }

        fn handle_icon_callback(&mut self, notification: u32) -> Result<(), WindowsTrayError> {
            match notification {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    self.controller.on_menu_action(WindowsPlatformAction::Open);
                    Ok(())
                }
                WM_RBUTTONUP => self.show_menu(),
                _ => Ok(()),
            }
        }

        fn show_menu(&mut self) -> Result<(), WindowsTrayError> {
            // SAFETY: Windows creates a private menu handle, immediately populated and destroyed
            // by this method on the same control thread.
            let menu = unsafe { CreatePopupMenu() }.map_err(|_| WindowsTrayError::Menu)?;
            let result = self.populate_and_show_menu(menu);
            // SAFETY: `menu` was created above and is not retained after this method returns.
            let _ = unsafe { DestroyMenu(menu) };
            result
        }

        fn populate_and_show_menu(
            &self,
            menu: windows::Win32::UI::WindowsAndMessaging::HMENU,
        ) -> Result<(), WindowsTrayError> {
            append_menu_item(menu, MENU_OPEN, "Open")?;
            append_menu_item(menu, MENU_PAUSE, "Pause Tracking")?;
            append_menu_item(menu, MENU_RESUME, "Resume Tracking")?;
            append_menu_item(menu, MENU_START_FOCUS, "Start Focus Session")?;
            append_menu_item(menu, MENU_QUIT, "Quit")?;
            let mut point = POINT::default();
            // SAFETY: POINT is initialized writable storage owned by this stack frame.
            unsafe { GetCursorPos(&raw mut point) }.map_err(|_| WindowsTrayError::Menu)?;
            // SAFETY: The control window owns the normal message loop. This is the standard
            // notification-area popup sequence and retains no caller-owned pointer.
            let displayed = unsafe {
                let _ = SetForegroundWindow(self.control_window);
                TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    None,
                    self.control_window,
                    None,
                )
            };
            if displayed.as_bool() {
                Ok(())
            } else {
                Err(WindowsTrayError::Menu)
            }
        }

        fn icon_data(
            &self,
            flags: windows::Win32::UI::Shell::NOTIFY_ICON_DATA_FLAGS,
        ) -> NOTIFYICONDATAW {
            let mut data = NOTIFYICONDATAW {
                cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX),
                hWnd: self.control_window,
                uID: TRAY_ICON_ID,
                uFlags: flags,
                uCallbackMessage: TRAY_CALLBACK_MESSAGE,
                Anonymous: windows::Win32::UI::Shell::NOTIFYICONDATAW_0 {
                    uVersion: NOTIFYICON_VERSION_4,
                },
                ..Default::default()
            };
            // SAFETY: IDI_APPLICATION is a system-owned stock icon. LoadIconW retains no pointer
            // and its returned handle stays valid for the process lifetime.
            data.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default();
            write_wide(&mut data.szTip, "OpenManic");
            data
        }
    }

    impl Drop for WindowsTray {
        fn drop(&mut self) {
            self.remove_icon();
        }
    }

    fn action_for_menu_id(menu_id: usize) -> Option<WindowsPlatformAction> {
        match menu_id {
            MENU_OPEN => Some(WindowsPlatformAction::Open),
            MENU_PAUSE => Some(WindowsPlatformAction::PauseTracking),
            MENU_RESUME => Some(WindowsPlatformAction::ResumeTracking),
            MENU_START_FOCUS => Some(WindowsPlatformAction::StartFocusSession),
            MENU_QUIT => Some(WindowsPlatformAction::Quit),
            _ => None,
        }
    }

    fn append_menu_item(
        menu: windows::Win32::UI::WindowsAndMessaging::HMENU,
        identifier: usize,
        label: &str,
    ) -> Result<(), WindowsTrayError> {
        let mut wide: Vec<u16> = label.encode_utf16().collect();
        wide.push(0);
        // SAFETY: The NUL-terminated label is retained until AppendMenuW returns; Windows copies
        // the text into its menu representation.
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                identifier,
                windows::core::PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(|_| WindowsTrayError::Menu)
    }

    fn write_wide(destination: &mut [u16], value: &str) {
        let length = destination.len().saturating_sub(1);
        for (slot, character) in destination
            .iter_mut()
            .zip(value.encode_utf16().take(length))
        {
            *slot = character;
        }
    }
}

#[cfg(windows)]
pub use native::{WindowsTray, WindowsTrayError};

#[cfg(test)]
mod tests {
    use super::{CloseToTrayDisposition, WindowsPlatformAction, WindowsTrayController};

    #[test]
    fn close_to_tray_preserves_tracking_and_emits_one_notice() {
        let mut controller = WindowsTrayController::new();

        assert_eq!(
            controller.on_main_window_close(true),
            CloseToTrayDisposition::HideToTray {
                show_one_time_notice: true
            }
        );
        assert_eq!(
            controller.on_main_window_close(true),
            CloseToTrayDisposition::HideToTray {
                show_one_time_notice: false
            }
        );
        assert!(controller.close_notice_shown());
        assert_eq!(controller.take_next_action(), None);
    }

    #[test]
    fn close_without_background_tracking_begins_only_coordinated_quit() {
        let mut controller = WindowsTrayController::new();

        assert_eq!(
            controller.on_main_window_close(false),
            CloseToTrayDisposition::BeginCoordinatedQuit
        );
        assert_eq!(controller.take_next_action(), None);
    }

    #[test]
    fn tray_actions_are_ordered_local_requests() {
        let mut controller = WindowsTrayController::new();
        controller.on_menu_action(WindowsPlatformAction::Open);
        controller.on_menu_action(WindowsPlatformAction::PauseTracking);
        controller.on_menu_action(WindowsPlatformAction::StartFocusSession);
        controller.on_menu_action(WindowsPlatformAction::Quit);

        assert_eq!(
            controller.take_next_action(),
            Some(WindowsPlatformAction::Open)
        );
        assert_eq!(
            controller.take_next_action(),
            Some(WindowsPlatformAction::PauseTracking)
        );
        assert_eq!(
            controller.take_next_action(),
            Some(WindowsPlatformAction::StartFocusSession)
        );
        assert_eq!(
            controller.take_next_action(),
            Some(WindowsPlatformAction::Quit)
        );
        assert_eq!(controller.take_next_action(), None);
    }

    #[test]
    fn preference_can_make_a_close_an_explicit_quit_request() {
        let mut controller = WindowsTrayController::new();
        controller.set_close_to_tray_enabled(false);

        assert_eq!(
            controller.on_main_window_close(true),
            CloseToTrayDisposition::BeginCoordinatedQuit
        );
    }

    #[test]
    fn bounded_menu_queue_preserves_quit_when_saturated() {
        let mut controller = WindowsTrayController::new();
        for _ in 0..16 {
            controller.on_menu_action(WindowsPlatformAction::Open);
        }

        controller.on_menu_action(WindowsPlatformAction::Quit);

        assert_eq!(controller.dropped_action_count(), 0);
        assert!(
            std::iter::from_fn(|| controller.take_next_action())
                .any(|action| action == WindowsPlatformAction::Quit)
        );
    }
}
