// file tren dia, dong lenh, va chay mot tien trinh khac.
//
// Day la cho duy nhat trong runtime cham vao the gioi ben ngoai ngoai viec in
// ra stdout. Truoc ban nay khong co gi o day ca, va do la ly do khong the
// viet compiler cua Pump bang chinh Pump: mot compiler khong doc noi dau vao
// cua no thi khong ton tai duoc.
//
// -- LOI DI DUONG NAO --
//
// Ham nao o day cung KHONG panic khi that bai. Panic la giet tien trinh, ma
// mot file thieu phai la loi bat duoc bang `catch` chu khong phai cai chet.
// Nhung runtime cung khong tu cam duoc loi vao pump_error_slot: o do phai la
// mot gia tri interface `Error`, ma itable cua no do compiler sinh ra chu
// khong phai crate nay.
//
// Nen quy uoc la kieu errno: moi cua vao o duoi XOA cai o loi luc bat dau,
// va ghi vao do khi hong. Doc thi tra ve null, ghi thi tra ve 0. Ben Pump,
// `std/io.pump` voi `std/os.pump` doc cai o do ra roi `fail` mot cach dang
// hoang. Ai goi thang builtin ma khong kiem thi tu chiu.
//
// -- BYTE DI THANH [int] --
//
// Pump chua co u8, `int` la 64 bit, nen mot byte ton nguyen mot o 8 byte:
// mot megabyte tren dia thanh tam megabyte trong heap. Ton that, biet roi,
// nhung dung truoc da; so nguyen co kich thuoc de ban sau.

use std::io::Write;
use std::process::Command;
use std::ptr;

use crate::array::{pump_array_get, pump_array_len, pump_array_new, pump_array_push};
use crate::gc::RootScope;
use crate::iface::pump_box_new;
use crate::string::{new_string, text_of};
use crate::{Global, TYPE_ID_ARRAY_REF, TYPE_ID_ARRAY_SCALAR, TYPE_ID_BOX_SCALAR};

// o loi kieu errno

// Giu ben Rust chu khong phai mot chuoi Pump: chuoi Pump nam tren heap cua GC
// nen se phai them mot goc nua, con cai nay thi khong lien quan gi den GC.
// Chi luc ai do hoi pump_os_error moi duc ra mot chuoi that.
static LAST_ERROR: Global<Option<String>> = Global::new(None);

fn clear_error() {
    *LAST_ERROR.get() = None;
}

fn record(error: impl std::fmt::Display) {
    *LAST_ERROR.get() = Some(error.to_string());
}

/// The message of the most recent failed call, for tests.
pub fn last_error() -> Option<String> {
    LAST_ERROR.get().clone()
}

// Muon mot `&str` tu mot chuoi Pump. Chuoi Pump luon la UTF-8 hop le, nen cho
// duy nhat hong duoc la con tro null.
fn borrow(s: *const u8) -> Option<&'static str> {
    if s.is_null() {
        record("a null string reached the operating system");
        return None;
    }
    Some(unsafe { text_of(s) })
}

// ===== doc file =====

/// Reads a whole file as text. Null when it cannot be read, or when its bytes
/// are not valid UTF-8; `pump_os_error` then says why.
#[no_mangle]
pub extern "C" fn pump_read_file_text(path: *const u8) -> *mut u8 {
    clear_error();
    let Some(path) = borrow(path) else {
        return ptr::null_mut();
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            record(error);
            return ptr::null_mut();
        }
    };
    // Chuoi Pump la UTF-8 hop le theo dinh nghia, nen file khong phai UTF-8
    // la mot loi chu khong phai cai de thay bang U+FFFD am tham.
    match std::str::from_utf8(&bytes) {
        Ok(text) => new_string(text.as_bytes()),
        Err(error) => {
            record(format_args!("not valid UTF-8: {error}"));
            ptr::null_mut()
        }
    }
}

