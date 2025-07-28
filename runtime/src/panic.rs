// panic, thoat, va cai o loi dang treo.
//
// ham nao o day bao panic thi khong bao gio quay ve. No viet mot dong ra
// stderr roi giet tien trinh. Code sinh ra dat mot terminator unreachable
// ngay sau moi loi goi, nen quay ve la quay ve mot cho khong ton tai.
//
// Moi kieu panic ...
// de code sinh ra khong phai dung mot cai string tren mot duong ma dang chuan
// bi chet den noi.
//
// Ham nao co kieu tra ve Pump la `T!` thi van chi tra ve dung bieu dien cua
// T; trong ABI nay khong cho nao tra ve nhieu gia tri ca. Loi di duong khac,
// qua bien toan cuc pump_error_slot. `fail e` cam loi vao o do roi tra ve gia
// tri 0, cho goi nao co the that bai thi kiem o do ngay sau, `!` thi return
// va de nguyen o do, con catch thi lay ra va xoa di. O do luon la mot goc GC.

use std::io::Write;
use std::process;
use std::ptr;

/// The exit code a panic produces.
pub const PANIC_EXIT_CODE: i32 = 101;

/// The pending-error slot: null when no error is in flight, otherwise the
/// `Error` interface value the most recent `fail` produced.
#[no_mangle]
pub static mut pump_error_slot: *mut u8 = ptr::null_mut();

fn abort_with(message: std::fmt::Arguments<'_>) -> ! {
    let _ = std::io::stdout().flush();
    let mut error = std::io::stderr();
    let _ = writeln!(error, "pump: panic: {message}");
    let _ = error.flush();
    process::exit(PANIC_EXIT_CODE)
}

pub(crate) fn abort_runtime(message: &str) -> ! {
    abort_with(format_args!("{message}"))
}

/// Aborts with a Pump `string` as the message.
#[no_mangle]
pub extern "C" fn pump_panic(message: *const u8) -> ! {
    if message.is_null() {
        abort_with(format_args!("(null message)"));
    }
    let text = unsafe { crate::string::text_of(message) };
    abort_with(format_args!("{text}"))
}

/// Aborts with a raw UTF-8 message, for panics raised before a `string`
/// object could be built.
#[no_mangle]
pub extern "C" fn pump_panic_cstr(bytes: *const u8, len: u64) -> ! {
    if bytes.is_null() {
        abort_with(format_args!("(null message)"));
    }
    let text = unsafe {
        let slice = std::slice::from_raw_parts(bytes, len as usize);
        String::from_utf8_lossy(slice).into_owned()
    };
    abort_with(format_args!("{text}"))
}

/// Aborts on an out-of-range index.
#[no_mangle]
pub extern "C" fn pump_panic_index(index: i64, length: i64) -> ! {
    abort_with(format_args!(
        "index {index} is out of range for a length of {length}"
    ))
}

/// Aborts on integer division or remainder by zero (grammar D-7).
#[no_mangle]
pub extern "C" fn pump_panic_divide_by_zero() -> ! {
    abort_with(format_args!("division by zero"))
}

/// Aborts when `null` reached a position that required a value.
#[no_mangle]
pub extern "C" fn pump_panic_null() -> ! {
    abort_with(format_args!("null where a value was required"))
}

/// Aborts on a negative shift count.
#[no_mangle]
pub extern "C" fn pump_panic_negative_shift(count: i64) -> ! {
    abort_with(format_args!("shift count {count} is negative"))
}

/// Aborts on a map lookup whose key is absent.
#[no_mangle]
pub extern "C" fn pump_panic_missing_key() -> ! {
    abort_with(format_args!("no such key in the map"))
}

/// Aborts when a collection was mutated while being iterated (grammar
/// 13.3.8).
#[no_mangle]
pub extern "C" fn pump_panic_concurrent_modification() -> ! {
    abort_with(format_args!(
        "a collection was modified while it was being iterated"
    ))
}

/// Terminates the process with `code`, running runtime shutdown first.
#[no_mangle]
pub extern "C" fn pump_exit(code: i32) -> ! {
    crate::start::pump_rt_shutdown(code);
    process::exit(code)
}

/// Stores an `Error` interface value into `pump_error_slot`.
#[no_mangle]
pub extern "C" fn pump_error_set(error: *mut u8) {
    unsafe { pump_error_slot = error };
}

/// Returns `pump_error_slot` and clears it.
#[no_mangle]
pub extern "C" fn pump_error_take() -> *mut u8 {
    unsafe {
        let error = pump_error_slot;
        pump_error_slot = ptr::null_mut();
        error
    }
}

/// 1 when `pump_error_slot` is non-null.
#[no_mangle]
pub extern "C" fn pump_error_pending() -> i8 {
    i8::from(unsafe { !pump_error_slot.is_null() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn the_error_slot_round_trips() {
        let _guard = testing::guard();
        assert_eq!(pump_error_pending(), 0);

        let error = 0x1234_5678_usize as *mut u8;
        pump_error_set(error);
        assert_eq!(pump_error_pending(), 1);
        assert_eq!(pump_error_take(), error);

        assert_eq!(pump_error_pending(), 0);
        assert!(pump_error_take().is_null());
    }
}
