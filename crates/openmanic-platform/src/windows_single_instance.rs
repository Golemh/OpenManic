//! Current-user single-instance names and the narrow local activation protocol.
//!
//! Activation is deliberately not a general IPC API. Its wire representation is a short,
//! versioned command with no request payload and no response payload. Unknown future commands are
//! ignored rather than treated as data requests or as a reason to start another writer.

/// Largest accepted activation request, including its protocol marker.
pub const WINDOWS_ACTIVATION_MESSAGE_LIMIT: usize = 64;

const ACTIVATION_PROTOCOL_PREFIX: &[u8] = b"OpenManic/activation-v1 ";

/// The accepted fixed local activation commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalActivationCommand {
    /// Restore the existing process's main window, with flash fallback if focus is denied.
    Activate,
    /// Ask the existing process to pause tracking.
    PauseTracking,
    /// Ask the existing process to resume tracking.
    ResumeTracking,
}

impl LocalActivationCommand {
    /// Serializes this command into the versioned, payload-free local protocol.
    #[must_use]
    pub const fn wire_message(self) -> &'static [u8] {
        match self {
            Self::Activate => b"OpenManic/activation-v1 Activate\n",
            Self::PauseTracking => b"OpenManic/activation-v1 PauseTracking\n",
            Self::ResumeTracking => b"OpenManic/activation-v1 ResumeTracking\n",
        }
    }

    /// Maps this private activation command to the corresponding local platform action.
    #[must_use]
    pub const fn platform_action(self) -> crate::WindowsPlatformAction {
        match self {
            Self::Activate => crate::WindowsPlatformAction::Open,
            Self::PauseTracking => crate::WindowsPlatformAction::PauseTracking,
            Self::ResumeTracking => crate::WindowsPlatformAction::ResumeTracking,
        }
    }
}

/// The safe result of decoding one bounded local activation request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationCommandDecode {
    /// A known command that the host may turn into a local platform action.
    Known(LocalActivationCommand),
    /// A well-formed or future request that must not be interpreted as data access.
    IgnoredUnknown,
    /// Invalid input that must be discarded without responding with local information.
    Rejected,
}

/// Parses a complete activation request without allocating or accepting user data.
#[must_use]
pub(crate) fn decode_activation_command(message: &[u8]) -> ActivationCommandDecode {
    if message.len() > WINDOWS_ACTIVATION_MESSAGE_LIMIT
        || !message.starts_with(ACTIVATION_PROTOCOL_PREFIX)
        || !message.ends_with(b"\n")
    {
        return ActivationCommandDecode::Rejected;
    }

    match &message[ACTIVATION_PROTOCOL_PREFIX.len()..message.len().saturating_sub(1)] {
        b"Activate" => ActivationCommandDecode::Known(LocalActivationCommand::Activate),
        b"PauseTracking" => ActivationCommandDecode::Known(LocalActivationCommand::PauseTracking),
        b"ResumeTracking" => ActivationCommandDecode::Known(LocalActivationCommand::ResumeTracking),
        command if command.is_empty() || !command.iter().all(u8::is_ascii_alphanumeric) => {
            ActivationCommandDecode::Rejected
        }
        _ => ActivationCommandDecode::IgnoredUnknown,
    }
}

#[cfg(windows)]
mod native {
    use std::{error::Error, fmt, mem::size_of};