/// Reads a whole file as bytes, one `int` per byte, each 0..255. Null when it
/// cannot be read.
#[no_mangle]
pub extern "C" fn pump_read_file_bytes(path: *const u8) -> *mut u8 {
    clear_error();
    let Some(path) = borrow(path) else {
        return ptr::null_mut();
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            record(error);
            return ptr::null_mut();
        }
    };

    let scope = RootScope::new();
    let array = scope.keep(pump_array_new(TYPE_ID_ARRAY_SCALAR, bytes.len() as u64));
    for byte in bytes {
        pump_array_push(array, u64::from(byte));
    }
    array
}

// ===== ghi file =====

/// Writes `data` over whatever `path` held. 1 on success, 0 on failure.
#[no_mangle]
pub extern "C" fn pump_write_file_text(path: *const u8, data: *const u8) -> i8 {
    clear_error();
    let (Some(path), Some(data)) = (borrow(path), borrow(data)) else {
        return 0;
    };
    match std::fs::write(path, data.as_bytes()) {
        Ok(()) => 1,
        Err(error) => {
            record(error);
            0
        }
    }
}

/// Writes the bytes of `data` over whatever `path` held. Every element must
/// be in 0..255. 1 on success, 0 on failure.
#[no_mangle]
pub extern "C" fn pump_write_file_bytes(path: *const u8, data: *const u8) -> i8 {
    clear_error();
    let Some(path) = borrow(path) else {
        return 0;
    };
    if data.is_null() {
        record("a null array reached the operating system");
        return 0;
    }

    // Kiem chu khong cat bot: `int` chua duoc ca ty gia tri khong phai byte,
    // va lang le lay tam bit thap thi mot cai bug shift sai se ghi ra file
    // mot cach im lang. Ai muon cat thi tu viet `& 0xff` lay.
    let count = pump_array_len(data);
    let mut buffer = Vec::with_capacity(count as usize);
    for index in 0..count as i64 {
        let value = pump_array_get(data, index) as i64;
        match u8::try_from(value) {
            Ok(byte) => buffer.push(byte),
            Err(_) => {
                record(format_args!(
                    "element {index} is {value}, which is not a byte 0..255"
                ));
                return 0;
            }
        }
    }

    match std::fs::write(path, &buffer) {
        Ok(()) => 1,
        Err(error) => {
            record(error);
            0
        }
    }
}

// ===== dong lenh va tien trinh =====

/// Every argument the process started with, the program name first.
#[no_mangle]
pub extern "C" fn pump_os_args() -> *mut u8 {
    clear_error();
    let arguments = crate::start::arguments();

    let scope = RootScope::new();
    let array = scope.keep(pump_array_new(TYPE_ID_ARRAY_REF, arguments.len() as u64));
    for argument in &arguments {
        pump_array_push(array, new_string(argument.as_bytes()) as u64);
    }
    array
}

/// Runs `program` with `arguments` to completion. Gives back a boxed `int?`
/// holding the exit code, or null when the program could not be started.
#[no_mangle]
pub extern "C" fn pump_os_run(program: *const u8, arguments: *const u8) -> *mut u8 {
    clear_error();
    let Some(program) = borrow(program) else {
        return ptr::null_mut();
    };

    let mut command = Command::new(program);
    if !arguments.is_null() {
        let count = pump_array_len(arguments);
        for index in 0..count as i64 {
            let slot = pump_array_get(arguments, index) as *const u8;
            if slot.is_null() {
                record(format_args!("argument {index} is null"));
                return ptr::null_mut();
            }
            command.arg(unsafe { text_of(slot) });
        }
    }

    // Con no viet chung stdout voi minh, nen day het cho minh ra truoc, khong
    // thi thu tu doc duoc lai khong phai thu tu viet.
    let _ = std::io::stdout().flush();

    let status = match command.status() {
        Ok(status) => status,
        Err(error) => {
            record(error);
            return ptr::null_mut();
        }
    };
    match status.code() {
        Some(code) => pump_box_new(TYPE_ID_BOX_SCALAR, code as i64 as u64),
        None => {
            // POSIX: bi signal giet thi khong co ma thoat nao ca.
            record("the process was ended by a signal");
            ptr::null_mut()
        }
    }
}

