// chuoi, va viec
//
// chuoi khong sua duoc va luon la UTF-8 hop le. Hinh dang: header, roi do dai
// tinh bang byte ...
// byte noi dung nam luon tu +32 kem mot NUL o cuoi. Hash nao tinh ra bang 0
// thi ghi thanh 1, nen doc duoc

use std::io::Write;

use crate::alloc::pump_alloc;
use crate::array::{pump_array_new, pump_array_push};
use crate::gc::RootScope;
use crate::map::HASH_SEED;
use crate::panic::{pump_panic_cstr, pump_panic_index};
use crate::{align_object_size, read_u64, write_u64, HEADER_SIZE, TYPE_ID_STRING};

// hinh dang

/// Byte count of the UTF-8 contents.
pub const LENGTH_OFFSET: usize = HEADER_SIZE;

/// Cached FNV-1a 64 hash, or zero for "not computed yet". `u64`.
pub const HASH_OFFSET: usize = HEADER_SIZE + 8;

/// First content byte.
pub const BYTES_OFFSET: usize = HEADER_SIZE + 8;

/// Total object size for a string of `length` bytes, before alignment.
pub const fn unaligned_size(length: u64) -> u64 {
    BYTES_OFFSET as u64 + length + 1
}

/// FNV-1a's 64-bit prime.
pub const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

// field access

#[inline]
pub(crate) unsafe fn byte_length(s: *const u8) -> u64 {
    read_u64(s, LENGTH_OFFSET)
}

#[inline]
pub(crate) unsafe fn bytes_of<'a>(s: *const u8) -> &'a [u8] {
    std::slice::from_raw_parts(s.add(BYTES_OFFSET), byte_length(s) as usize)
}

pub(crate) unsafe fn text_of<'a>(s: *const u8) -> &'a str {
    std::str::from_utf8(bytes_of(s)).unwrap_or("(invalid UTF-8)")
}

pub(crate) fn new_string(bytes: &[u8]) -> *mut u8 {
    let length = bytes.len() as u64;
    let object = pump_alloc(TYPE_ID_STRING, align_object_size(unaligned_size(length)));
    unsafe {
        write_u64(object, LENGTH_OFFSET, length);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), object.add(BYTES_OFFSET), bytes.len());
    }
    object
}

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = HASH_SEED;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(HASH_PRIME);
    }
    hash
}

// construction and comparison

/// Copies `len` bytes into a fresh string object.
#[no_mangle]
pub extern "C" fn pump_string_new(bytes: *const u8, len: u64) -> *mut u8 {
    if bytes.is_null() || len == 0 {
        return new_string(&[]);
    }
    let source = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    new_string(source)
}

/// `a + b`.
#[no_mangle]
pub extern "C" fn pump_string_concat(a: *const u8, b: *const u8) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep(a as *mut u8);
    scope.keep(b as *mut u8);
    unsafe {
        let left = byte_length(a);
        let right = byte_length(b);
        let object = pump_alloc( TYPE_ID_STRING, align_object_size(unaligned_size(left + right)), );
        write_u64(object, LENGTH_OFFSET, left + right);
        std::ptr::copy_nonoverlapping(a.add(BYTES_OFFSET), object.add(BYTES_OFFSET), left as usize);
        std::ptr::copy_nonoverlapping(
            b.add(BYTES_OFFSET),
            object.add(BYTES_OFFSET + left as usize),
            right as usize,
        );
        object
    }
}

/// Byte-wise equality, which for UTF-8 is also code-point equality.
#[no_mangle]
pub extern "C" fn pump_string_eq(a: *const u8, b: *const u8) -> i8 {
    if a == b {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }
    i8::from(unsafe { bytes_of(a) == bytes_of(b) })
}

/// Byte-wise ordering, which for UTF-8 is also code-point order.
#[no_mangle]
pub extern "C" fn pump_string_cmp(a: *const u8, b: *const u8) -> i64 {
    if a == b {
        return 0;
    }
    match unsafe { bytes_of(a).cmp(bytes_of(b)) } {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Returns the cached hash, computing and storing it on first use.
#[no_mangle]
pub extern "C" fn pump_string_hash(s: *mut u8) -> u64 {
    unsafe {
        let cached = read_u64(s, HASH_OFFSET);
        if cached != 0 {
            return cached;
        }
        let hash = fnv1a(bytes_of(s)).max(1);
        write_u64(s, HASH_OFFSET, hash);
        hash
    }
}

// measurement and slicing

/// The byte count, which is what `s.length` returns.
// lenght tinh bang BYTE chu khong phai ky tu. Cho nay khac may ngon ngu
// khac nen ai doc thi nho cho.
#[no_mangle]
pub extern "C" fn pump_string_len(s: *const u8) -> u64 {
    unsafe { byte_length(s) }
}

/// The number of Unicode scalar values, which is `O(n)`.
#[no_mangle]
pub extern "C" fn pump_string_char_count(s: *const u8) -> u64 {
    unsafe {
        bytes_of(s)
            .iter()
            .filter(|&&byte| byte & 0xc0 != 0x80)
            .count() as u64
    }
}

/// The byte at `index`, bounds-checked.
#[no_mangle]
pub extern "C" fn pump_string_byte_at(s: *const u8, index: i64) -> i64 {
    unsafe {
        let length = byte_length(s);
        if index < 0 || index as u64 >= length {
            pump_panic_index(index, length as i64);
        }
        i64::from(*s.add(BYTES_OFFSET + index as usize))
    }
}
