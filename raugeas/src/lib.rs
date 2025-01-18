//! Augeas bindings for Rust
//!
//! [Augeas](http://augeas.net/) is a library for reading, modifying, and writing a structured file,
//! like configuration files.
//!
//! This library is a low-level binding to the C API of Augeas, with a few abstractions to make it
//! more idiomatic to use in Rust. It does not aim to provide a high-level API to manipulate
//! configuration files, but rather to provide a safe and idiomatic way to interact with Augeas.
//!
//! The main differences with the C API are:
//!
//! * Add `clear` and `clearm` methods clear values (instead of passing an `Option` to `set`/`setm`).
//! * Add `touch` method to create a node if it does not exist.
//! * Use a `Span` struct to represent the span of a node in a file.
//! * Use a `Position` enum to indicate where to insert a new node.
//! * Use an `Attr` struct to represent the attributes of a node.
//! * Use a `SaveMode` enum to indicate how to save changes.
//!
//! ## Usage
//!
//! In `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! raugeas = "0.3.1"
//! ```
//!
//! ## Summary
//!
//! A typical interaction looks like this:
//!
//! ```
//! use raugeas::{Augeas, Flags};
//!
//! let mut aug = Augeas::init(Some("/"), "", Flags::NONE).unwrap();
//!
//! // Get the ip address for host.example.com from `/etc/hosts`.
//! let entry = aug.get("etc/hosts/*[canonical = 'host.example.com']/ip")?;
//! if let Some(ip) = entry {
//!     println!("The ip for host.example.com is {}", ip);
//! } else {
//!     println!("There is no entry for host.example.com in /etc/hosts");
//! }
//!
//! // Add an alias for host.example.com.
//! aug.set(
//!     "etc/hosts/*[canonical = 'host.example.com']/alias[last()+1]",
//!     "server.example.com",
//! )?;
//! # Ok::<(), raugeas::Error>(())
//! ```

// References for the Rust implementation:
//
// * https://github.com/hercules-team/augeas/blob/master/src/augeas.h
// * https://github.com/hercules-team/ruby-augeas/tree/master/lib

#![warn(rust_2018_idioms, unused_qualifications, missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate bitflags;

use raugeas_sys::*;
use std::borrow::Cow;
use std::convert::From;
use std::ffi::{CString, OsStr, OsString};
use std::fmt::Display;
use std::mem::transmute;
use std::ops::Range;
use std::os::raw::{c_char, c_int};
use std::os::unix::prelude::{OsStrExt, OsStringExt};
use std::str::FromStr;
use std::{fmt, ptr};

pub mod error;
use error::AugeasError;
pub use error::Error;
use error::ErrorCode;

mod flags;
pub use self::flags::Flags;

mod util;
use crate::error::{ErrorPosition, OsTreeError, TreeError};
use crate::util::{build_path, new_memstream, ptr_to_os_string, read_memstream};
use util::ptr_to_string;

/// Shortcut for the result type used in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The handle for the Augeas library.
///
/// The Augeas handle points to the in-memory data that Augeas manages, in
/// particular, the tree generated from parsing configuration files.
pub struct Augeas {
    ptr: *mut augeas,
}

/// Parameters for the save modes.
///
/// After calling [`save`](#method.save), the list of files that has been changed can be found in the nodes.
/// `/augeas/events/saved`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// Perform all the steps necessary for a file, including generating the contents of files from
    /// the tree, but do not make any changes in the filesystem. This mode is very useful for making
    /// sure that all changes can be saved without error, and to find out which files would actually
    /// be modified.
    Noop,
    /// Overwrite files in place.
    Overwrite,
    /// Write the modified file into a file with extension `.augnew` without modifying the original file.
    NewFile,
    /// Save the original file with the extension `.augsave` and write the new file under the original filename.
    Backup,
}

impl Display for SaveMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveMode::Noop => write!(f, "noop"),
            SaveMode::Overwrite => write!(f, "overwrite"),
            SaveMode::NewFile => write!(f, "newfile"),
            SaveMode::Backup => write!(f, "backup"),
        }
    }
}

/// The insert position.
///
/// Use this enum with [`insert`](#method.insert) to indicate whether the
/// new node should be inserted before or after the node passed to
/// [`insert`](#method.insert).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// Insert the new node before the node passed to [`insert`](#method.insert)
    Before,
    /// Insert the new node after the node passed to [`insert`](#method.insert)
    After,
}

impl FromStr for Position {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "before" => Ok(Position::Before),
            "after" => Ok(Position::After),
            _ => Err(Error::Augeas(AugeasError::new_unknown(format!(
                "Invalid position: {}",
                s
            )))),
        }
    }
}

impl From<Position> for c_int {
    fn from(pos: Position) -> Self {
        match pos {
            Position::Before => 1,
            Position::After => 0,
        }
    }
}

/// A span in the tree.
///
/// The [`span`](#method.span) indicates where in a file a particular node
/// was found. The `label` and `value` give the character range from which
/// the node's label and value were read, and `span` gives the entire region
/// in the file from which the node was constructed. If any of these values are
/// not available, e.g., because the node has no value, the corresponding range
/// will be empty.
///
/// The `filename` provides the entire path to the file in the file system.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Span {
    /// Character range for the label of the node in a file.
    ///
    /// Empty if not available.
    pub label: Range<u32>,
    /// Character range for the value of the node in a file.
    ///
    /// Empty if not available.
    pub value: Range<u32>,
    /// Character range for the entire node in a file.
    //
    /// Empty if not available.
    pub span: Range<u32>,
    /// The path to the file in the file system.
    ///
    /// `None` if the node is not associated with a file.
    pub filename: Option<String>,
}

/// A span in the tree.
///
/// The [`span`](#method.span) indicates where in a file a particular node
/// was found. The `label` and `value` give the character range from which
/// the node's label and value were read, and `span` gives the entire region
/// in the file from which the node was constructed. If any of these values are
/// not available, e.g., because the node has no value, the corresponding range
/// will be empty.
///
/// The `filename` provides the entire path to the file in the file system.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct OsSpan {
    /// Character range for the label of the node in a file.
    ///
    /// Empty if not available.
    pub label: Range<u32>,
    /// Character range for the value of the node in a file.
    ///
    /// Empty if not available.
    pub value: Range<u32>,
    /// Character range for the entire node in a file.
    //
    /// Empty if not available.
    pub span: Range<u32>,
    /// The path to the file in the file system.
    ///
    /// `None` if the node is not associated with a file.
    pub filename: Option<OsString>,
}

impl TryFrom<OsSpan> for Span {
    type Error = Error;

    fn try_from(span: OsSpan) -> std::result::Result<Self, Self::Error> {
        let filename = match span.filename.map(|f| String::from_utf8(f.into_vec())) {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => return Err(e.into()),
            None => None,
        };

        Ok(Span {
            label: span.label,
            value: span.value,
            span: span.span,
            filename,
        })
    }
}

/// Attributes of a node
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attr {
    /// The label of the node
    pub label: Option<String>,
    /// The value of the node
    pub value: Option<String>,
    /// The path of the file the node belongs to, or `None` if the node does not belong to a file.
    pub file_path: Option<String>,
}