    use windows::Win32::{
        Foundation::{CloseHandle, GENERIC_ALL, HANDLE},
        Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce, GetLengthSid,
            GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, PSID,
            SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl,
            SetSecurityDescriptorOwner, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile},
        System::{
            Pipes::{
                CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
            },
            Threading::{CreateMutexExW, GetCurrentProcess, MUTEX_ALL_ACCESS, OpenProcessToken},
        },
    };

    use super::{LocalActivationCommand, WINDOWS_ACTIVATION_MESSAGE_LIMIT};

    const ACTIVATION_MESSAGE_LIMIT_U32: u32 = 64;

    /// Failure to obtain or contact the local current-user instance without raw OS error leakage.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum WindowsInstanceError {
        /// The current process token could not provide a stable user SID for ACL construction.
        CurrentUserSecurity,
        /// Windows could not create or open the current-user instance mutex.
        Mutex,
        /// The existing local process did not accept an activation command immediately.
        ActivationUnavailable,
    }

    impl fmt::Display for WindowsInstanceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let message = match self {
                Self::CurrentUserSecurity => {
                    "Windows could not prepare current-user OpenManic instance security"
                }
                Self::Mutex => "Windows could not coordinate the local OpenManic instance",
                Self::ActivationUnavailable => {
                    "the existing OpenManic instance did not accept local activation"
                }
            };
            formatter.write_str(message)
        }
    }

    impl Error for WindowsInstanceError {}

    /// The result of attempting to own the one-per-signed-in-user mutex.
    #[derive(Debug)]
    pub enum InstanceAcquisition {
        /// This process owns instance coordination and must still acquire OM-295's data-root lock.
        Primary(WindowsInstanceOwner),
        /// A current-user instance already exists; callers send an activation command then exit.
        ExistingInstance(WindowsExistingInstance),
    }

    /// An owner of the current-user mutex.
    ///
    /// It intentionally says nothing about writer ownership: the OM-295 data-root lock remains
    /// the final, independent protection against concurrent SQLite writers.
    #[derive(Debug)]
    pub struct WindowsInstanceOwner {
        mutex: HANDLE,
        security: CurrentUserSecurity,
        pipe_name: Vec<u16>,
    }

    impl WindowsInstanceOwner {
        /// Attempts current-user instance acquisition before resolving the data root.
        ///
        /// # Errors
        ///
        /// Returns a privacy-safe failure when Windows cannot construct the current-user ACL or
        /// create/open the named mutex.
        pub fn acquire() -> Result<InstanceAcquisition, WindowsInstanceError> {
            let security = CurrentUserSecurity::from_current_process()?;
            let name_suffix = security.name_suffix();
            let mutex_name = utf16_name(&format!("Local\\OpenManic.Instance.{name_suffix}"));
            let pipe_name = utf16_name(&format!("\\\\.\\pipe\\OpenManic.Activation.{name_suffix}"));
            let attributes = security.attributes();

            // SAFETY: The security descriptor and UTF-16 mutex name remain valid throughout the
            // synchronous call. The returned handle is owned by this value and closed in Drop.
            let mutex = unsafe {
                CreateMutexExW(
                    Some(&raw const attributes),
                    windows::core::PCWSTR(mutex_name.as_ptr()),
                    0,
                    MUTEX_ALL_ACCESS.0,
                )
            }
            .map_err(|_| WindowsInstanceError::Mutex)?;

            // SAFETY: GetLastError immediately follows the documented CreateMutexExW call and
            // merely reads thread-local Windows state.
            let already_exists = unsafe { windows::Win32::Foundation::GetLastError() }
                == windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
            if already_exists {
                // SAFETY: The just-returned mutex handle is not retained by this process when a
                // current-user owner already exists.
                let _ = unsafe { CloseHandle(mutex) };
                return Ok(InstanceAcquisition::ExistingInstance(
                    WindowsExistingInstance { pipe_name },
                ));
            }

            Ok(InstanceAcquisition::Primary(Self {
                mutex,
                security,
                pipe_name,
            }))
        }

        /// Returns the opaque current-user pipe name for the private activation server.
        #[must_use]
        pub(crate) fn pipe_name(&self) -> &[u16] {
            &self.pipe_name
        }

        /// Returns current-user security attributes for a server created by this owner.
        #[must_use]
        pub(crate) fn security_attributes(&self) -> SECURITY_ATTRIBUTES {
            self.security.attributes()
        }

        /// Creates the current-user-only listener after the primary has acquired the instance.
        ///
        /// The listener performs a bounded, payload-free command read. Its blocking receive must
        /// be driven by a dedicated listener owner, never by the UI or hidden control callback.
        ///
        /// # Errors
        ///
        /// Returns a privacy-safe failure when Windows cannot create the ACL-protected pipe.
        pub fn activation_server(&self) -> Result<WindowsActivationServer, WindowsInstanceError> {
            WindowsActivationServer::create(self)
        }

        /// Wakes a blocked local activation listener so coordinated shutdown can join it.
        ///
        /// The wire request remains the fixed payload-free activation command; no process data is
        /// exposed. Callers set their own stop signal before invoking this method, so the listener
        /// exits after accepting this wake-up request rather than restoring the viewport.
        #[must_use]
        pub fn wake_activation_listener(&self) -> ActivationSendOutcome {
            WindowsExistingInstance {
                pipe_name: self.pipe_name.clone(),
            }
            .send(LocalActivationCommand::Activate)
        }
    }

    impl Drop for WindowsInstanceOwner {
        fn drop(&mut self) {
            // SAFETY: This value owns the successful CreateMutexExW handle and drops it once.
            let _ = unsafe { CloseHandle(self.mutex) };
        }
    }

    /// A second launch that can only send one of the fixed local activation commands.
    #[derive(Debug)]
    pub struct WindowsExistingInstance {
        pipe_name: Vec<u16>,
    }

    /// Delivery result without any response data from the existing process.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ActivationSendOutcome {
        /// The existing process accepted the fixed local command.
        Delivered,
        /// The mutex existed but no activation pipe accepted the immediate request.
        ExistingInstanceUnreachable,
    }

    impl WindowsExistingInstance {
        /// Sends one payload-free local activation command and never reads process data back.
        #[must_use]
        pub fn send(&self, command: LocalActivationCommand) -> ActivationSendOutcome {
            let message = command.wire_message();
            debug_assert!(message.len() <= WINDOWS_ACTIVATION_MESSAGE_LIMIT);
            let mut response_length = 0_u32;
            // SAFETY: Both the NUL-terminated pipe name and fixed command slice stay alive for
            // the synchronous call. There is no response buffer, so the protocol cannot expose
            // process data to a second launch.
            let delivered = unsafe {
                CallNamedPipeW(
                    windows::core::PCWSTR(self.pipe_name.as_ptr()),
                    Some(message.as_ptr().cast()),
                    u32::try_from(message.len()).unwrap_or(ACTIVATION_MESSAGE_LIMIT_U32),
                    None,
                    0,
                    &raw mut response_length,
                    0,
                )
            }
            .as_bool();
            if delivered {
                ActivationSendOutcome::Delivered
            } else {
                ActivationSendOutcome::ExistingInstanceUnreachable
            }
        }
    }

    /// One ACL-protected current-user activation listener.
    ///
    /// The listener uses exactly one pipe instance and returns only decoded fixed commands. It
    /// never returns data to the connecting process, and callers must keep it off the UI and
    /// control-window threads because `receive_next` deliberately waits for a local connection.
    #[derive(Debug)]
    pub struct WindowsActivationServer {
        pipe: HANDLE,
    }

    // SAFETY: this owned named-pipe HANDLE has no thread-affine state. The server is moved once
    // into its dedicated listener thread and all connect/read/disconnect operations remain
    // serialized through `&mut self`; Drop closes the handle on that same owner thread.
    unsafe impl Send for WindowsActivationServer {}

    impl WindowsActivationServer {
        fn create(owner: &WindowsInstanceOwner) -> Result<Self, WindowsInstanceError> {
            let attributes = owner.security_attributes();
            // SAFETY: The current-user ACL and NUL-terminated name remain valid for this
            // synchronous call. The returned handle is private to this server and closed in Drop.
            let pipe = unsafe {
                CreateNamedPipeW(
                    windows::core::PCWSTR(owner.pipe_name().as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    ACTIVATION_MESSAGE_LIMIT_U32,
                    ACTIVATION_MESSAGE_LIMIT_U32,
                    0,
                    Some(&raw const attributes),
                )
            };
            if pipe.is_invalid() {
                return Err(WindowsInstanceError::ActivationUnavailable);
            }
            Ok(Self { pipe })
        }

        /// Waits for one local client and returns a bounded, decoded command without a response.
        ///
        /// Unknown protocol revisions/commands are returned as
        /// [`crate::ActivationCommandDecode::IgnoredUnknown`]
        /// so a future client cannot become a data-access path. Invalid input is rejected. The
        /// server is immediately reusable after this call because the pipe is disconnected before
        /// the result is returned.
        ///
        /// # Errors
        ///
        /// Returns a generic local-activation failure if Windows cannot complete the bounded
        /// connection or read.
        pub fn receive_next(
            &mut self,
        ) -> Result<super::ActivationCommandDecode, WindowsInstanceError> {
            // SAFETY: This server owns the named-pipe handle. A client that connected between
            // CreateNamedPipeW and this call is recognized through ERROR_PIPE_CONNECTED below.
            let connected = unsafe { ConnectNamedPipe(self.pipe, None) };
            if connected.is_err()
                // SAFETY: GetLastError is read immediately after ConnectNamedPipe's documented
                // failure path and does not expose state beyond the local control decision.
                && unsafe { windows::Win32::Foundation::GetLastError() }
                    != windows::Win32::Foundation::ERROR_PIPE_CONNECTED
            {
                return Err(WindowsInstanceError::ActivationUnavailable);
            }

            let mut buffer = [0_u8; WINDOWS_ACTIVATION_MESSAGE_LIMIT];
            let mut read = 0_u32;
            // SAFETY: The fixed stack buffer is valid for the synchronous bounded read; no
            // OVERLAPPED storage is supplied because this listener is explicitly not a callback.
            let outcome =
                unsafe { ReadFile(self.pipe, Some(&mut buffer), Some(&raw mut read), None) }
                    .map(|()| super::decode_activation_command(&buffer[..read as usize]))
                    .map_err(|_| WindowsInstanceError::ActivationUnavailable);
            // SAFETY: The pipe handle is owned by this server. Disconnect restores it to the
            // listening state even after malformed input; Drop will close it only once later.
            let _ = unsafe { DisconnectNamedPipe(self.pipe) };
            outcome
        }
    }

    impl Drop for WindowsActivationServer {
        fn drop(&mut self) {
            // SAFETY: This value owns one CreateNamedPipeW handle. Disconnect is best effort
            // before close so a partially connected client cannot retain the endpoint.
            let _ = unsafe { DisconnectNamedPipe(self.pipe) };
            // SAFETY: This value closes its private pipe handle exactly once.
            let _ = unsafe { CloseHandle(self.pipe) };
        }
    }

    #[derive(Debug)]
    struct CurrentUserSecurity {
        sid: Box<[u8]>,
        acl_words: Box<[usize]>,
        descriptor: Box<SECURITY_DESCRIPTOR>,
    }

    impl CurrentUserSecurity {
        fn from_current_process() -> Result<Self, WindowsInstanceError> {
            let mut token = HANDLE::default();
            // SAFETY: GetCurrentProcess yields the pseudo-handle for this process. Windows writes
            // one owned token handle to initialized stack storage, which is closed below.
            unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    windows::Win32::Security::TOKEN_QUERY,
                    &raw mut token,
                )
            }
            .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            let result = Self::from_token(token);
            // SAFETY: OpenProcessToken created this handle for the current process and this method
            // never stores it beyond the current SID copy.
            let _ = unsafe { CloseHandle(token) };
            result
        }

        fn from_token(token: HANDLE) -> Result<Self, WindowsInstanceError> {
            let sid = Self::copy_token_sid(token)?;

            let acl_bytes = size_of::<ACL>()
                .saturating_add(size_of::<ACCESS_ALLOWED_ACE>())
                .saturating_add(sid.len())
                .saturating_sub(size_of::<u32>());
            let word_count = acl_bytes.div_ceil(size_of::<usize>());
            let acl_bytes_u32 =
                u32::try_from(acl_bytes).map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            let mut acl_words = vec![0_usize; word_count].into_boxed_slice();
            let acl = acl_words.as_mut_ptr().cast::<ACL>();
            // SAFETY: The word-backed allocation has sufficient size and alignment for ACL. The
            // following calls initialize its header and one ACE for the copied current-user SID.
            unsafe {
                InitializeAcl(acl, acl_bytes_u32, ACL_REVISION)
                    .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
                AddAccessAllowedAce(
                    acl,
                    ACL_REVISION,
                    GENERIC_ALL.0,
                    PSID(sid.as_ptr().cast_mut().cast()),
                )
                .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            }

            let mut descriptor = Box::<SECURITY_DESCRIPTOR>::default();
            // SAFETY: The descriptor, ACL, and copied SID stay owned by this value while any
            // mutex or pipe creation consumes its borrowed SECURITY_ATTRIBUTES.
            unsafe {
                InitializeSecurityDescriptor(
                    windows::Win32::Security::PSECURITY_DESCRIPTOR((&raw mut *descriptor).cast()),
                    1,
                )
                .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
                SetSecurityDescriptorOwner(
                    windows::Win32::Security::PSECURITY_DESCRIPTOR((&raw mut *descriptor).cast()),
                    Some(PSID(sid.as_ptr().cast_mut().cast())),
                    false,
                )
                .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
                SetSecurityDescriptorDacl(
                    windows::Win32::Security::PSECURITY_DESCRIPTOR((&raw mut *descriptor).cast()),
                    true,
                    Some(acl.cast_const()),
                    false,
                )
                .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            }

            Ok(Self {
                sid,
                acl_words,
                descriptor,
            })
        }

        fn copy_token_sid(token: HANDLE) -> Result<Box<[u8]>, WindowsInstanceError> {
            let mut required = 0_u32;
            // SAFETY: This standard sizing call passes no data buffer and lets Windows write only
            // the required byte count. Its expected insufficient-buffer result is intentionally
            // ignored because the size is checked before allocation.
            let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &raw mut required) };
            let minimum_token_user_size =
                u32::try_from(size_of::<TOKEN_USER>()).unwrap_or(u32::MAX);
            if required < minimum_token_user_size {
                return Err(WindowsInstanceError::CurrentUserSecurity);
            }
            let token_bytes_len =
                usize::try_from(required).map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            let mut token_bytes = vec![0_u8; token_bytes_len];
            // SAFETY: The vector supplies exactly the byte capacity requested above and Windows
            // writes a TOKEN_USER whose SID remains valid until this vector is dropped.
            unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(token_bytes.as_mut_ptr().cast()),
                    required,
                    &raw mut required,
                )
            }
            .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            // SAFETY: GetTokenInformation initialized the leading TOKEN_USER. `read_unaligned`
            // copies that value without assuming the byte buffer has TOKEN_USER alignment.
            let token_user =
                unsafe { std::ptr::read_unaligned(token_bytes.as_ptr().cast::<TOKEN_USER>()) };
            // SAFETY: `token_user.User.Sid` was populated by GetTokenInformation and remains
            // valid while `token_bytes` is retained below.
            let sid_length = usize::try_from(unsafe { GetLengthSid(token_user.User.Sid) })
                .map_err(|_| WindowsInstanceError::CurrentUserSecurity)?;
            if sid_length == 0 {
                return Err(WindowsInstanceError::CurrentUserSecurity);
            }
            let mut sid = vec![0_u8; sid_length].into_boxed_slice();
            // SAFETY: The token-owned SID is at least `sid_length` bytes and the destination owns
            // exactly that many writable bytes. This copy ends the token buffer lifetime tie.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    token_user.User.Sid.0.cast::<u8>(),
                    sid.as_mut_ptr(),
                    sid_length,
                );
            }

            Ok(sid)
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            // The descriptor borrows this allocation as its DACL, so retaining the field here is
            // part of the security-attribute lifetime contract rather than dead storage.
            let _ = &self.acl_words;
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
                lpSecurityDescriptor: (&raw const *self.descriptor).cast_mut().cast(),
                bInheritHandle: false.into(),
            }
        }

        fn name_suffix(&self) -> String {
            // FNV-1a makes the current-user object names stable without embedding a raw SID in
            // diagnostics. Security still comes from the ACL; this is not treated as a secret.
            let hash = self
                .sid
                .iter()
                .fold(0xcbf2_9ce4_8422_2325_u64, |value, byte| {
                    (value ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
                });
            format!("{hash:016x}")
        }
    }

    fn utf16_name(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
pub use native::{
    ActivationSendOutcome, InstanceAcquisition, WindowsActivationServer, WindowsExistingInstance,
    WindowsInstanceError, WindowsInstanceOwner,
};

#[cfg(test)]
mod tests {
    use super::{
        ActivationCommandDecode, LocalActivationCommand, WINDOWS_ACTIVATION_MESSAGE_LIMIT,
        decode_activation_command,
    };
    use crate::WindowsPlatformAction;

    #[test]
    fn fixed_commands_round_trip_without_payloads() {
        for command in [
            LocalActivationCommand::Activate,
            LocalActivationCommand::PauseTracking,
            LocalActivationCommand::ResumeTracking,
        ] {
            assert_eq!(
                decode_activation_command(command.wire_message()),
                ActivationCommandDecode::Known(command)
            );
        }
    }

    #[test]
    fn future_command_is_ignored_without_becoming_data_access() {
        assert_eq!(
            decode_activation_command(b"OpenManic/activation-v1 StartFocus\n"),
            ActivationCommandDecode::IgnoredUnknown
        );
    }

    #[test]
    fn activation_commands_stay_local_typed_actions() {
        assert_eq!(
            LocalActivationCommand::Activate.platform_action(),
            WindowsPlatformAction::Open
        );
        assert_eq!(
            LocalActivationCommand::PauseTracking.platform_action(),
            WindowsPlatformAction::PauseTracking
        );
        assert_eq!(
            LocalActivationCommand::ResumeTracking.platform_action(),
            WindowsPlatformAction::ResumeTracking
        );
    }

    #[test]
    fn malformed_or_oversized_commands_are_rejected() {
        assert_eq!(
            decode_activation_command(b"Activate\n"),
            ActivationCommandDecode::Rejected
        );
        assert_eq!(
            decode_activation_command(b"OpenManic/activation-v1 Pause Tracking\n"),
            ActivationCommandDecode::Rejected
        );
        let oversized = vec![b'x'; WINDOWS_ACTIVATION_MESSAGE_LIMIT + 1];
        assert_eq!(
            decode_activation_command(&oversized),
            ActivationCommandDecode::Rejected
        );
    }
}
