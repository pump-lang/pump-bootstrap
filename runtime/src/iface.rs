// gia tri interface, closure, va hop.
//
// gia tri interface la mot con tro beo duoc dong hop: 32 byte, itable tinh o
// +16 va object o +24. Dong hop thay vi be hai tu di khap noi de moi gia tri
// Pump deu rong dung mot tu, the la quy uoc goi va GC deu dong deu. Gia phai
// tra la mot lan cap phat cho moi lan doi, ma doi thi it hon goi nhieu.
//
// Dispatch la hai lan nap cong mot lan goi gian tiep, va no xay ra het trong
// code sinh ra, khong dong nao trong file nay nam tren duong do. Slot k cua
// itable o `itable + 24 + 8*k`, receiver di vao lam tham so vat ly so 0.
//
// closure: con tro code tho o +16, so capture o +24, roi moi capture mot con
// tro tu +32. Moi capture tro toi mot cai hop dung chung voi tat ca ai cung
// capture cai bien do, vi closure bat BIEN chu khong bat gia tri.
//
// GC khong bao gio di theo con tro itable, cung khong di theo con tro code.
// Ca hai deu khong phai object cua GC.

use crate::alloc::pump_alloc;
use crate::gc::RootScope;
use crate::{align_object_size, read_u64, write_ptr, write_u64, HEADER_SIZE, SLOT_SIZE};

// hinh dang

/// A box's single 8-byte slot, widened per `docs/abi.md` section 2.2.
pub const BOX_VALUE_OFFSET: usize = HEADER_SIZE;

/// Total size of a box object.
pub const BOX_SIZE: u64 = 32;

/// Address of a closure's compiled body.
pub const CODE_OFFSET: usize = HEADER_SIZE;

/// Number of captured bindings a closure holds.
pub const CAPTURE_COUNT_OFFSET: usize = HEADER_SIZE + 8;

/// First capture slot.
pub const CAPTURES_OFFSET: usize = HEADER_SIZE + 16;

/// Total size of a closure with `count` captures, before alignment.
pub const fn closure_unaligned_size(count: u64) -> u64 {
    CAPTURES_OFFSET as u64 + count * SLOT_SIZE as u64
}

/// Byte offset of capture `index` within a closure object.
pub const fn capture_offset(index: u64) -> usize {
    CAPTURES_OFFSET + index as usize * SLOT_SIZE
}

/// The static itable of an interface value.
pub const IFACE_ITABLE_OFFSET: usize = HEADER_SIZE;

/// The underlying object of an interface value, or a box when the concrete
/// type is a primitive.
pub const IFACE_DATA_OFFSET: usize = HEADER_SIZE + 8;

/// Total size of an interface value.
pub const IFACE_SIZE: u64 = 32;

// itables

/// The interface's `DefId`, at the start of an itable.
pub const ITABLE_INTERFACE_ID_OFFSET: usize = 0;

/// The concrete type's runtime type id.
pub const ITABLE_TYPE_ID_OFFSET: usize = 8;

/// The number of method pointers that follow.
pub const ITABLE_METHOD_COUNT_OFFSET: usize = 16;

/// The first method pointer.
pub const ITABLE_METHODS_OFFSET: usize = 24;

/// Byte offset of the method pointer for interface slot `slot`.
pub const fn itable_method_offset(slot: u64) -> usize {
    ITABLE_METHODS_OFFSET + slot as usize * SLOT_SIZE
}

/// Total size of an itable with `method_count` methods.
pub const fn itable_size(method_count: u64) -> u64 {
    ITABLE_METHODS_OFFSET as u64 + method_count * SLOT_SIZE as u64
}

/// The `DefId` of the interface an itable implements.
pub unsafe fn itable_interface_id(itable: *const u8) -> u64 {
    read_u64(itable, ITABLE_INTERFACE_ID_OFFSET)
}

/// The runtime type id of the concrete type an itable serves.
pub unsafe fn itable_type_id(itable: *const u8) -> u64 {
    read_u64(itable, ITABLE_TYPE_ID_OFFSET)
}

