// mang.
//
// docs/abi.md 4.2 va 10. Object array co dinh 48 byte - do dai o +16, suc
// chua o +24, con tro buffer o +32, modcount o +40 - tro toi mot buffer cap
// phat rieng, nen mot lan push khong bao gio lam xe dich chinh cai object
// array. Phan tu thu i nam o `data + 16 + i*8`, so 16 do la header cua chinh
// cai buffer.
//
// O nao cung 8 byte, kieu phan tu la gi cung the. Scalar be hon thi noi so 0
// vao, float thi luu nguyen mau bit cua no.
//
// modcount tang len moi lan co thay doi cau truc. Vong for chep lai mot ban
// roi moi vong lai so, do la cach ma "sua collection trong luc dang duyet"
// thanh mot cai panic chu khong thanh hong du lieu am tham.

use crate::alloc::{buffer_capacity, pump_alloc, pump_alloc_buffer};
use crate::gc::{trace_plan, RootScope, TracePlan};
use crate::panic::{abort_runtime, pump_panic_index};
use crate::{read_ptr, read_u64, type_id_of, write_ptr, write_u64, HEADER_SIZE, SLOT_SIZE};

// hinh dang

/// Number of live elements.
pub const LENGTH_OFFSET: usize = HEADER_SIZE;

/// Element slots the buffer can hold.
pub const CAPACITY_OFFSET: usize = HEADER_SIZE + 8;

/// The element buffer object, or null when the capacity is zero.
pub const DATA_OFFSET: usize = HEADER_SIZE + 16;

/// Modification counter, bumped by every structural change.
pub const MODCOUNT_OFFSET: usize = HEADER_SIZE + 24;

/// Total size of an array object, which never varies.
pub const SIZE: u64 = HEADER_SIZE as u64 + 32;

/// The smallest element buffer the runtime allocates.
pub const MIN_CAPACITY: u64 = 4;

/// Byte offset of element `index` within the element buffer object.
pub const fn element_offset(index: u64) -> usize {
    HEADER_SIZE + index as usize * SLOT_SIZE
}

// doc ghi field

#[inline]
pub(crate) unsafe fn length(array: *const u8) -> u64 {
    read_u64(array, LENGTH_OFFSET)
}

#[inline]
pub(crate) unsafe fn capacity(array: *const u8) -> u64 {
    read_u64(array, CAPACITY_OFFSET)
}

#[inline]
pub(crate) unsafe fn data(array: *const u8) -> *mut u8 {
    read_ptr(array, DATA_OFFSET)
}

#[inline]
pub(crate) unsafe fn element(array: *const u8, index: u64) -> u64 {
    read_u64(data(array), element_offset(index))
}

#[inline]
pub(crate) unsafe fn set_element(array: *mut u8, index: u64, value: u64) {
    write_u64(data(array), element_offset(index), value);
}

#[inline]
unsafe fn bump_modcount(array: *mut u8) {
    write_u64(
        array,
        MODCOUNT_OFFSET,
        read_u64(array, MODCOUNT_OFFSET).wrapping_add(1),
    );
}

unsafe fn elements_are_refs(array: *const u8) -> bool {
    matches!(
        trace_plan(type_id_of(array)),
        TracePlan::Array {
            elements_are_refs: true
        }
    )
}

// no ra

fn grow_to(array: *mut u8, wanted: u64) {
    unsafe {
        if wanted == 0 || wanted <= capacity(array) {
            return;
        }
        let Some(bytes) = wanted.checked_mul(SLOT_SIZE as u64) else {
            abort_runtime("an array grew past the addressable range");
        };

        let scope = RootScope::new();
        scope.keep(array);
        let buffer = pump_alloc_buffer(bytes);

        let live = length(array);
        let previous = data(array);
        if !previous.is_null() && live > 0 {
            std::ptr::copy_nonoverlapping(
                previous.add(HEADER_SIZE),
                buffer.add(HEADER_SIZE),
                live as usize * SLOT_SIZE,
            );
        }

        write_ptr(array, DATA_OFFSET, buffer);
        write_u64(
            array,
            CAPACITY_OFFSET,
            (buffer_capacity(buffer) / SLOT_SIZE) as u64,
        );
    }
}

fn grow_for_one_more(array: *mut u8) {
    unsafe {
        let capacity = capacity(array);
        if length(array) < capacity {
            return;
        }
        grow_to(array, (capacity * 2).max(MIN_CAPACITY));
    }
}

