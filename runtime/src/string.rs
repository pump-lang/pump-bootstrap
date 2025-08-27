// chuoi, va viec in ra stdout.
//
// chuoi khong sua duoc va luon la UTF-8 hop le. Hinh dang: header, roi do dai
// tinh bang byte o +16, roi cai hash FNV-1a 64 bit tinh luoi o +24, roi may
// byte noi dung nam luon tu +32 kem mot NUL o cuoi. Hash nao tinh ra bang 0
// thi ghi thanh 1, nen doc duoc so 0 luon co nghia la "chua tinh".

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
pub const BYTES_OFFSET: usize = HEADER_SIZE + 16;

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
        let object = pump_alloc(
            TYPE_ID_STRING,
            align_object_size(unaligned_size(left + right)),
        );
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

/// The substring between two byte offsets, with `end` exclusive.
#[no_mangle]
pub extern "C" fn pump_string_slice(s: *const u8, start: i64, end: i64) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep(s as *mut u8);
    unsafe {
        let length = byte_length(s) as i64;
        if start < 0 || start > length {
            pump_panic_index(start, length);
        }
        if end < start || end > length {
            pump_panic_index(end, length);
        }
        let bytes = bytes_of(s);
        if !is_boundary(bytes, start as usize) || !is_boundary(bytes, end as usize) {
            panic_not_a_boundary();
        }
        new_string(&bytes[start as usize..end as usize])
    }
}

fn is_boundary(bytes: &[u8], offset: usize) -> bool {
    offset == bytes.len() || bytes[offset] & 0xc0 != 0x80
}

fn panic_not_a_boundary() -> ! {
    const MESSAGE: &[u8] = b"a string was sliced in the middle of a character";
    pump_panic_cstr(MESSAGE.as_ptr(), MESSAGE.len() as u64)
}

/// The string's scalar values as a `[char]`.
#[no_mangle]
pub extern "C" fn pump_string_chars(s: *const u8) -> *mut u8 {
    let scalars: Vec<u32> = unsafe { text_of(s).chars().map(u32::from).collect() };

    let scope = RootScope::new();
    let array = scope.keep(pump_array_new(
        crate::TYPE_ID_ARRAY_SCALAR,
        scalars.len() as u64,
    ));
    for scalar in scalars {
        pump_array_push(array, u64::from(scalar));
    }
    array
}

// rendering

/// A one-character string.
#[no_mangle]
pub extern "C" fn pump_string_from_char(c: u32) -> *mut u8 {
    let scalar = char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER);
    let mut encoded = [0u8; 4];
    new_string(scalar.encode_utf8(&mut encoded).as_bytes())
}

/// `"true"` or `"false"`.
#[no_mangle]
pub extern "C" fn pump_string_from_bool(b: i8) -> *mut u8 {
    new_string(if b != 0 { b"true" } else { b"false" })
}

/// Decimal, with a leading `-` when negative.
#[no_mangle]
pub extern "C" fn pump_string_from_int(v: i64) -> *mut u8 {
    new_string(v.to_string().as_bytes())
}

/// Decimal.
#[no_mangle]
pub extern "C" fn pump_string_from_uint(v: u64) -> *mut u8 {
    new_string(v.to_string().as_bytes())
}

/// The shortest representation that round-trips, always carrying a decimal
/// point or an exponent so that a rendered `float` never reads as an `int`.
#[no_mangle]
pub extern "C" fn pump_string_from_float(v: f64) -> *mut u8 {
    if v.is_nan() {
        return new_string(b"nan");
    }
    if v.is_infinite() {
        return new_string(if v < 0.0 { b"-inf" } else { b"inf" });
    }
    let mut text = v.to_string();
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    new_string(text.as_bytes())
}

/// `char(x)` from a `uint`.
#[no_mangle]
pub extern "C" fn pump_char_from_uint(v: u64) -> u32 {
    match u32::try_from(v).ok().and_then(char::from_u32) {
        Some(scalar) => u32::from(scalar),
        None => {
            const MESSAGE: &[u8] = b"char() was given a value that is not a Unicode scalar";
            pump_panic_cstr(MESSAGE.as_ptr(), MESSAGE.len() as u64)
        }
    }
}

// output

/// Writes to standard output with no trailing newline.
#[no_mangle]
pub extern "C" fn pump_print(s: *const u8) {
    let bytes = unsafe { bytes_of(s) };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
}

/// Writes to standard output followed by a newline.
#[no_mangle]
pub extern "C" fn pump_println(s: *const u8) {
    let bytes = unsafe { bytes_of(s) };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.write_all(b"\n");
}