/// The number of methods an itable carries.
pub unsafe fn itable_method_count(itable: *const u8) -> u64 {
    read_u64(itable, ITABLE_METHOD_COUNT_OFFSET)
}

/// The code address in slot `slot` of an itable.
pub unsafe fn itable_method(itable: *const u8, slot: u64) -> *const u8 {
    read_u64(itable, itable_method_offset(slot)) as *const u8
}

// constructors

/// Boxes `data` together with `itable` into an interface value.
#[no_mangle]
pub extern "C" fn pump_iface_new(itable: *const u8, data: *mut u8) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep(data);
    let value = pump_alloc(crate::TYPE_ID_INTERFACE, IFACE_SIZE);
    unsafe {
        write_ptr(value, IFACE_ITABLE_OFFSET, itable as *mut u8);
        write_ptr(value, IFACE_DATA_OFFSET, data);
    }
    value
}

/// A closure with `capture_count` capture slots, all null.
#[no_mangle]
pub extern "C" fn pump_closure_new(type_id: u32, code: *const u8, capture_count: u64) -> *mut u8 {
    let size = align_object_size(closure_unaligned_size(capture_count));
    let closure = pump_alloc(type_id, size);
    unsafe {
        write_ptr(closure, CODE_OFFSET, code as *mut u8);
        write_u64(closure, CAPTURE_COUNT_OFFSET, capture_count);
    }
    closure
}

/// A box holding one 8-byte slot.
#[no_mangle]
pub extern "C" fn pump_box_new(type_id: u32, value: u64) -> *mut u8 {
    let scope = RootScope::new();
    scope.keep_slot(value, type_id == crate::TYPE_ID_BOX_REF);
    let boxed = pump_alloc(type_id, BOX_SIZE);
    unsafe { write_u64(boxed, BOX_VALUE_OFFSET, value) };
    boxed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use crate::{read_ptr, type_id_of};

    #[test]
    fn the_layout_constants_match_the_document() {
        assert_eq!(BOX_VALUE_OFFSET, 16);
        assert_eq!(CODE_OFFSET, 16);
        assert_eq!(CAPTURE_COUNT_OFFSET, 24);
        assert_eq!(CAPTURES_OFFSET, 32);
        assert_eq!(IFACE_ITABLE_OFFSET, 16);
        assert_eq!(IFACE_DATA_OFFSET, 24);
        assert_eq!(itable_method_offset(0), 24);
        assert_eq!(itable_method_offset(3), 48);
        assert_eq!(itable_size(2), 40);
        assert_eq!(align_object_size(closure_unaligned_size(0)), 32);
        assert_eq!(align_object_size(closure_unaligned_size(2)), 48);
    }

    #[test]
    fn an_interface_value_holds_its_itable_and_its_data() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let data = pump_alloc(types.node, 32);
        let itable = 0x1000_usize as *const u8;

        let value = pump_iface_new(itable, data);
        unsafe {
            assert_eq!(type_id_of(value), crate::TYPE_ID_INTERFACE);
            assert_eq!(read_ptr(value, IFACE_ITABLE_OFFSET), itable as *mut u8);
            assert_eq!(read_ptr(value, IFACE_DATA_OFFSET), data);
        }
    }

    #[test]
    fn a_closure_records_its_code_and_capture_count() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let code = 0x2000_usize as *const u8;

        let closure = pump_closure_new(types.closure, code, 3);
        unsafe {
            assert_eq!(crate::header(closure).size, 64);
            assert_eq!(read_ptr(closure, CODE_OFFSET), code as *mut u8);
            assert_eq!(read_u64(closure, CAPTURE_COUNT_OFFSET), 3);
            for index in 0..3 {
                assert!(read_ptr(closure, capture_offset(index)).is_null());
            }
        }
    }

    #[test]
    fn a_box_holds_one_widened_slot() {
        let _guard = testing::guard();

        let scalar = pump_box_new(crate::TYPE_ID_BOX_SCALAR, 0xdead_beef);
        unsafe {
            assert_eq!(crate::header(scalar).size, BOX_SIZE);
            assert_eq!(read_u64(scalar, BOX_VALUE_OFFSET), 0xdead_beef);
        }
    }
}
