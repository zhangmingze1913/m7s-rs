//! Minimal dynamic plugin ABI v1.
//!
//! Exports:
//! - `m7s_plugin_abi() -> u32`
//! - `m7s_plugin_name() -> *const c_char`
//! - `m7s_transform(input, len, out, cap) -> isize`

use std::os::raw::{c_char, c_uchar};

#[no_mangle]
pub extern "C" fn m7s_plugin_abi() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn m7s_plugin_name() -> *const c_char {
    b"hello\0".as_ptr().cast()
}

/// Uppercases ASCII into `out`, returns bytes written or -1.
#[no_mangle]
pub unsafe extern "C" fn m7s_transform(
    input: *const c_uchar,
    len: usize,
    out: *mut c_uchar,
    cap: usize,
) -> isize {
    if input.is_null() || out.is_null() || len > cap {
        return -1;
    }
    let src = std::slice::from_raw_parts(input, len);
    let dst = std::slice::from_raw_parts_mut(out, cap);
    for (i, b) in src.iter().enumerate() {
        dst[i] = b.to_ascii_uppercase();
    }
    len as isize
}
