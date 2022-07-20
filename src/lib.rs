//! This is the intended entry point to this library.
//!
//! It should only be in a very rare circumstance that a [`DetailedError`] is constructed directly.
//!
//! The main purpose of this library is to create a way to propagate errors through applications
//! in the easiest way possible whilst also:
//! 1. Emitting an event [`tracing::event`] with the underlying error that occurred, containing as
//!    much information as possible
//! 2. Allowing you to provide context to the underlying error
//! 3. Mapping that internal _private_ error into one that can be turned straight into a sanitized
//! public response for your customer
//!
//! Truthfully _at the moment_ it is held together by blue-tack and shoe strings. However it does
//! work provided you conform exactly to what is expected.
//!
//! ## Key Components & Constraints
//!
//! ### Your private error
//!
//! - This is the first argument to the macro
//! - It must implement [`Error`](StdError) + [`Send`] + [`Sync`] + `'static`
//!
//! ### Your public error
//!
//! - This is the second argument to the macro
//! - It must implement [`ToResponse`] + [`Debug`]
//!
//! ### Context
//!
//! - The optional third argument is context you want to wrap your private error with
//!
//! ### Additional fields
//!
//! - The 4th+ arguments are key + value pairs that you want to add to the tracing message that is
//! emitted
//!
//! # Examples
//!
//! ```
//! use thiserror::Error as ThisError;
//! use serde_json::{json, Value};
//!
//! use std::fmt::{self, Debug};
//!
//! use api_error::{DetailedError, ToResponse, e};
//!
//!
//! #[derive(Debug, ThisError)]
//! enum PublicError {
//!     #[error("An unexpected server error occurred, please try again in 5 seconds.")]
//!     UnexpectedServerError
//! }
//!
//! impl ToResponse for PublicError {
//!     type Response = Value;
//!
//!     fn to_response(&self) -> Self::Response {
//!         let category = format!("{:?}", self);
//!         let msg = format!("{}", self);
//!         json!({
//!             "category": category,
//!             "msg": msg
//!         })
//!     }
//! }
//!
//! type Error = DetailedError<PublicError>;
//!
//! fn test() -> Result<(), Error> {
//!     use std::fs::File;
//!
//!     let f = File::open("random.txt").map_err(|e| {
//!         e!(
//!             e, PublicError::UnexpectedServerError,
//!             "failed to read my amazing file"
//!         )
//!     })?;
//!     Ok(())
//! }
//!
//! fn main() {
//!     let e = test().unwrap_err();
//!     let json_response = e.to_response();
//!     assert_eq!(json_response["category"].as_str().unwrap(), "UnexpectedServerError");
//!     assert_eq!(json_response["msg"].as_str().unwrap(), "An unexpected server error occurred, please try again in 5 seconds.");
//! }
//!
//! ```
//!
//! [StdError]: std::error::Error
//! [Debug]: std::fmt::Debug
//! [Display]: std::fmt::Display

#[cfg(feature = "anyhow")]
use anyhow::Error as InnerError;

#[cfg(feature = "eyre")]
use eyre::Report as InnerError;

#[cfg(all(feature = "anyhow", feature = "eyre"))]
compile_error!("features `anyhow` and `eyre` are mutually exclusive, please choose one");

use tracing::{debug, error, info, trace, warn, Level};

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Display};
use std::ops::Deref;

pub struct DetailedError<Pub>
where
    Pub: ToResponse + Debug,
{
    pub private: InnerError,
    pub public: Pub,
    meta: Meta,
}

/// This trait indicates how you want to turn your `PublicError` type into a `Response`.
///
/// It is entirely up to you to choose how you would like to implement this
pub trait ToResponse {
    type Response;

    fn to_response(&self) -> Self::Response;
}

pub struct Meta {
    fields: HashMap<String, String>,
    file: String,
    module: String,
    line: u32,
    level: Level,
    has_logged: bool,
}

