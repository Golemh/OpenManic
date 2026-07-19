//! Typed failures crossing the application port boundary.

use core::fmt;

/// A typed failure from an application-facing port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    /// A port could not perform its contract-defined operation.
    PortFailure {
        /// The failed application port.
        port: ApplicationPort,
        /// The stable reason category reported by that port.
        reason: PortFailureReason,
    },
}

impl ApplicationError {
    /// Creates a typed port-boundary failure.
    #[must_use]
    pub const fn port_failure(port: ApplicationPort, reason: PortFailureReason) -> Self {
        Self::PortFailure { port, reason }
    }
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortFailure { port, reason } => write!(formatter, "{port} port failed: {reason}"),
        }
    }
}

impl std::error::Error for ApplicationError {}

/// Identifies the application port that returned an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationPort {
    /// The command submission boundary.
    Command,
    /// The projection request boundary.
    Projection,
}

impl fmt::Display for ApplicationPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Command => "command",
            Self::Projection => "projection",
        };
        formatter.write_str(name)
    }
}

/// A stable application-port failure category that exposes no adapter internals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortFailureReason {
    /// The port is temporarily unable to receive work.
    Unavailable,
    /// The port cannot accept more work without violating its backpressure policy.
    Busy,
    /// The port failed while performing its operation.
    Failed,
}

impl fmt::Display for PortFailureReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::Failed => "operation failed",
        };
        formatter.write_str(description)
    }
}
