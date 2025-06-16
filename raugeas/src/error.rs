//! Error types for the library

use raugeas_sys::*;
use std::ffi::{NulError, OsString};
use std::string::FromUtf8Error;
use std::{fmt, io};

/// Represents possible errors that can occur.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Augeas error
    Augeas(AugeasError),
    /// Error read from the tree
    Tree(Box<OsTreeError>),
    /// Unexpected nul character
    Nul(NulError),
    /// Error reading a string as UTF-8
    Utf8(FromUtf8Error),
    /// I/O error
    IO(io::Error),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Augeas(ref err) => err.fmt(f),
            Error::Nul(ref err) => err.fmt(f),
            Error::Utf8(ref err) => err.fmt(f),
            Error::Tree(ref err) => err.fmt(f),
            Error::IO(ref err) => err.fmt(f),
        }
    }
}

/// Error coming from the Augeas library
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AugeasError {
    /// The error code representing the error type.
    pub code: ErrorCode,
    /// Return a human-readable message for the error code.
    pub message: Option<String>,
    /// A human-readable message elaborating the error code.
    pub minor_message: Option<String>,
    /// Additional details about the error.
    pub details: Option<String>,
}

impl AugeasError {
    pub(super) fn new_unknown(message: impl Into<String>) -> AugeasError {
        AugeasError {
            code: ErrorCode::Unknown,
            message: Some(message.into()),
            minor_message: None,
            details: None,
        }
    }
}

impl fmt::Display for AugeasError {
    // Write:
    // ```
    // augeas error:{code}:{message}
    //              {minor_message}
    //              {details}
    // ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = self.message.as_ref().map(String::as_ref).unwrap_or("");
        writeln!(f, "augeas error:{:?}:{}", self.code, message)?;

        if let Some(minor_message) = &self.minor_message {
            writeln!(f, "      {}", minor_message)?;
        }

        if let Some(details) = &self.details {
            writeln!(f, "      {}", details)?;
        }

        Ok(())
    }
}

impl Error {
    /// Check if the error is of a specific code from the Augeas library.
    pub fn is_code(&self, code: ErrorCode) -> bool {
        match self {
            Error::Augeas(err) => err.code == code,
            _ => false,
        }
    }
}

impl From<NulError> for Error {
    fn from(err: NulError) -> Error {
        Error::Nul(err)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Error {
        Error::Utf8(err)
    }
}

impl From<AugeasError> for Error {
    fn from(err: AugeasError) -> Error {
        Error::Augeas(err)
    }
}

impl From<OsTreeError> for Error {
    fn from(err: OsTreeError) -> Error {
        Error::Tree(Box::new(err))
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IO(err)
    }
}

/// Error codes from the Augeas library
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Out of memory
    NoMem,
    /// Internal error (bug)
    Internal,
    /// Invalid path expression
    PathExpr,
    /// No match for path expression
    NoMatch,
    /// Too many matches for path expression
    TooManyMatches,
    /// Syntax error in lens file
    Syntax,
    /// Lens lookup failed
    NoLens,
    /// Multiple transforms
    MultipleTransforms,
    /// No span for this node
    NoSpan,
    /// Cannot move node into its descendant
    MoveDescendant,
    /// Failed to execute command
    CmdRun,
    /// Invalid argument in function call
    BadArg,
    /// Invalid label
    Label,
    /// Cannot copy node into its descendant
    CopyDescendant,
    /// Cannot open or read a file
    FileAccess,
    /// Another error
    Unknown,
}