/// Writes to standard error followed by a newline.
#[no_mangle]
pub extern "C" fn pump_print_error(s: *const u8) {
    let bytes = unsafe { bytes_of(s) };
    let _ = std::io::stdout().flush();
    let mut error = std::io::stderr().lock();
    let _ = error.write_all(bytes);
    let _ = error.write_all(b"\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn string(text: &str) -> *mut u8 {
        pump_string_new(text.as_ptr(), text.len() as u64)
    }

    fn read(s: *const u8) -> String {
        unsafe { text_of(s).to_owned() }
    }

    #[test]
    fn the_layout_constants_match_the_document() {
        assert_eq!(LENGTH_OFFSET, 16);
        assert_eq!(HASH_OFFSET, 24);
        assert_eq!(BYTES_OFFSET, 32);
        assert_eq!(align_object_size(unaligned_size(0)), 48);
        assert_eq!(align_object_size(unaligned_size(15)), 48);
        assert_eq!(align_object_size(unaligned_size(16)), 64);
    }

    #[test]
    fn a_string_carries_its_bytes_its_length_and_a_trailing_nul() {
        let _guard = testing::guard();
        let s = string("héllo");

        assert_eq!(pump_string_len(s), 6);
        assert_eq!(pump_string_char_count(s), 5);
        assert_eq!(read(s), "héllo");
        unsafe { assert_eq!(*s.add(BYTES_OFFSET + 6), 0) };
    }

    #[test]
    fn concatenation_joins_the_bytes() {
        let _guard = testing::guard();
        let joined = pump_string_concat(string("pump"), string(" 0.1"));
        assert_eq!(read(joined), "pump 0.1");
        assert_eq!(read(pump_string_concat(string(""), string(""))), "");
    }

    #[test]
    fn equality_and_ordering_are_byte_wise() {
        let _guard = testing::guard();
        assert_eq!(pump_string_eq(string("abc"), string("abc")), 1);
        assert_eq!(pump_string_eq(string("abc"), string("abd")), 0);
        assert_eq!(pump_string_eq(string("ab"), string("abc")), 0);

        assert_eq!(pump_string_cmp(string("abc"), string("abd")), -1);
        assert_eq!(pump_string_cmp(string("abd"), string("abc")), 1);
        assert_eq!(pump_string_cmp(string("abc"), string("abc")), 0);
        assert_eq!(pump_string_cmp(string("ab"), string("abc")), -1);
    }

    #[test]
    fn the_hash_is_cached_and_never_zero() {
        let _guard = testing::guard();
        let s = string("the quick brown fox");
        unsafe { assert_eq!(read_u64(s, HASH_OFFSET), 0) };

        let first = pump_string_hash(s);
        assert_ne!(first, 0);
        assert_eq!(pump_string_hash(s), first);
        assert_eq!(first, fnv1a(b"the quick brown fox"));

        // Equal contents in distinct objects hash the same, which is what a
        // string-keyed map depends on.
        assert_eq!(pump_string_hash(string("the quick brown fox")), first);
    }

    #[test]
    fn slicing_works_on_character_boundaries() {
        let _guard = testing::guard();
        let s = string("héllo");
        assert_eq!(read(pump_string_slice(s, 0, 3)), "hé");
        assert_eq!(read(pump_string_slice(s, 3, 6)), "llo");
        assert_eq!(read(pump_string_slice(s, 6, 6)), "");
        assert_eq!(read(pump_string_slice(s, 0, 6)), "héllo");

        // Byte 2 is the second half of the é, so it is not a legal boundary.
        let bytes = unsafe { bytes_of(s) };
        assert!(!is_boundary(bytes, 2));
        assert!(is_boundary(bytes, 3));
        assert!(is_boundary(bytes, 6));
    }

    #[test]
    fn bytes_and_chars_are_different_views() {
        let _guard = testing::guard();
        let s = string("hé");
        assert_eq!(pump_string_byte_at(s, 0), 0x68);
        assert_eq!(pump_string_byte_at(s, 1), 0xc3);

        let chars = pump_string_chars(s);
        assert_eq!(crate::array::pump_array_len(chars), 2);
        assert_eq!(crate::array::pump_array_get(chars, 0), 0x68);
        assert_eq!(crate::array::pump_array_get(chars, 1), 0xe9);
    }

    #[test]
    fn every_primitive_renders() {
        let _guard = testing::guard();
        assert_eq!(read(pump_string_from_bool(1)), "true");
        assert_eq!(read(pump_string_from_bool(0)), "false");
        assert_eq!(read(pump_string_from_int(-42)), "-42");
        assert_eq!(
            read(pump_string_from_uint(u64::MAX)),
            "18446744073709551615"
        );
        assert_eq!(read(pump_string_from_char(0x1f600)), "\u{1f600}");

        assert_eq!(read(pump_string_from_float(1.0)), "1.0");
        assert_eq!(read(pump_string_from_float(-0.5)), "-0.5");
        assert_eq!(read(pump_string_from_float(f64::INFINITY)), "inf");
        assert_eq!(read(pump_string_from_float(f64::NEG_INFINITY)), "-inf");
        assert_eq!(read(pump_string_from_float(f64::NAN)), "nan");
    }

    #[test]
    fn a_valid_scalar_converts_and_keeps_its_value() {
        let _guard = testing::guard();
        assert_eq!(pump_char_from_uint(0x41), 0x41);
        assert_eq!(pump_char_from_uint(0x10_ffff), 0x10_ffff);
    }
}
