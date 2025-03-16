//! Private macros module for conditional tracing
//!
//! This module provides macros that simplify conditional tracing by
//! squashing the two-line pattern:
//!
//! ```
//! #[cfg(feature = "tracing")]
//! tracing::debug!("message", args);
//! ```
//!
//! Into a `debug!` macro call.
//!
//! These macros have the same names as the tracing crate's macros,
//! but they are conditionally compiled based on the "tracing" feature.
#![allow(unused_imports)]
#![allow(unused_macros)]

/// Conditionally emits a trace-level log message when the "tracing" feature is enabled.
///
/// This macro does nothing when the "tracing" feature is disabled.
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        #[cfg(feature = "tracing")]
        tracing::trace!($($arg)*);
    };
}

/// Conditionally emits a debug-level log message when the "tracing" feature is enabled.
///
/// This macro does nothing when the "tracing" feature is disabled.
macro_rules! debug {
    ($($arg:tt)*) => {
        #[cfg(feature = "tracing")]
        tracing::debug!($($arg)*);
    };
}

/// Conditionally emits an info-level log message when the "tracing" feature is enabled.
///
/// This macro does nothing when the "tracing" feature is disabled.
macro_rules! info {
    ($($arg:tt)*) => {
        #[cfg(feature = "tracing")]
        tracing::info!($($arg)*);
    };
}

/// Conditionally emits a warn-level log message when the "tracing" feature is enabled.
///
/// This macro does nothing when the "tracing" feature is disabled.
macro_rules! warning {
    ($($arg:tt)*) => {
        #[cfg(feature = "tracing")]
        tracing::warn!($($arg)*);
    };
}

/// Conditionally emits an error-level log message when the "tracing" feature is enabled.
///
/// This macro does nothing when the "tracing" feature is disabled.
macro_rules! error {
    ($($arg:tt)*) => {
        #[cfg(feature = "tracing")]
        tracing::error!($($arg)*);
    };
}

pub(crate) use {debug, error, info, trace, warning};