unsafe fn check_index(array: *const u8, index: i64) -> u64 {
    let live = length(array);
    if index < 0 || index as u64 >= live {
        pump_panic_index(index, live as i64);
    }
    index as u64
}

pub(crate) fn empty_array(type_id: u32) -> *mut u8 {
    pump_alloc(type_id, SIZE)
}

// cua vao

/// A new empty array with room for `capacity` elements.
#[no_mangle]
pub extern "C" fn pump_array_new(type_id: u32, capacity: u64) -> *mut u8 {
    let scope = RootScope::new();
    let array = scope.keep(empty_array(type_id));
    grow_to(array, capacity);
    array
}

/// A new array of `length` zeroed elements. Suc chua xin dung bang lenght
/// luon, vi cho nay gan nhu bao gio cung la dien cho day ngay sau do.
#[no_mangle]
pub extern "C" fn pump_array_with_length(type_id: u32, length: u64) -> *mut u8 {
    let array = pump_array_new(type_id, length);
    unsafe { write_u64(array, LENGTH_OFFSET, length) };
    array
}

/// The number of live elements.
#[no_mangle]
pub extern "C" fn pump_array_len(a: *const u8) -> u64 {
    unsafe { length(a) }
}

/// Element `index` as a raw 8-byte slot.
#[no_mangle]
pub extern "C" fn pump_array_get(a: *const u8, index: i64) -> u64 {
    unsafe {
        let index = check_index(a, index);
        element(a, index)
    }
}

/// Writes element `index`.
#[no_mangle]
pub extern "C" fn pump_array_set(a: *mut u8, index: i64, value: u64) {
    unsafe {
        let index = check_index(a, index);
        set_element(a, index, value);
    }
}

/// Appends an element, growing the buffer when needed and bumping `modcount`.
#[no_mangle]
pub extern "C" fn pump_array_push(a: *mut u8, value: u64) {
    let scope = RootScope::new();
    scope.keep(a);
    unsafe {
        scope.keep_slot(value, elements_are_refs(a));
        grow_for_one_more(a);
        let live = length(a);
        set_element(a, live, value);
        write_u64(a, LENGTH_OFFSET, live + 1);
        bump_modcount(a);
    }
}

/// Removes and returns the last element.
#[no_mangle]
pub extern "C" fn pump_array_pop(a: *mut u8) -> u64 {
    unsafe {
        let live = length(a);
        if live == 0 {
            // truoc t goi pump_panic_index(-1, 0) o day, nhung "index -1" la
            // cai chi so ma ban cai dat voi lay chu khong phai cai ma nguoi
            // viet code lam. `m[k]` voi `m.get(k)` di thanh cap, con mang thi
            // chi co moi nua panic, nen loi phai noi thang ra la pop mang rong.
            abort_runtime("pop from an empty array");
        }
        let last = live - 1;
        let value = element(a, last);
        set_element(a, last, 0);
        write_u64(a, LENGTH_OFFSET, last);
        bump_modcount(a);
        value
    }
}

/// Grows the buffer so it can hold at least `capacity` elements.
#[no_mangle]
pub extern "C" fn pump_array_reserve(a: *mut u8, capacity: u64) {
    unsafe {
        if capacity <= self::capacity(a) {
            return;
        }
        grow_to(a, capacity);
        bump_modcount(a);
    }
}

/// A new array holding `a` followed by `b`.
#[no_mangle]
pub extern "C" fn pump_array_concat(a: *const u8, b: *const u8) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep(a as *mut u8);
    scope.keep(b as *mut u8);
    unsafe {
        let left = length(a);
        let right = length(b);
        let result = scope.keep(pump_array_new(type_id_of(a), left + right));
        copy_elements(a, 0, result, 0, left);
        copy_elements(b, 0, result, left, right);
        write_u64(result, LENGTH_OFFSET, left + right);
        result
    }
}

/// A new array holding elements `start..end`, with `end` exclusive.
#[no_mangle]
pub extern "C" fn pump_array_slice(a: *const u8, start: i64, end: i64) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep(a as *mut u8);
    unsafe {
        let live = length(a) as i64;
        if start < 0 || start > live {
            pump_panic_index(start, live);
        }
        if end < start || end > live {
            pump_panic_index(end, live);
        }
        let count = (end - start) as u64;
        let result = scope.keep(pump_array_new(type_id_of(a), count));
        copy_elements(a, start as u64, result, 0, count);
        write_u64(result, LENGTH_OFFSET, count);
        result
    }
}