/// The message of the most recent failed call, or `""` when the most recent
/// call went through.
#[no_mangle]
pub extern "C" fn pump_os_error() -> *mut u8 {
    match LAST_ERROR.get() {
        Some(message) => new_string(message.as_bytes()),
        None => new_string(&[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string::pump_string_new;
    use crate::testing;

    fn string(text: &str) -> *mut u8 {
        pump_string_new(text.as_ptr(), text.len() as u64)
    }

    fn read_text(s: *const u8) -> String {
        unsafe { text_of(s).to_owned() }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("pump-os-test-{name}"));
        path
    }

    #[test]
    fn text_goes_out_and_comes_back() {
        let _guard = testing::guard();
        let path = scratch("text.txt");
        let _ = std::fs::remove_file(&path);

        let name = string(&path.to_string_lossy());
        assert_eq!(pump_write_file_text(name, string("xin chao")), 1);
        assert!(last_error().is_none());

        let back = pump_read_file_text(name);
        assert!(!back.is_null());
        assert_eq!(read_text(back), "xin chao");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_reads_as_null_with_a_message() {
        let _guard = testing::guard();
        let name = string(&scratch("nothing-here.txt").to_string_lossy());

        assert!(pump_read_file_text(name).is_null());
        assert!(last_error().is_some());
        assert!(!read_text(pump_os_error()).is_empty());

        assert!(pump_read_file_bytes(name).is_null());
        assert!(last_error().is_some());
    }

    #[test]
    fn bytes_go_out_and_come_back_one_element_each() {
        let _guard = testing::guard();
        let path = scratch("bytes.bin");
        let _ = std::fs::remove_file(&path);
        let name = string(&path.to_string_lossy());

        let data = pump_array_new(TYPE_ID_ARRAY_SCALAR, 4);
        for byte in [0u64, 127, 128, 255] {
            pump_array_push(data, byte);
        }
        assert_eq!(pump_write_file_bytes(name, data), 1);

        let back = pump_read_file_bytes(name);
        assert!(!back.is_null());
        assert_eq!(pump_array_len(back), 4);
        assert_eq!(pump_array_get(back, 0), 0);
        assert_eq!(pump_array_get(back, 1), 127);
        assert_eq!(pump_array_get(back, 2), 128);
        assert_eq!(pump_array_get(back, 3), 255);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_value_outside_a_byte_is_refused_rather_than_truncated() {
        let _guard = testing::guard();
        let path = scratch("refused.bin");
        let _ = std::fs::remove_file(&path);
        let name = string(&path.to_string_lossy());

        let data = pump_array_new(TYPE_ID_ARRAY_SCALAR, 2);
        pump_array_push(data, 1);
        pump_array_push(data, 256);
        assert_eq!(pump_write_file_bytes(name, data), 0);

        let message = last_error().expect("a refusal leaves a message");
        assert!(message.contains("256"), "{message}");
        assert!(message.contains("0..255"), "{message}");

        let negative = pump_array_new(TYPE_ID_ARRAY_SCALAR, 1);
        pump_array_push(negative, (-1i64) as u64);
        assert_eq!(pump_write_file_bytes(name, negative), 0);
    }

    #[test]
    fn a_successful_call_clears_the_message() {
        let _guard = testing::guard();
        let missing = string(&scratch("still-nothing.txt").to_string_lossy());
        assert!(pump_read_file_text(missing).is_null());
        assert!(last_error().is_some());

        let path = scratch("clears.txt");
        let name = string(&path.to_string_lossy());
        assert_eq!(pump_write_file_text(name, string("ok")), 1);
        assert!(last_error().is_none());
        assert_eq!(read_text(pump_os_error()), "");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn arguments_come_back_as_an_array_of_strings() {
        let _guard = testing::guard();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        // pump_rt_init khong nhan argv nao trong test, nen day la mang rong.
        let args = pump_os_args();
        assert_eq!(pump_array_len(args), 0);
    }

    #[test]
    fn a_program_that_cannot_start_gives_null_and_a_message() {
        let _guard = testing::guard();
        let missing = string("pump-no-such-program-anywhere");
        assert!(pump_os_run(missing, ptr::null()).is_null());
        assert!(last_error().is_some());
    }
}