impl ErrorCode {
    /// Match error codes from the library to the enum
    #[allow(non_upper_case_globals)]
    pub(super) fn from_raw(code: aug_errcode_t) -> Option<ErrorCode> {
        Some(match code {
            aug_errcode_t_AUG_NOERROR => return None,
            aug_errcode_t_AUG_ENOMEM => ErrorCode::NoMem,
            aug_errcode_t_AUG_EINTERNAL => ErrorCode::Internal,
            aug_errcode_t_AUG_EPATHX => ErrorCode::PathExpr,
            aug_errcode_t_AUG_ENOMATCH => ErrorCode::NoMatch,
            aug_errcode_t_AUG_EMMATCH => ErrorCode::TooManyMatches,
            aug_errcode_t_AUG_ESYNTAX => ErrorCode::Syntax,
            aug_errcode_t_AUG_ENOLENS => ErrorCode::NoLens,
            aug_errcode_t_AUG_EMXFM => ErrorCode::MultipleTransforms,
            aug_errcode_t_AUG_ENOSPAN => ErrorCode::NoSpan,
            aug_errcode_t_AUG_EMVDESC => ErrorCode::MoveDescendant,
            aug_errcode_t_AUG_ECMDRUN => ErrorCode::CmdRun,
            aug_errcode_t_AUG_EBADARG => ErrorCode::BadArg,
            aug_errcode_t_AUG_ELABEL => ErrorCode::Label,
            aug_errcode_t_AUG_ECPDESC => ErrorCode::CopyDescendant,
            aug_errcode_t_AUG_EFILEACCESS => ErrorCode::FileAccess,
            _ => ErrorCode::Unknown,
        })
    }
}

impl From<ErrorCode> for Error {
    fn from(code: ErrorCode) -> Error {
        Error::Augeas(AugeasError {
            code,
            message: None,
            minor_message: None,
            details: None,
        })
    }
}

/// For some errors (at least `parse_failed`), the error contains the position in the source file
/// where the error occurred.
#[derive(Clone, PartialEq, Debug)]
pub struct ErrorPosition {
    /// Line number
    pub line: usize,
    /// Character number
    pub char: usize,
    /// Byte position
    pub position: usize,
}

/// An error read from inside the tree.
///
/// Tries to collect as much information as possible.
///
/// The error will show up underneath `/augeas/FILENAME/error` if there is a filename, and underneath
/// `/augeas/text/PATH` otherwise. `PATH` is the path to the toplevel node in
/// the tree where the lens application happened.
#[derive(Clone, PartialEq, Debug)]
pub struct OsTreeError {
    /// Error kind
    pub kind: String,
    /// Human-readable error message
    pub message: OsString,
    /// Path to tree node where error occurred
    pub path: Option<OsString>,
    /// Position in the file where error occurred
    pub position: Option<ErrorPosition>,
    /// The lens where the error occurred
    pub lens: Option<String>,
}

impl From<OsTreeError> for TreeError {
    fn from(err: OsTreeError) -> TreeError {
        TreeError {
            kind: err.kind,
            message: err.message.to_string_lossy().into_owned(),
            path: err.path.map(|p| p.to_string_lossy().into_owned()),
            position: err.position,
            lens: err.lens,
        }
    }
}

impl From<Box<OsTreeError>> for TreeError {
    fn from(err: Box<OsTreeError>) -> TreeError {
        (*err).into()
    }
}

/// An error read from inside the tree.
///
/// Tries to collect as much information as possible.
///
/// The error will show up underneath `/augeas/FILENAME/error` if there is a filename, and underneath
/// `/augeas/text/PATH` otherwise. `PATH` is the path to the toplevel node in
/// the tree where the lens application happened.
#[derive(Clone, PartialEq, Debug)]
pub struct TreeError {
    /// Error kind
    pub kind: String,
    /// Human-readable error message
    pub message: String,
    /// Path to tree node where error occurred
    pub path: Option<String>,
    /// Position in the file where error occurred
    pub position: Option<ErrorPosition>,
    /// The lens where the error occurred
    pub lens: Option<String>,
}

impl fmt::Display for OsTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "augeas error:{}:{}",
            self.kind,
            self.message.to_string_lossy()
        )?;
        if let Some(path) = &self.path {
            write!(f, " at {}", path.to_string_lossy())?;
        }
        if let Some(position) = &self.position {
            write!(
                f,
                " at {}:{}:{}",
                position.line, position.char, position.position
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "augeas error:{}:{}", self.kind, self.message)?;
        if let Some(path) = &self.path {
            write!(f, " at {}", path)?;
        }
        if let Some(position) = &self.position {
            write!(
                f,
                " at {}:{}:{}",
                position.line, position.char, position.position
            )?;
        }
        Ok(())
    }
}