/// Attributes of a node
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OsAttr {
    /// The label of the node
    pub label: Option<OsString>,
    /// The value of the node
    pub value: Option<OsString>,
    /// The path of the file the node belongs to, or `None` if the node does not belong to a file.
    pub file_path: Option<OsString>,
}

impl TryFrom<OsAttr> for Attr {
    type Error = Error;

    fn try_from(attr: OsAttr) -> std::result::Result<Self, Self::Error> {
        let label = match attr.label.map(|f| String::from_utf8(f.into_vec())) {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => return Err(e.into()),
            None => None,
        };
        let value = match attr.value.map(|f| String::from_utf8(f.into_vec())) {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => return Err(e.into()),
            None => None,
        };
        let file_path = match attr.file_path.map(|f| String::from_utf8(f.into_vec())) {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => return Err(e.into()),
            None => None,
        };

        Ok(Attr {
            label,
            value,
            file_path,
        })
    }
}

/// Outcome of a successful [`srun`](#method.srun) command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommandsNumber {
    /// A `quit` command was issued.
    Quit,
    /// Number of commands that were successful.
    Success(u32),
}

impl Augeas {
    /// Create a new Augeas handle.
    ///
    /// Use `root` as the filesystem root. If `root` is `None`, use the value
    /// of the environment variable `AUGEAS_ROOT` if it is set; otherwise,
    /// use `/`.
    ///
    /// The `loadpath` is a colon-separated list of directories that modules
    /// should be searched in. This is in addition to the standard load path
    /// and the directories listed in the environment variable `AUGEAS_LENS_LIB`.
    ///
    /// The `flags` are a bitmask of values from `Flags`.
    pub fn init<T: AsRef<OsStr>, S: AsRef<OsStr>>(
        root: Option<T>,
        loadpath: S,
        flags: Flags,
    ) -> Result<Self> {
        let root = &match root {
            Some(r) => {
                let r = r.as_ref();
                Some(CString::new(r.as_bytes())?)
            }
            None => None,
        };
        let root = match root {
            Some(r) => r.as_ptr(),
            None => ptr::null(),
        };
        let load_path = loadpath.as_ref();
        let load_path = CString::new(load_path.as_bytes())?;
        let load_path = load_path.as_ptr();

        // We want to get errors to help the user understand what went wrong if possible.
        // We force error checking after to prevent access to a broken tree.
        let flags = flags | Flags::NO_ERROR_CLOSE;
        let flags = flags.bits();

        let augeas = unsafe { aug_init(root, load_path, flags) };

        if augeas.is_null() {
            let message = String::from("Failed to initialize Augeas");
            return Err(Error::Augeas(AugeasError::new_unknown(message)));
        }
        let res = Augeas { ptr: augeas };

        // As we set `NO_ERROR_CLOSE`, we check for errors here.
        res.check_error()?;

        Ok(res)
    }

    /// Get the version number of the Augeas library
    pub fn version(&self) -> Result<String> {
        Ok(self.get("/augeas/version")?.unwrap_or_default())
    }

    /// Lookup the value associated with `path`.
    ///
    /// Return `None` if there is no value associated with `path` or if no
    /// node matches `path`, and `Some(value)` if there is exactly one node
    /// with a value.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn get_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<OsString>> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;
        let path = path.as_ptr();
        let mut value: *const c_char = ptr::null_mut();

        unsafe { aug_get(self.ptr, path, &mut value) };
        self.check_error()?;

        let value = ptr_to_os_string(value);

