// khoi dong va tat may.
//
// module nay CO Y khong dinh nghia `main`. Compiler sinh ra cai do, va nho
// the ma pumpc link duoc crate nay nhu mot dependency Rust binh thuong cho
// JIT ma khong dung hai cai `main` vao nhau. Cai duoc sinh ra co dinh nhu vay:
//
//   int32_t main(int32_t argc, char argv) {
//       void *stack_bottom = &argc;
//       pump_rt_init(stack_bottom, pump_type_table, type_count,
//                    pump_global_roots, root_count, argc, argv);
//       pump_module_init();
//       int32_t code = pump_program_main();
//       pump_rt_shutdown(code);
//       return code;
//   }
//
// `pump run` lam thang bon buoc do chu khong di qua cai main nay.
//
// Cai stack_bottom ma main truyen vao chinh la thu lam cho viec gom rac kha
// thi. No la dau xa cua khoang ma GC quet, va chua ghi duoc no thi
// pump_gc_collect tu choi chay, vi doan bua se hoac sot goc dang song hoac
// cai nay doc vao vung

use std::io::Write;

use crate::gc::collector;
use crate::Global;

struct Arguments {
    argc: i32,
    argv: *const *const u8,
}

static ARGUMENTS: Global<Arguments> = Global::new(Arguments {
    argc: 0,
    argv: std::ptr::null(),
});

/// Initialises the runtime.
#[no_mangle]
pub extern "C" fn pump_rt_init(
    stack_bottom: *const u8,
    type_table: *const crate::TypeDescriptor,
    type_count: u64,
    global_roots: *mut *mut u8,
    root_count: u64,
    argc: i32,
    argv: *const *const u8,
) {
    let collector = collector();
    collector.stack_bottom = stack_bottom as usize;
    collector.type_table = type_table;
    collector.type_count = type_count;
    collector.global_roots = global_roots;
    collector.root_count = root_count;

    let arguments = ARGUMENTS.get();
    arguments.argc = argc;
    arguments.argv = argv;
}

/// Flushes output and releases the heap.
#[no_mangle]
pub extern "C" fn pump_rt_shutdown(exit_code: i32) {
    let _ = exit_code;

    let collector = collector();
    collector.stack_bottom = 0;
    collector.type_table = std::ptr::null();
    collector.type_count = 0;
    collector.global_roots = std::ptr::null_mut();
    collector.root_count = 0;

    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    crate::alloc::heap().release();
}

/// The number of process arguments `main` received.
pub fn argument_count() -> i32 {
    ARGUMENTS.get().argc
}

/// The process arguments as text, with any non-UTF-8 byte replaced.
pub fn arguments() -> Vec<String> {
    let arguments = ARGUMENTS.get();
    if arguments.argv.is_null() || arguments.argc <= 0 {
        return Vec::new();
    }
    (0..arguments.argc as usize)
        .filter_map(|index| unsafe {
            let entry = *arguments.argv.add(index);
            if entry.is_null() {
                return None;
            }
            let bytes = std::ffi::CStr::from_ptr(entry as *const std::ffi::c_char).to_bytes();
            Some(String::from_utf8_lossy(bytes).into_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn init_records_the_scan_range_and_the_type_table() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        assert_ne!(collector().stack_bottom, 0);
        assert!(crate::gc::can_collect());
        assert!(crate::gc::descriptor(types.node).is_some());
    }

    #[test]
    fn shutdown_stops_collection_and_gives_the_heap_back() {
        let _guard = testing::guard();
        let _types = testing::install_type_table();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        crate::alloc::pump_alloc(16, 64);
        assert!(crate::alloc::heap_bytes_reserved() > 0);

        pump_rt_shutdown(0);
        assert!(!crate::gc::can_collect());
        assert_eq!(crate::alloc::heap_bytes_reserved(), 0);

        // goi may lan cung the: `pump_exit` tat may tren duong ra, roi
        // `main` tat lan nua khi dieu khien quay ve no.
        pump_rt_shutdown(0);
    }

    #[test]
    fn arguments_are_empty_when_main_passed_none() {
        let _guard = testing::guard();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        assert_eq!(argument_count(), 0);
        assert!(arguments().is_empty());
    }
}
