use libc::c_char;
use std::ffi::{CStr, OsStr, OsString};
use std::mem::transmute;
use std::os::unix::prelude::OsStrExt;
use std::{io, slice};

use crate::Result;

pub fn build_path<T: AsRef<OsStr>, S: AsRef<OsStr>>(prefix: T, suffix: S) -> OsString {
    let mut path = prefix.as_ref().to_os_string();
    path.push(suffix.as_ref());
    path
}

pub fn ptr_to_string(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        let s = unsafe { CStr::from_ptr(s) };
        let s = String::from_utf8_lossy(s.to_bytes()).into_owned();
        Some(s)
    }
}

pub fn ptr_to_os_string(s: *const c_char) -> Option<OsString> {
    if s.is_null() {
        None
    } else {
        let s = unsafe { CStr::from_ptr(s) };
        let s = OsStr::from_bytes(s.to_bytes());
        let s = s.to_os_string();
        Some(s)
    }
}

/// Helper for functions writing in a FILE, returning the output as a String.
pub(crate) unsafe fn new_memstream(
    buf: *mut *mut c_char,
    size: *mut usize,
) -> Result<*mut libc::FILE> {
    unsafe {
        let out = libc::open_memstream(buf, size);
        if out.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Ok(out)
    }
}

pub(crate) unsafe fn read_memstream(
    buf: *mut *mut c_char,
    size: *mut usize,
    out: *mut libc::FILE,
) -> Result<OsString> {
    unsafe {
        assert!(
            !buf.is_null() && !size.is_null() && !out.is_null(),
            "Invalid read_memstream arguments"
        );
        if libc::fclose(out) != 0 {
            return Err(io::Error::last_os_error().into());
        }

        // Make buffer from content + size
        let b_slice: &[u8] = transmute(slice::from_raw_parts(*buf, *size + 1));
        let res_out =
            CStr::from_bytes_with_nul(b_slice).expect("Invalid buffer content from open_memstream");
        let res_out = OsStr::from_bytes(res_out.to_bytes());
        let res = res_out.to_os_string();

        // We are responsible for freeing the buffer we got from `open_memstream`.
        libc::free(*buf as *mut libc::c_void);

        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn test_ptr_to_string() {
        let s = c"Hello, World!".as_ptr();
        assert_eq!(
            ptr_to_string(s as *const _),
            Some("Hello, World!".to_string())
        );

        let s = ptr::null();
        assert_eq!(ptr_to_string(s), None);
    }

    #[test]
    fn test_new_memstream() {
        unsafe {
            let mut buf = ptr::null_mut();
            let mut size = 0;
            let file = new_memstream(&mut buf, &mut size).unwrap();

            assert!(!file.is_null());
        }
    }

    #[test]
    fn test_read_memstream() {
        unsafe {
            let mut buf = ptr::null_mut();
            let mut size = 0;
            let file = new_memstream(&mut buf, &mut size).unwrap();

            let hello = "Hello, World!";
            let s = CString::new(hello).unwrap();
            libc::fprintf(file, s.as_ptr());

            let res = read_memstream(&mut buf, &mut size, file).unwrap();
            assert_eq!(res, hello);
        }
    }

    #[test]
    fn test_read_memstream_empty() {
        unsafe {
            let mut buf = ptr::null_mut();
            let mut size = 0;
            let file = new_memstream(&mut buf, &mut size).unwrap();
            let res = read_memstream(&mut buf, &mut size, file).unwrap();
            assert_eq!(res, OsString::from(""));
        }
    }
}