        Ok(value)
    }

    /// Lookup the value associated with `path`.
    ///
    /// Return `None` if there is no value associated with `path` or if no
    /// node matches `path`, and `Some(value)` if there is exactly one node
    /// with a value.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn get<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<String>> {
        match self.get_os(path) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Lookup the label associated with `path`.
    ///
    /// Return `Some(label)` if `path` matches a node that has a label, and
    /// `None` if `path` matches no node, or matches exactly one node
    /// without a label.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn label_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<OsString>> {
        let path = path.as_ref();
        let path_c = CString::new(path.as_bytes())?;
        let mut value: *const c_char = ptr::null();

        unsafe { aug_label(self.ptr, path_c.as_ptr(), &mut value) };
        self.check_error()?;

        let value = ptr_to_os_string(value);

        Ok(value)
    }

    /// Lookup the label associated with `path`.
    ///
    /// Return `Some(label)` if `path` matches a node that has a label, and
    /// `None` if `path` matches no node, or matches exactly one node
    /// without a label.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn label<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<String>> {
        match self.label_os(path) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Find all nodes matching `path`.
    ///
    /// Find all nodes matching the path expression `path` and return their
    /// paths in an unambiguous form that can be used with
    /// [`get`](#method.get) to get their value.
    pub fn matches_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Vec<OsString>> {
        let path = path.as_ref();
        let c_path = CString::new(path.as_bytes())?;

        unsafe {
            let mut matches_ptr: *mut *mut c_char = ptr::null_mut();
            let num_matches = aug_match(self.ptr, c_path.as_ptr(), &mut matches_ptr);
            self.check_error()?;

            let matches_vec = (0..num_matches)
                .map(|i| {
                    let match_ptr: *const c_char = transmute(*matches_ptr.offset(i as isize));
                    let str = ptr_to_os_string(match_ptr).unwrap();
                    libc::free(transmute::<*const c_char, *mut libc::c_void>(match_ptr));
                    str
                })
                .collect::<Vec<OsString>>();

            libc::free(transmute::<*mut *mut c_char, *mut libc::c_void>(
                matches_ptr,
            ));

            Ok(matches_vec)
        }
    }

    /// Find all nodes matching `path`.
    ///
    /// Same as `matches`, but returns a `String`. Fails on non-UTF-8 values.
    ///
    /// Find all nodes matching the path expression `path` and return their
    /// paths in an unambiguous form that can be used with
    /// [`get`](#method.get) to get their value.
    pub fn matches<T: AsRef<OsStr>>(&self, path: T) -> Result<Vec<String>> {
        match self.matches_os(path) {
            Ok(matches) => {
                let matches = matches
                    .into_iter()
                    .map(|m| String::from_utf8(m.into_vec()).map_err(|e| e.into()))
                    .collect::<Result<Vec<String>>>()?;
                Ok(matches)
            }
            Err(e) => Err(e),
        }
    }

    /// Count the nodes matching `path`
    ///
    /// Find all nodes matching the path expression `path` and return how
    /// many there are.
    pub fn count<T: AsRef<OsStr>>(&self, path: T) -> Result<u32> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;

        let r = unsafe { aug_match(self.ptr, path.as_ptr(), ptr::null_mut()) };
        self.check_error()?;

        Ok(r as u32)
    }

    /// Save all pending changes to disk.
    ///
    /// Turn all files in the tree for which entries have been changed,
    /// added, or deleted back into text and write them back to file.
    ///
    /// If `SAVE_NEWFILE` is set in the `Flags` passed to [`init`](#method.init), create
    /// changed files as new files with the extension `.augnew`, and leave the
    /// original file unmodified.
    ///
    /// Otherwise, if `SAVE_BACKUP` is set in the `Flags` passed to [`init`](#method.init),
    /// move the original file to a new file with extension `.augsave`.
    ///
    /// If neither of these flags is set, overwrite the original file.
    pub fn save(&mut self) -> Result<()> {
        unsafe { aug_save(self.ptr) };
        self.check_error()?;

        Ok(())
    }

    /// Set the value of a node.
    ///
    /// Find the node matching `path` and change its value. If there is no
    /// such node, an attempt is made to create one, though that might not
    /// be possible depending on the complexity of the path expression
    /// `path`.
    ///
    /// It is an error if more than one node matches `path`.
    pub fn set<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, path: T, value: S) -> Result<()> {
        let path = path.as_ref();
        let value = value.as_ref();

        let path_c = CString::new(path.as_bytes())?;
        let value_c = CString::new(value.as_bytes())?;

        unsafe { aug_set(self.ptr, path_c.as_ptr(), value_c.as_ptr()) };
        self.check_error()?;

        Ok(())
    }

    /// Clear the value of a node.
    ///
    /// Find the node matching `path` and clear its value.
    ///
    /// It is an error if more than one node matches `path`.
    pub fn clear<T: AsRef<OsStr>>(&mut self, path: T) -> Result<()> {
        let path = path.as_ref();
        let path_c = CString::new(path.as_bytes())?;

        unsafe { aug_set(self.ptr, path_c.as_ptr(), ptr::null()) };
        self.check_error()?;

        Ok(())
    }

    /// Create the `path` with empty value if it doesn't exist.
    pub fn touch<T: AsRef<OsStr>>(&mut self, path: T) -> Result<()> {
        let path = path.as_ref();
        let path_c = CString::new(path.as_bytes())?;

        let no_matches = self.matches_os(path)?.is_empty();
        if no_matches {
            unsafe { aug_set(self.ptr, path_c.as_ptr(), ptr::null()) };
        }
        self.check_error()?;

        Ok(())
    }

    /// Insert a new node: find the node matching `path` and create a new
    /// sibling for it with the given `label`. The sibling is created
    /// before or after the node `path`, depending on the value of `pos`.
    ///
    /// It is an error if `path` matches no nodes, or more than one
    /// node. The `label` must not contain a `/`, `*` or end with a
    /// bracketed index `[N]`.
    pub fn insert<T: AsRef<OsStr>, S: AsRef<OsStr>>(
        &mut self,
        path: T,
        label: S,
        pos: Position,
    ) -> Result<()> {
        let path = path.as_ref();
        let label = label.as_ref();

        let path = CString::new(path.as_bytes())?;
        let label = CString::new(label.as_bytes())?;

        unsafe { aug_insert(self.ptr, path.as_ptr(), label.as_ptr(), c_int::from(pos)) };
        self.check_error()?;

        Ok(())
    }

    /// Remove `path` and all its children and return the number of nodes
    /// removed.
    pub fn rm<T: AsRef<OsStr>>(&mut self, path: T) -> Result<u32> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;
        let r = unsafe { aug_rm(self.ptr, path.as_ptr()) };
        self.check_error()?;
        // coercing i32 to u32 is fine here since r is only negative
        // when an error occurred and `check_error` notices that from
        // the result of aug_error
        Ok(r as u32)
    }

    /// Move the node matching `src` to `dst`.
    ///
    /// `src` must match exactly one node in the tree.
    /// `dst` must either match exactly one node in the tree or may not
    /// exist yet. If `dst` exists already, it and all its descendants are
    /// deleted. If `dst` does not exist yet, it and all its missing ancestors are
    /// created.
    ///
    /// Note that the node `src` always becomes the node `dst`: when you move `/a/b`
    /// to `/x`, the node `/a/b` is now called `/x`, no matter whether `/x` existed
    /// initially or not.
    pub fn mv<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, src: T, dst: S) -> Result<()> {
        let src = src.as_ref();
        let dst = dst.as_ref();

        let src = CString::new(src.as_bytes())?;
        let dst = CString::new(dst.as_bytes())?;

        unsafe { aug_mv(self.ptr, src.as_ptr(), dst.as_ptr()) };
        self.check_error()?;

        Ok(())
    }

    /// Define a variable `name` whose value is the result of evaluating
    /// `expr`. If a variable `name` already exists, its name will be
    /// replaced with the result of evaluating `expr`. Context will not be
    /// applied to `expr`.
    ///
    /// Path variables can be used in path expressions later on by prefixing
    /// them with '$'.
    pub fn defvar<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, name: T, expr: S) -> Result<()> {
        let name = name.as_ref();
        let expr = expr.as_ref();

        let name = CString::new(name.as_bytes())?;
        let expr = CString::new(expr.as_bytes())?;

        unsafe { aug_defvar(self.ptr, name.as_ptr(), expr.as_ptr()) };
        self.check_error()?;

        Ok(())
    }

    /// Remove the variable `name`.
    ///
    /// It is not an error if the variable does not exist.
    pub fn rmvar<T: AsRef<OsStr>>(&mut self, name: T) -> Result<()> {
        let name = name.as_ref();
        let name = CString::new(name.as_bytes())?;

        unsafe { aug_defvar(self.ptr, name.as_ptr(), ptr::null_mut()) };
        self.check_error()?;

        Ok(())
    }

    /// Define a variable `name` whose value is the result of evaluating `expr`,
    /// which must evaluate to a nodeset. If a variable `name`
    /// already exists, its name will be replaced with the result of evaluating
    /// `expr`.
    ///
    /// If `expr` evaluates to an empty nodeset, a node is created,
    /// equivalent to calling [`set(expr, value)`](#method.set) and `name`
    /// will be the nodeset containing that single node.
    ///
    /// If a node was created, the method returns `true`, and `false` if no
    /// node was created.
    pub fn defnode<T: AsRef<OsStr>, S: AsRef<OsStr>, V: AsRef<OsStr>>(
        &mut self,
        name: T,
        expr: S,
        value: V,
    ) -> Result<bool> {
        let name = name.as_ref();
        let expr = expr.as_ref();
        let value = value.as_ref();

        let name = CString::new(name.as_bytes())?;
        let expr = CString::new(expr.as_bytes())?;
        let value = CString::new(value.as_bytes())?;
        let mut cr: i32 = 0;

        unsafe {
            aug_defnode(
                self.ptr,
                name.as_ptr(),
                expr.as_ptr(),
                value.as_ptr(),
                &mut cr,
            )
        };
        self.check_error()?;

        Ok(cr == 1)
    }

    /// Load files into the tree.
    ///
    /// Which files to load and what lenses to use on
    /// them is specified under `/augeas/load` in the tree; each entry
    /// `/augeas/load/NAME` specifies a 'transform', by having itself exactly one
    /// child `lens` and any number of children labelled `incl` and `excl`. The
    /// value of `NAME` has no meaning.
    ///
    /// The `lens` grandchild of `/augeas/load` specifies which lens to use, and
    /// can either be the fully qualified name of a lens `Module.lens` or
    /// `@Module`. The latter form means that the lens from the transform marked
    /// for autoloading in `MODULE` should be used.
    ///
    /// The `incl` and `excl` grandchildren of `/augeas/load` indicate which files
    /// to transform. Their values are used as glob patterns. Any file that
    /// matches at least one `incl` pattern and no `excl` pattern is
    /// transformed. The order of `incl` and `excl` entries is irrelevant.
    ///
    /// When [`init`](#method.init) is first called, it populates `/augeas/load` with the
    /// transforms marked for autoloading in all the modules it finds.
    ///
    /// Before loading any files, `load` will remove everything underneath
    /// `/augeas/files` and `/files`, regardless of whether any entries have been
    /// modified or not.
    ///
    /// Note that success includes the case
    /// where some files could not be loaded. Details of such files can be found
    /// as `/augeas//error`.
    pub fn load(&mut self) -> Result<()> {
        unsafe { aug_load(self.ptr) };
        self.check_error()?;

        Ok(())
    }

    /// Set the value of multiple nodes in one operation. Find or create a node
    /// matching `sub` by interpreting `sub` as a path expression relative to each
    /// node matching `base`.
    pub fn setm<T: AsRef<OsStr>, S: AsRef<OsStr>, V: AsRef<OsStr>>(
        &mut self,
        base: T,
        sub: S,
        value: V,
    ) -> Result<u32> {
        let base = base.as_ref();
        let sub = sub.as_ref();
        let value = value.as_ref();

        let base = CString::new(base.as_bytes())?;
        let sub = CString::new(sub.as_bytes())?;
        let value = CString::new(value.as_bytes())?;

        let r = unsafe { aug_setm(self.ptr, base.as_ptr(), sub.as_ptr(), value.as_ptr()) };
        self.check_error()?;

        Ok(r as u32)
    }

    /// Clear the value of multiple nodes in one operation.
    ///
    /// Clear nodes matching `sub` by interpreting `sub` as a path expression relative to each
    /// node matching `base`.
    pub fn clearm<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, base: T, sub: S) -> Result<u32> {
        let base = base.as_ref();
        let sub = sub.as_ref();

        let base = CString::new(base.as_bytes())?;
        let sub = CString::new(sub.as_bytes())?;

        let r = unsafe { aug_setm(self.ptr, base.as_ptr(), sub.as_ptr(), ptr::null()) };
        self.check_error()?;

        Ok(r as u32)
    }

    /// Get the span according to the input file of the node associated with `path`. If
    /// the node is associated with a file, the span is returned.
    pub fn span_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<OsSpan>> {
        let path = path.as_ref();

        let path = CString::new(path.as_bytes())?;
        let mut filename: *mut c_char = ptr::null_mut();
        let mut result = OsSpan::default();

        unsafe {
            aug_span(
                self.ptr,
                path.as_ptr(),
                &mut filename,
                &mut result.label.start,
                &mut result.label.end,
                &mut result.value.start,
                &mut result.value.end,
                &mut result.span.start,
                &mut result.span.end,
            );
        }

        let err = unsafe { aug_error(self.ptr) };
        let err = ErrorCode::from_raw(err as _);
        if err == Some(ErrorCode::NoSpan) {
            return Ok(None);
        }
        self.check_error()?;

        result.filename = ptr_to_os_string(filename);
        unsafe { libc::free(filename as *mut libc::c_void) };
        Ok(Some(result))
    }

    /// Get the span according to the input file of the node associated with `path`. If
    /// the node is associated with a file, the span is returned.
    pub fn span<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<Span>> {
        match self.span_os(path) {
            Ok(Some(s)) => Ok(Some(s.try_into()?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Use the value of node `node` as a string and transform it into a tree
    /// using the lens `lens` and store it in the tree at `path`, which will be
    /// overwritten. `path` and `node` are path expressions.
    pub fn text_store<T: AsRef<OsStr>, S: AsRef<OsStr>, V: AsRef<OsStr>>(
        &mut self,
        lens: T,
        node: S,
        path: V,
    ) -> Result<()> {
        let lens = lens.as_ref();
        let node = node.as_ref();
        let path = path.as_ref();

        let lens = CString::new(lens.as_bytes())?;
        let node = CString::new(node.as_bytes())?;
        let c_path = CString::new(path.as_bytes())?;

        unsafe { aug_text_store(self.ptr, lens.as_ptr(), node.as_ptr(), c_path.as_ptr()) };
        self.check_error()?;

        let mut err_path = OsString::from("/augeas/text");
        err_path.push(path);

        self.check_tree_error(err_path)?;

        Ok(())
    }

    /// Transform the tree at `path` into a string using lens `lens` and store it in
    /// the node `node_out`, assuming the tree was initially generated using the
    /// value of node `node_in`. `path`, `node_in` and `node_out` are path expressions.
    pub fn text_retrieve<T: AsRef<OsStr>, S: AsRef<OsStr>, V: AsRef<OsStr>, R: AsRef<OsStr>>(
        &mut self,
        lens: T,
        node_in: S,
        path: V,
        node_out: R,
    ) -> Result<()> {
        let lens = lens.as_ref();
        let node_in = node_in.as_ref();
        let path = path.as_ref();
        let node_out = node_out.as_ref();

        let lens = CString::new(lens.as_bytes())?;
        let node_in = CString::new(node_in.as_bytes())?;
        let c_path = CString::new(path.as_bytes())?;
        let node_out = CString::new(node_out.as_bytes())?;

        unsafe {
            aug_text_retrieve(
                self.ptr,
                lens.as_ptr(),
                node_in.as_ptr(),
                c_path.as_ptr(),
                node_out.as_ptr(),
            )
        };
        self.check_error()?;

        let mut err_path = OsString::from("/augeas/text");
        err_path.push(path);

        self.check_tree_error(err_path)?;

        Ok(())
    }

    /// Rename the label of all nodes matching `src` to `label`.
    ///
    /// Returns the number of nodes renamed.
    pub fn rename<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, src: T, label: S) -> Result<u32> {
        let src = src.as_ref();
        let lbl = label.as_ref();

        let src = CString::new(src.as_bytes())?;
        let lbl = CString::new(lbl.as_bytes())?;

        let r = unsafe { aug_rename(self.ptr, src.as_ptr(), lbl.as_ptr()) };
        self.check_error()?;

        Ok(r as u32)
    }

    /// Add a transform for `file` using `lens`.
    ///
    /// `excl` specifies if this the file is to be included or excluded from the `lens`.
    /// The `lens` may be a module name, or a full lens name.
    //  If a module name is given, then lns will be the lens assumed.
    pub fn transform<T: AsRef<OsStr>, S: AsRef<OsStr>>(
        &mut self,
        lens: T,
        file: S,
        excl: bool,
    ) -> Result<()> {
        let lens = lens.as_ref();
        let file = file.as_ref();

        let lens = CString::new(lens.as_bytes())?;
        let file = CString::new(file.as_bytes())?;

        unsafe { aug_transform(self.ptr, lens.as_ptr(), file.as_ptr(), excl as i32) };
        self.check_error()?;

        Ok(())
    }

    /// Copy the node `src` to `dst`.
    ///
    /// `src` must match exactly one node in the
    /// tree. `dst` must either match exactly one node in the tree or may not
    /// exist yet. If `dst` exists already, it and all its descendants are
    /// deleted. If `dst` does not exist yet, it and all its missing ancestors are
    /// created.
    pub fn cp<T: AsRef<OsStr>, S: AsRef<OsStr>>(&mut self, src: T, dst: S) -> Result<()> {
        let src = src.as_ref();
        let dst = dst.as_ref();

        let src = CString::new(src.as_bytes())?;
        let dst = CString::new(dst.as_bytes())?;

        unsafe { aug_cp(self.ptr, src.as_ptr(), dst.as_ptr()) };
        self.check_error()?;

        Ok(())
    }

    /// Escape special characters in a string such that it can be used as part
    /// of a path expressions and only matches a node named exactly
    /// `inp`. Characters that have special meanings in path expressions, such as
    /// `[` and `]` are prefixed with a `\\`. Note that this function assumes
    /// that it is passed a name, not a path, and will therefore escape `/`,
    /// too.
    ///
    /// Returns `None` if `inp` does not need any escaping at all.
    pub fn escape_name_os<T: AsRef<OsStr>>(&self, inp: T) -> Result<Option<OsString>> {
        let inp = inp.as_ref();
        let inp = CString::new(inp.as_bytes())?;
        let mut out: *mut c_char = ptr::null_mut();

        unsafe { aug_escape_name(self.ptr, inp.as_ptr(), &mut out) };

        let s = ptr_to_os_string(out);
        unsafe { libc::free(out as *mut libc::c_void) };
        self.check_error()?;

        Ok(s)
    }

    /// Escape special characters in a string such that it can be used as part
    /// of a path expressions and only matches a node named exactly
    /// `inp`. Characters that have special meanings in path expressions, such as
    /// `[` and `]` are prefixed with a `\\`. Note that this function assumes
    /// that it is passed a name, not a path, and will therefore escape `/`,
    /// too.
    pub fn escape_name<'a>(&self, inp: &'a str) -> Result<Cow<'a, str>> {
        match self.escape_name_os(inp) {
            Ok(Some(p)) => Ok(String::from_utf8(p.into_vec())?.into()),
            Ok(None) => Ok(inp.into()),
            Err(e) => Err(e),
        }
    }

    /// Load a `file` using the lens that would ordinarily be used by [`load`](#method.load),
    /// i.e. the lens whose autoload statement matches the `file`. Similar to
    /// [`load`](#method.load), this function returns successfully even if `file` does not exist
    /// or if the `file` cannot be processed by the associated lens. It is an
    /// error though if no lens can be found to process `file`.
    pub fn load_file<T: AsRef<OsStr>>(&mut self, file: T) -> Result<()> {
        let file = file.as_ref();
        let file = CString::new(file.as_bytes())?;

        unsafe { aug_load_file(self.ptr, file.as_ptr()) };
        self.check_error()?;

        Ok(())
    }

    /// For the node matching `path`, return the path to the node representing the
    /// file to which `path` belongs.
    ///
    /// Returns `None` if `path` does not match any nodes.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn source_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<OsString>> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;
        let mut file_path: *mut c_char = ptr::null_mut();

        unsafe { aug_source(self.ptr, path.as_ptr(), &mut file_path) };
        let s = ptr_to_os_string(file_path);
        unsafe { libc::free(file_path as *mut libc::c_void) };
        self.check_error()?;

        Ok(s)
    }

    /// For the node matching `path`, return the path to the node representing the
    /// file to which `path` belongs.
    ///
    /// Returns `None` if `path` does not match any nodes.
    ///
    /// It is an error if `path` matches more than one node.
    pub fn source<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<String>> {
        match self.source_os(path) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Return the contents of the file that would be written for the file associated with `path`.
    ///
    /// If there is no file corresponding to `path`, it returns `None`.
    pub fn preview_os<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<OsString>> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;
        let mut out: *mut c_char = ptr::null_mut();

        unsafe { aug_preview(self.ptr, path.as_ptr(), &mut out) };

        let s = ptr_to_os_string(out);
        unsafe { libc::free(out as *mut libc::c_void) };
        self.check_error()?;

        Ok(s)
    }

    /// Return the contents of the file that would be written for the file associated with `path`.
    ///
    /// If there is no file corresponding to `path`, it returns `None`.
    pub fn preview<T: AsRef<OsStr>>(&self, path: T) -> Result<Option<String>> {
        match self.preview_os(path) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Look up the `i`th node in the variable `var` and retrieve information about
    /// it.
    ///
    /// It is assumed that `var` was defined with a path expression evaluating to
    /// a nodeset, like `/files/etc/hosts//*`.
    ///
    /// If `var` does not exist, or is not a nodeset, or if it has fewer than `i`
    /// nodes, this call fails.
    pub fn ns_attr_os<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<OsAttr> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let mut value: *const c_char = ptr::null_mut();
        let mut label: *const c_char = ptr::null_mut();
        let mut file_path: *mut c_char = ptr::null_mut();

        let rc = unsafe {
            aug_ns_attr(
                self.ptr,
                var.as_ptr(),
                i as c_int,
                &mut value,
                &mut label,
                &mut file_path,
            )
        };
        if rc < 0 {
            self.check_error()?;
        }

        let attr = OsAttr {
            label: ptr_to_os_string(label),
            value: ptr_to_os_string(value),
            file_path: ptr_to_os_string(file_path),
        };

        unsafe { libc::free(file_path as *mut libc::c_void) };
        self.check_error()?;

        Ok(attr)
    }

    /// Look up the `i`th node in the variable `var` and retrieve information about
    /// it.
    ///
    /// It is assumed that `var` was defined with a path expression evaluating to
    /// a nodeset, like `/files/etc/hosts//*`.
    ///
    /// If `var` does not exist, or is not a nodeset, or if it has fewer than `i`
    /// nodes, this call fails.
    pub fn ns_attr<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<Attr> {
        match self.ns_attr_os(var, i) {
            Ok(a) => Ok(a.try_into()?),
            Err(e) => Err(e),
        }
    }

    /// Look up the label among its siblings for the `i`th node in the variable `var`.
    pub fn ns_label_os<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<OsString> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let mut label: *const c_char = ptr::null_mut();

        let rc = unsafe {
            aug_ns_label(
                self.ptr,
                var.as_ptr(),
                i as c_int,
                &mut label,
                ptr::null_mut(),
            )
        };
        if rc < 0 {
            self.check_error()?;
        }

        match ptr_to_os_string(label) {
            Some(label) => Ok(label),
            None => Err(Error::from(ErrorCode::NoMatch)),
        }
    }

    /// Look up the label among its siblings for the `i`th node in the variable `var`.
    pub fn ns_label<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<String> {
        match self.ns_label_os(var, i) {
            Ok(p) => Ok(String::from_utf8(p.into_vec())?),
            Err(e) => Err(e),
        }
    }

    /// Look up the index of the `i`th node in the variable `var` among its siblings.
    ///
    /// The `index` will be set to the number of siblings + 1 of the node `var[i+1]` that precede it.
    /// If the node `var[i+1]` does not have any siblings with the same label as
    //  itself, `index` will be set to 0.
    pub fn ns_index<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<u32> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let mut index: c_int = 0;

        unsafe {
            aug_ns_label(
                self.ptr,
                var.as_ptr(),
                i as c_int,
                ptr::null_mut(),
                &mut index,
            )
        };
        self.check_error()?;

        Ok(index as u32)
    }

    /// Look up the value of the `i`th node in variable `var`.
    pub fn ns_value_os<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<Option<OsString>> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let mut value: *const c_char = ptr::null_mut();
        unsafe { aug_ns_value(self.ptr, var.as_ptr(), i as c_int, &mut value) };

        self.check_error()?;

        Ok(ptr_to_os_string(value))
    }

    /// Look up the value of the `i`th node in variable `var`.
    pub fn ns_value<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<Option<String>> {
        match self.ns_value_os(var, i) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Return the number of nodes in variable `var`.
    pub fn ns_count<T: AsRef<OsStr>>(&self, var: T) -> Result<u32> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let rc = unsafe { aug_ns_count(self.ptr, var.as_ptr()) };
        self.check_error()?;

        Ok(rc as u32)
    }

    /// Get the fully qualified path to the `i`th node in `var`.
    pub fn ns_path_os<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<Option<OsString>> {
        let var = var.as_ref();
        let var = CString::new(var.as_bytes())?;

        let mut path: *mut c_char = ptr::null_mut();

        unsafe { aug_ns_path(self.ptr, var.as_ptr(), i as c_int, &mut path) };
        let p = ptr_to_os_string(path);
        unsafe { libc::free(path as *mut libc::c_void) };

        self.check_error()?;

        Ok(p)
    }

    /// Get the fully qualified path to the `i`th node in `var`.
    pub fn ns_path<T: AsRef<OsStr>>(&self, var: T, i: u32) -> Result<Option<String>> {
        match self.ns_path_os(var, i) {
            Ok(Some(p)) => Ok(Some(String::from_utf8(p.into_vec())?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Print each node matching `path` and its descendants.
    pub fn print_os<T: AsRef<OsStr>>(&self, path: T) -> Result<OsString> {
        let path = path.as_ref();
        let path = CString::new(path.as_bytes())?;
        let res;

        unsafe {
            let mut buf = ptr::null_mut();
            let mut size = 0;
            let file = new_memstream(&mut buf, &mut size)?;
            aug_print(self.ptr, file as *mut _, path.as_ptr());
            res = read_memstream(&mut buf, &mut size, file)?;
        };
        self.check_error()?;

        Ok(res)
    }

    /// Print each node matching `path` and its descendants.
    pub fn print<T: AsRef<OsStr>>(&self, path: T) -> Result<String> {
        match self.print_os(path) {
            Ok(p) => Ok(String::from_utf8(p.into_vec())?),
            Err(e) => Err(e),
        }
    }

    /// Run one or more newline-separated commands.
    ///
    /// The output of the commands
    /// will be returned as a `String`. Running just 'help' will print what commands are
    /// available. Commands accepted by this are identical to what `augtool`
    /// accepts.
    pub fn srun_os<T: AsRef<OsStr>>(&self, commands: T) -> Result<(CommandsNumber, OsString)> {
        let commands = commands.as_ref();
        let commands = CString::new(commands.as_bytes())?;

        let res;
        let num;

        unsafe {
            let mut buf = ptr::null_mut();
            let mut size = 0;
            let file = new_memstream(&mut buf, &mut size)?;
            num = aug_srun(self.ptr, file as *mut _, commands.as_ptr());
            res = read_memstream(&mut buf, &mut size, file)?;
        };
        self.check_error()?;

        let commands_num = if num == -1 {
            unreachable!("srun -1 output is handled by check_error");
        } else if num == -2 {
            CommandsNumber::Quit
        } else {
            CommandsNumber::Success(num as u32)
        };

        Ok((commands_num, res))
    }

    /// Run one or more newline-separated commands.
    ///
    /// The output of the commands
    /// will be returned as a `String`. Running just 'help' will print what commands are
    /// available. Commands accepted by this are identical to what `augtool`
    /// accepts.
    pub fn srun<T: AsRef<OsStr>>(&self, commands: T) -> Result<(CommandsNumber, String)> {
        match self.srun_os(commands) {
            Ok((num, p)) => Ok((num, String::from_utf8(p.into_vec())?)),
            Err(e) => Err(e),
        }
    }

    fn check_error(&self) -> std::result::Result<(), AugeasError> {
        self.error().map(Err).unwrap_or(Ok(()))
    }

    fn error(&self) -> Option<AugeasError> {
        let err = unsafe { aug_error(self.ptr) };
        let code = ErrorCode::from_raw(err as _)?;
        let message = unsafe { ptr_to_string(aug_error_message(self.ptr)) };
        let minor_message = unsafe { ptr_to_string(aug_error_minor_message(self.ptr)) };
        let details = unsafe { ptr_to_string(aug_error_details(self.ptr)) };

        Some(AugeasError {
            code,
            message,
            minor_message,
            details,
        })
    }

    /// Set the behavior of the save operation.
    pub fn set_save_mode(&mut self, mode: SaveMode) -> Result<()> {
        self.set("/augeas/save", mode.to_string())
    }

    fn check_tree_error<T: AsRef<OsStr>>(&self, path: T) -> Result<()> {
        let path = path.as_ref();
        match self.tree_error_os(path)? {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// Try to read error information about a given path.
    ///
    /// Use either:
    ///
    /// * `/augeas/FILENAME` if the error happened for a file, or
    /// * `/augeas/text/PATH` otherwise. `PATH` is the path to the toplevel node in
    ///   the tree where the lens application happened.
    pub fn tree_error_os<T: AsRef<OsStr>>(&self, err_path: T) -> Result<Option<OsTreeError>> {
        let err_path = err_path.as_ref();

        // For convenience, also accept paths containing "/error"
        let err_path = if err_path.to_string_lossy().ends_with("/error") {
            Cow::Borrowed(err_path)
        } else {
            Cow::Owned(build_path(err_path, "/error"))
        };
        let err = self.get(&err_path)?;
        if let Some(kind) = err {
            let message = self
                .get_os(build_path(&err_path, "/message"))?
                .unwrap_or_default();
            let path = self.get_os(build_path(&err_path, "/path"))?;
            let lens = self.get(build_path(&err_path, "/lens"))?;

            // These 3 are set together or not at all
            let pos: Option<usize> = self
                .get_os(build_path(&err_path, "/pos"))?
                .and_then(|s| s.to_string_lossy().parse().ok());
            let line: Option<usize> = self
                .get_os(build_path(&err_path, "/line"))?
                .and_then(|s| s.to_string_lossy().parse().ok());
            let char: Option<usize> = self
                .get_os(build_path(&err_path, "/char"))?
                .and_then(|s| s.to_string_lossy().parse().ok());
            let position = match (pos, line, char) {
                (Some(position), Some(line), Some(char)) => Some(ErrorPosition {
                    position,
                    line,
                    char,
                }),
                _ => None,
            };

            Ok(Some(OsTreeError {
                kind,
                message,
                path,
                position,
                lens,
            }))
        } else {
            Ok(None)
        }
    }

    /// Try to read error information about a given path.
    ///
    /// Use either:
    ///
    /// * `/augeas/FILENAME` if the error happened for a file, or
    /// * `/augeas/text/PATH` otherwise. `PATH` is the path to the toplevel node in
    ///   the tree where the lens application happened.
    pub fn tree_error<T: AsRef<OsStr>>(&self, err_path: T) -> Result<Option<TreeError>> {
        match self.tree_error_os(err_path) {
            Ok(Some(e)) => Ok(Some(e.into())),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for Augeas {
    fn drop(&mut self) {
        unsafe {
            aug_close(self.ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_test() {
        let p = Position::After;
        assert_eq!(p, "after".parse().unwrap());

        let p = Position::Before;
        assert_eq!(p, "BEFORE".parse().unwrap());
    }

    #[test]
    fn init_test() {
        let aug = Augeas::init(None::<&str>, "", Flags::NO_MODULE_AUTOLOAD);
        assert!(aug.is_ok());
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE);
        assert!(aug.is_ok());
    }

    #[test]
    fn version_test() {
        let aug = Augeas::init(Some("tests/test_root"), "toto", Flags::NONE).unwrap();
        assert!(aug.version().unwrap().starts_with("1."));
    }

    #[test]
    fn get_test() {
        use error::ErrorCode;
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let root_uid = aug
            .get("etc/passwd/root/uid")
            .unwrap()
            .unwrap_or("unknown".to_string());

        assert_eq!(&root_uid, "0", "ID of root was {}", root_uid);

        let nothing = aug.get("/foo");
        assert!(nothing.is_ok());
        assert!(nothing.ok().unwrap().is_none());

        let many = aug.get("etc/passwd/*");

        if let Err(Error::Augeas(err)) = many {
            assert_eq!(err.code, ErrorCode::TooManyMatches)
        } else {
            panic!("Unexpected value: {:?}", many)
        }
    }

    #[test]
    fn set_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        aug.set("etc/passwd/root/uid", "42").unwrap();
        let uid = aug.get("etc/passwd/root/uid").unwrap().unwrap();
        assert_eq!("42", uid);
    }

    #[test]
    fn clear_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        aug.clear("etc/passwd/root/uid").unwrap();
        let uid = aug.get("etc/passwd/root/uid").unwrap();
        assert_eq!(None, uid);
    }

    #[test]
    fn touch_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let exists = aug.matches("etc/passwd/epicaste/uid").unwrap();
        assert_eq!(exists.len(), 0);
        // Creates missing nodes.
        aug.touch("etc/passwd/epicaste/uid").unwrap();
        let exists = aug.matches("etc/passwd/epicaste/uid").unwrap();
        assert_eq!(exists.len(), 1);
        // Does not overwrite existing nodes.
        aug.touch("etc/passwd/root/uid").unwrap();
        let uid = aug.get("etc/passwd/root/uid").unwrap().unwrap();
        assert_eq!("0", uid);
    }

    #[test]
    fn get_nonutf8_test() {
        let invalid: &[u8; 2] = &[0xc3, 0x28];
        let invalid = OsStr::from_bytes(invalid);
        dbg!(invalid);

        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.set("etc/passwd/root/home", invalid).unwrap();

        let home = aug.get_os("etc/passwd/root/home").unwrap().unwrap();
        assert_eq!(invalid, home);
        assert!(dbg!(aug.get("etc/passwd/root/home")).is_err());
        assert!(aug.get("etc/passwd/root/uid").is_ok());
    }

    #[test]
    fn label_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let root_name = aug.label("etc/passwd/root").unwrap().unwrap();

        assert_eq!(&root_name, "root", "name of root was {}", root_name);
    }

    #[test]
    fn matches_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let users = aug.matches("etc/passwd/*").unwrap();
        let count = aug.count("etc/passwd/*").unwrap();

        assert_eq!(9, users.len());
        assert_eq!(9, count);
        assert_eq!("/files/etc/passwd/root", users[0]);
        assert_eq!("/files/etc/passwd/nobody", users[8]);
    }

    #[test]
    fn count_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let users = aug.count("etc/passwd/*").unwrap();
        assert_eq!(9, users);
    }

    #[test]
    fn insert_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.insert("etc/passwd/root", "before", Position::Before)
            .unwrap();
        aug.insert("etc/passwd/root", "after", Position::After)
            .unwrap();
        let users = aug.matches("etc/passwd/*").unwrap();
        assert_eq!(
            [
                "/files/etc/passwd/before",
                "/files/etc/passwd/root",
                "/files/etc/passwd/after"
            ],
            users[0..3]
        );
    }

    #[test]
    fn rm_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let e = aug.rm("/augeas[");
        assert!(e.is_err());

        let r = aug.rm("etc/passwd").unwrap();
        assert_eq!(64, r);
    }

    #[test]
    fn mv_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let e = aug.mv("etc/passwd", "etc/passwd/root");
        assert!(e.is_err());

        aug.mv("etc/passwd", "etc/other").unwrap();
        assert_eq!(0, aug.count("etc/passwd").unwrap());
        assert_eq!(1, aug.count("etc/other").unwrap());
    }

    #[test]
    fn defvar_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.defvar("x", "etc/passwd/*").unwrap();
        let n = aug.count("$x").unwrap();

        assert_eq!(9, n);
    }

    #[test]
    fn defnode_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let created = aug.defnode("y", "etc/notthere", "there").unwrap();
        assert!(created);

        let there = aug.get("$y").unwrap();
        assert_eq!("there", there.expect("failed to get etc/notthere"));

        let created = aug.defnode("z", "etc/passwd", "there").unwrap();
        assert!(!created);
    }

    #[test]
    fn load_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.set("etc/passwd/root/uid", "42").unwrap();
        aug.load().unwrap();
        let uid = aug.get("etc/passwd/root/uid").unwrap();
        assert_eq!("0", uid.expect("expected value for root/uid"));
    }

    #[test]
    fn setm_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let count = aug.setm("etc/passwd", "*/shell", "/bin/zsh").unwrap();
        assert_eq!(9, count);
    }

    #[test]
    fn clearm_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let count = aug.setm("etc/passwd", "*/shell", "/bin/zsh").unwrap();
        assert_eq!(9, count);
    }

    #[test]
    fn span_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::ENABLE_SPAN).unwrap();

        // happy path
        let span = aug.span("etc/passwd/root").unwrap().unwrap();
        assert_eq!(0..4, span.label);
        assert_eq!(0..0, span.value);
        assert_eq!(0..32, span.span);
        assert_eq!("tests/test_root/etc/passwd", span.filename.unwrap());

        // no span info associated with node
        let span = aug.span("/augeas/load").unwrap();
        assert!(span.is_none());

        // too many matches
        let span = aug.span("etc/passwd/*");
        assert!(span.is_err());
    }

    #[test]
    fn store_retrieve_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.set("/text/in", "alex:x:12:12:Alex:/home/alex:/bin/sh\n")
            .unwrap();
        aug.text_store("Passwd.lns", "/text/in", "/stored").unwrap();
        aug.set("/stored/alex/uid", "17").unwrap();

        aug.text_retrieve("Passwd.lns", "/text/in", "/stored", "/text/out")
            .unwrap();
        let text = aug.get("/text/out").unwrap().unwrap();
        assert_eq!("alex:x:17:12:Alex:/home/alex:/bin/sh\n", text);

        // Invalidate the tree; 'shell' must be present
        aug.rm("/stored/alex/shell").unwrap();
        let err = aug
            .text_retrieve("Passwd.lns", "/text/in", "/stored", "/text/out")
            .err()
            .unwrap();

        if let Error::Tree(e) = err {
            let e: TreeError = e.into();
            assert_eq!("/stored/stored/alex", e.path.unwrap().as_str());
            assert_eq!(None, e.position);
            assert_eq!("put_failed", e.kind);
            assert!(!e.lens.unwrap().is_empty());
            assert!(e.message.starts_with("Failed to match tree under"));
        } else {
            panic!("Unexpected error: {:?}", err);
        }

        aug.set("/text/in", "alex:invalid passwd entry").unwrap();
        let err = aug
            .text_store("Passwd.lns", "/text/in", "/stored")
            .err()
            .unwrap();
        if let Error::Tree(e) = err {
            let e: TreeError = e.into();
            assert_eq!(e.message, "Iterated lens matched less than it should");
            assert_eq!(
                ErrorPosition {
                    position: 5,
                    line: 1,
                    char: 5,
                },
                e.position.unwrap()
            );
            assert_eq!("parse_failed", e.kind);
            assert_eq!("/stored/stored/alex", e.path.unwrap());
            assert!(!e.lens.unwrap().is_empty());
        } else {
            panic!("Unexpected error: {:?}", err);
        }
    }

    #[test]
    fn rename_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.rename("etc/passwd/root", "ruth").unwrap();

        let ruth = aug.get("etc/passwd/ruth/uid").unwrap().unwrap();
        assert_eq!("0", ruth);

        let root = aug.get("etc/passwd/root/uid").unwrap();
        assert!(root.is_none());
    }

    #[test]
    fn transform_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.transform("Hosts.lns", "/usr/local/etc/hosts", false)
            .unwrap();
        let p = aug
            .get("/augeas/load/Hosts/incl[. = '/usr/local/etc/hosts']")
            .unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn cp_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.cp("etc/passwd/root", "etc/passwd/ruth").unwrap();
        let ruth = aug.get("etc/passwd/ruth/uid").unwrap().unwrap();
        assert_eq!("0", ruth);

        let root = aug.get("etc/passwd/root/uid").unwrap().unwrap();
        assert_eq!("0", root);
    }

    #[test]
    fn escape_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        // no escaping needed
        let n = aug.escape_name("foo").unwrap();
        assert_eq!("foo", n);

        let n = aug.escape_name("foo[").unwrap();
        assert_eq!("foo\\[", n);
    }

    #[test]
    fn load_file_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NO_LOAD).unwrap();

        aug.load_file("/etc/passwd").unwrap();
        let root = aug.get("etc/passwd/root/uid").unwrap();
        assert!(root.is_some());

        let err = aug.load_file("/var/no/lens/for/this");
        assert!(err.is_err());
        let e = err.err().unwrap();
        assert!(e.is_code(ErrorCode::NoLens));
    }

    #[test]
    fn source_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let s = aug.source("etc/passwd/root/uid").unwrap();
        assert_eq!(s.unwrap(), "/files/etc/passwd")
    }

    #[test]
    fn preview_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        let s = aug.preview("etc/non-existing").unwrap_err();
        assert!(matches!(s, Error::Augeas(err) if err.code == ErrorCode::NoMatch));

        let s = aug.preview("etc/passwd/root/uid").unwrap();
        let content = include_str!("../tests/test_root/etc/passwd");
        assert_eq!(s.unwrap(), content);
    }

    #[test]
    fn ns_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();

        aug.defvar("x", "etc/passwd/*").unwrap();
        aug.defvar("uid", "etc/passwd/*/uid").unwrap();

        let attr = aug.ns_attr("x", 0).unwrap();
        assert_eq!("root", attr.label.unwrap());
        assert!(attr.value.is_none());
        assert_eq!("/files/etc/passwd", attr.file_path.unwrap());

        let attr = aug.ns_attr("x", 10000).unwrap_err();
        assert!(attr.is_code(ErrorCode::NoMatch));

        let attr = aug.ns_attr("y", 0).unwrap_err();
        assert!(attr.is_code(ErrorCode::NoMatch));

        let label = aug.ns_label("x", 0).unwrap();
        assert_eq!("root", label);

        let index = aug.ns_index("x", 4).unwrap();
        assert_eq!(0, index);

        let uid = aug.ns_value("uid", 2).unwrap().unwrap();
        assert_eq!("2", &uid);

        let count = aug.ns_count("uid").unwrap();
        assert_eq!(9, count);

        let path = aug.ns_path("uid", 0).unwrap().unwrap();
        assert_eq!("/files/etc/passwd/root/uid", path);
    }

    #[test]
    fn print_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let content = aug.print("/augeas/version").unwrap();
        assert!(content.starts_with("/augeas/version = \"1."));
    }

    #[test]
    fn srun_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let (num, res) = aug.srun("help").unwrap();
        assert!(matches!(num, CommandsNumber::Success(_)));
        assert!(res.contains("commands:"));
    }

    #[test]
    fn srun_with_invalid_command() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let result = aug.srun("invalid_command");
        assert!(result.is_err());
    }

    #[test]
    fn srun_with_quit_command() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let (num, _out) = aug.srun("quit").unwrap();
        assert_eq!(num, CommandsNumber::Quit);
    }

    #[test]
    fn save_mode_test() {
        let mut aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        aug.set_save_mode(SaveMode::Backup).unwrap();
        let mode = aug.get("/augeas/save").unwrap().unwrap();
        assert_eq!("backup", mode);
        aug.set_save_mode(SaveMode::Noop).unwrap();
        let mode = aug.get("/augeas/save").unwrap().unwrap();
        assert_eq!("noop", mode);
        aug.set_save_mode(SaveMode::NewFile).unwrap();
        let mode = aug.get("/augeas/save").unwrap().unwrap();
        assert_eq!("newfile", mode);
        aug.set_save_mode(SaveMode::Overwrite).unwrap();
        let mode = aug.get("/augeas/save").unwrap().unwrap();
        assert_eq!("overwrite", mode);
    }

    #[test]
    fn error_test() {
        let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE).unwrap();
        let garbled = aug.matches("/invalid[");

        if let Err(Error::Augeas(err)) = garbled {
            assert_eq!(err.code, ErrorCode::PathExpr);
            assert_eq!(err.message.unwrap(), "Invalid path expression");
            assert_eq!(err.minor_message.unwrap(), "illegal string literal");
            assert_eq!(err.details.unwrap(), "/invalid[|=|")
        } else {
            panic!("Unexpected value: {:?}", garbled)
        }
    }
}