impl<Pub> DetailedError<Pub>
where
    Pub: ToResponse + Debug,
{
    pub fn new<P: StdError + Send + Sync + 'static, C: Display + Send + Sync + 'static>(
        private: P,
        public: Pub,
        context: Option<C>,
        level: Level,
        file: String,
        line: u32,
        module: String,
    ) -> Self {
        Self::new_with_tracing(
            private,
            public,
            context,
            level,
            file,
            line,
            module,
            HashMap::with_capacity(0),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_tracing<
        P: StdError + Send + Sync + 'static,
        C: Display + Send + Sync + 'static,
    >(
        private: P,
        public: Pub,
        context: Option<C>,
        level: Level,
        file: String,
        line: u32,
        module: String,
        fields: HashMap<String, String>,
    ) -> Self {
        let meta = Meta {
            fields,
            file,
            module,
            line,
            level,
            has_logged: false,
        };
        #[cfg(feature = "anyhow")]
        let private = if let Some(ctx) = context {
            anyhow::Error::new(private).context(ctx)
        } else {
            anyhow::Error::new(private)
        };
        #[cfg(feature = "eyre")]
        let private = if let Some(ctx) = context {
            eyre::Report::new(private).wrap_err(ctx)
        } else {
            eyre::Report::new(private)
        };
        let mut err = DetailedError {
            public,
            private,
            meta,
        };
        err.log();
        err
    }

    pub fn to_response(&self) -> Pub::Response {
        self.public.to_response()
    }

    pub fn into_inner(self) -> (InnerError, Pub) {
        (self.private, self.public)
    }

    #[inline]
    pub fn log(&mut self) {
        let error = &self.private;
        let meta = &self.meta;
        if self.meta.has_logged {
            return;
        }

        let mut errors: Vec<String> = vec![];

        // Skip the first entry, which is going to go into the msg field
        for cause in error.chain().skip(1) {
            errors.push(cause.to_string());
        }

        let has_fields = !meta.fields.is_empty();
        match meta.level {
            Level::ERROR if has_fields => {
                error!(
                    errors = ?errors,
                    public_error = ?self.public,
                    additional_context = ?meta.fields,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::ERROR => {
                error!(
                    errors = ?errors,
                    public_error = ?self.public,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::WARN if has_fields => {
                warn!(
                    errors = ?errors,
                    public_error = ?self.public,
                    additional_context = ?meta.fields,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::WARN => {
                warn!(
                    errors = ?errors,
                    public_error = ?self.public,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::INFO if has_fields => {
                info!(
                    errors = ?errors,
                    public_error = ?self.public,
                    additional_context = ?meta.fields,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::INFO => {
                info!(
                    errors = ?errors,
                    public_error = ?self.public,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::DEBUG if has_fields => {
                debug!(
                    errors = ?errors,
                    public_error = ?self.public,
                    additional_context = ?meta.fields,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::DEBUG => {
                debug!(
                    errors = ?errors,
                    public_error = ?self.public,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::TRACE if has_fields => {
                trace!(
                    errors = ?errors,
                    public_error = ?self.public,
                    additional_context = ?meta.fields,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
            Level::TRACE => {
                trace!(
                    errors = ?errors,
                    public_error = ?self.public,
                    file = %meta.file, line = %meta.line as i64,
                    module = %meta.module,
                    "{}", error
                );
            }
        }
        self.meta.has_logged = true;
    }
}

impl<Pub> fmt::Debug for DetailedError<Pub>
where
    Pub: ToResponse + Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.private)
    }
}

impl<Pub> fmt::Display for DetailedError<Pub>
where
    Pub: ToResponse + Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.private)
    }
}

impl<Pub> StdError for DetailedError<Pub>
where
    Pub: ToResponse + Debug,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.private.source()
    }
}

impl<P> Deref for DetailedError<P>
where
    P: ToResponse + Debug,
{
    type Target = InnerError;

    fn deref(&self) -> &Self::Target {
        &self.private
    }
}

/// Create a new error and emit an event with [`tracing::Level::ERROR`]
///
/// This is shorthand for `detailed_error!(Level::ERROR, ...)`
#[macro_export]
macro_rules! e {
    ($private:ident, $public:expr) => {
        $crate::detailed_error!(tracing::Level::ERROR, $private, $public)
    };
    ($private:ident, $public:expr, $ctx:expr) => {
        $crate::detailed_error!(tracing::Level::ERROR, $private, $public, $ctx)
    };
}

/// Create a new error and emit an event with [`tracing::Level::WARN`]
///
/// This is shorthand for `detailed_error!(Level::WARN, ...)`
#[macro_export]
macro_rules! w {
    ($private:ident, $public:expr) => {
        $crate::detailed_error!(tracing::Level::WARN, $private, $public)
    };
    ($private:ident, $public:expr, $ctx:expr) => {
        $crate::detailed_error!(tracing::Level::WARN, $private, $public, $ctx)
    };
}

/// Create a new error and emit an event with with the provided error level
#[macro_export]
macro_rules! detailed_error {
    ($lvl:path, $private:ident, $public:expr) => {
        $crate::DetailedError::new(
            $private,
            $public,
            None,
            $lvl,
            std::file!().to_string(),
            std::line!(),
            std::module_path!().to_string(),
        )
    };
    ($lvl:path, $private:ident, $public:expr, $ctx:expr) => {
        $crate::DetailedError::new(
            $private,
            $public,
            Some($ctx),
            $lvl,
            std::file!().to_string(),
            std::line!(),
            std::module_path!().to_string(),
        )
    };
    ($lvl:path, $private:ident, $public:expr, $ctx:expr, $($k:expr => $v:expr),* $(,)?) => {{
        let mut map: std::collections::HashMap<String, String> = std::convert::From::from([$(($k.to_string(), $v.to_string()),)*]);
        $crate::DetailedError::new_with_tracing(
            $private,
            $public,
            Some($ctx),
            $lvl,
            std::file!().to_string(),
            std::line!(),
            std::module_path!().to_string(),
            map,
        )
    }};
}
