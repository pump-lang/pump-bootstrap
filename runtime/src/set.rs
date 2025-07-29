// set.
//
// object set co dung cai hinh 80 byte y het map, ke ca cot value, chi la cot
// value khong bao gio dung toi. Phi 8 byte moi phan tu de doi lay mot ban cai
// dat bang bam thay vi hai, voi 1.0 thi doi the la duoc, nen o day cai gi
// cung goi thang sang crate::map. Phan tu nam o cot key, value luon bang 0.

use crate::map::{self, SLOT_FLAG_KEY_IS_REF};

/// A new empty set.
#[no_mangle]
pub extern "C" fn pump_set_new(type_id: u32) -> *mut u8 {
    map::new_table(type_id, true)
}

/// The number of elements.
#[no_mangle]
pub extern "C" fn pump_set_len(s: *const u8) -> u64 {
    unsafe { map::length(s) }
}

/// Inserts `element`.
#[no_mangle]
pub extern "C" fn pump_set_add(s: *mut u8, element: u64) -> i8 {
    i8::from(map::insert(s, element, 0))
}

/// Whether `element` is present.
#[no_mangle]
pub extern "C" fn pump_set_has(s: *const u8, element: u64) -> i8 {
    i8::from(unsafe { map::lookup(s, element) }.is_some())
}

/// Removes `element`.
#[no_mangle]
pub extern "C" fn pump_set_remove(s: *mut u8, element: u64) -> i8 {
    i8::from(map::remove(s, element))
}

/// Advances an iteration, in insertion order.
#[no_mangle]
pub extern "C" fn pump_set_iter_next(s: *const u8, cursor: *mut u64, out_element: *mut u64) -> i8 {
    i8::from(unsafe { map::iter_next(s, cursor, out_element, std::ptr::null_mut()) })
}

/// Whether this set's element slots hold references, which is what the shared
/// table uses to decide what has to be rooted across an allocation.
pub unsafe fn elements_are_refs(s: *const u8) -> bool {
    crate::read_u32(s, map::SLOT_FLAGS_OFFSET) & SLOT_FLAG_KEY_IS_REF != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::RootScope;
    use crate::string::pump_string_new;
    use crate::testing;

    fn string(text: &str) -> u64 {
        pump_string_new(text.as_ptr(), text.len() as u64) as u64
    }

    fn elements_in_order(set: *const u8) -> Vec<u64> {
        let mut cursor = 0u64;
        let mut element = 0u64;
        let mut collected = Vec::new();
        while pump_set_iter_next(set, &mut cursor, &mut element) != 0 {
            collected.push(element);
        }
        collected
    }

    #[test]
    fn adding_is_idempotent_and_reports_novelty() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let set = pump_set_new(types.scalar_set);
        assert_eq!(pump_set_add(set, 42), 1);
        assert_eq!(pump_set_add(set, 42), 0);
        assert_eq!(pump_set_len(set), 1);
        assert_eq!(pump_set_has(set, 42), 1);
        assert_eq!(pump_set_has(set, 43), 0);

        assert_eq!(pump_set_remove(set, 42), 1);
        assert_eq!(pump_set_remove(set, 42), 0);
        assert_eq!(pump_set_len(set), 0);
    }

    #[test]
    fn iteration_is_insertion_order() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let set = pump_set_new(types.scalar_set);
        for element in [9u64, 3, 7, 1, 5] {
            pump_set_add(set, element);
        }
        assert_eq!(elements_in_order(set), vec![9, 3, 7, 1, 5]);

        pump_set_remove(set, 7);
        assert_eq!(elements_in_order(set), vec![9, 3, 1, 5]);
    }

    #[test]
    fn string_elements_compare_by_content() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let set = pump_set_new(types.string_set);
        assert_eq!(pump_set_add(set, string("pump")), 1);
        assert_eq!(pump_set_add(set, string("pump")), 0);
        assert_eq!(pump_set_len(set), 1);
        unsafe { assert!(elements_are_refs(set)) };
    }

    #[test]
    fn a_set_keeps_its_elements_alive() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        let scope = RootScope::new();
        let set = scope.keep(pump_set_new(types.string_set));
        for index in 0..200u64 {
            pump_set_add(set, string(&format!("element {index}")));
        }

        crate::gc::pump_gc_collect();

        assert_eq!(pump_set_len(set), 200);
        for index in 0..200u64 {
            assert_eq!(pump_set_has(set, string(&format!("element {index}"))), 1);
        }
        drop(scope);
    }
}