unsafe fn copy_elements(source: *const u8, from: u64, target: *mut u8, to: u64, count: u64) {
    if count == 0 {
        return;
    }
    std::ptr::copy_nonoverlapping(
        data(source).add(element_offset(from)),
        data(target).add(element_offset(to)),
        count as usize * SLOT_SIZE,
    );
}

/// The modification counter of an array, a map or a set, dispatching on the
/// descriptor kind.
#[no_mangle]
pub extern "C" fn pump_collection_modcount(collection: *const u8) -> u64 {
    unsafe {
        match trace_plan(type_id_of(collection)) {
            TracePlan::Array { .. } => read_u64(collection, MODCOUNT_OFFSET),
            TracePlan::Table { .. } => read_u64(collection, crate::map::MODCOUNT_OFFSET),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn slots(array: *const u8) -> Vec<u64> {
        unsafe {
            (0..length(array))
                .map(|index| element(array, index))
                .collect()
        }
    }

    #[test]
    fn the_layout_constants_match_the_document() {
        assert_eq!(LENGTH_OFFSET, 16);
        assert_eq!(CAPACITY_OFFSET, 24);
        assert_eq!(DATA_OFFSET, 32);
        assert_eq!(MODCOUNT_OFFSET, 40);
        assert_eq!(SIZE, 48);
        assert_eq!(element_offset(0), 16);
        assert_eq!(element_offset(3), 40);
    }

    #[test]
    fn a_new_array_is_empty_and_has_no_buffer() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let array = pump_array_new(types.scalar_array, 0);
        unsafe {
            assert_eq!(length(array), 0);
            assert_eq!(capacity(array), 0);
            assert!(data(array).is_null());
            assert_eq!(read_u64(array, MODCOUNT_OFFSET), 0);
        }
    }

    #[test]
    fn pushing_grows_the_buffer_and_keeps_the_elements() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let array = pump_array_new(types.scalar_array, 0);
        for value in 0..1000u64 {
            pump_array_push(array, value * 7);
        }

        assert_eq!(pump_array_len(array), 1000);
        for value in 0..1000u64 {
            assert_eq!(pump_array_get(array, value as i64), value * 7);
        }
        unsafe { assert!(capacity(array) >= 1000) };
    }

    #[test]
    fn a_structural_change_bumps_the_modification_counter() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let array = pump_array_new(types.scalar_array, 4);
        let before = pump_collection_modcount(array);
        pump_array_push(array, 1);
        let after_push = pump_collection_modcount(array);
        assert_ne!(before, after_push);

        pump_array_set(array, 0, 2);
        assert_eq!(pump_collection_modcount(array), after_push);

        pump_array_pop(array);
        assert_ne!(pump_collection_modcount(array), after_push);
    }

    #[test]
    fn popping_returns_the_last_element_and_clears_its_slot() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let array = pump_array_new(types.scalar_array, 4);
        pump_array_push(array, 10);
        pump_array_push(array, 20);

        assert_eq!(pump_array_pop(array), 20);
        assert_eq!(pump_array_len(array), 1);
        unsafe { assert_eq!(read_u64(data(array), element_offset(1)), 0) };
    }

    #[test]
    fn with_length_produces_zeroed_elements() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let array = pump_array_with_length(types.scalar_array, 5);
        assert_eq!(slots(array), vec![0; 5]);
    }

    #[test]
    fn concatenation_and_slicing_copy_the_right_ranges() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let left = pump_array_new(types.scalar_array, 0);
        let right = pump_array_new(types.scalar_array, 0);
        for value in 1..=3u64 {
            pump_array_push(left, value);
        }
        for value in 4..=6u64 {
            pump_array_push(right, value);
        }

        let joined = pump_array_concat(left, right);
        assert_eq!(slots(joined), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(slots(pump_array_slice(joined, 2, 5)), vec![3, 4, 5]);
        assert_eq!(slots(pump_array_slice(joined, 6, 6)), Vec::<u64>::new());
    }

    #[test]
    fn an_array_of_references_keeps_its_elements_alive() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        let scope = RootScope::new();
        let array = scope.keep(pump_array_new(types.ref_array, 0));
        for _ in 0..64 {
            pump_array_push(array, pump_alloc(types.node, 32) as u64);
        }

        crate::gc::pump_gc_collect();

        unsafe {
            for index in 0..length(array) {
                assert_eq!(type_id_of(element(array, index) as *const u8), types.node);
            }
        }
        drop(scope);
    }
}
