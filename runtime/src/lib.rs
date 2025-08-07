// runtime cua Pump.
//
// link tinh vao moi chuong trinh Pump da dich, va link luon vao ca pumpc de
// `pump run` con dua duoc may dia chi nay cho JIT.
//
// Moi cua vao pub trong crate nay la mot nua cua mot giao keo ghi trong
// docs/abi.md. Nua kia la src/clif.rs. Chi co moi cai tai lieu do giu cho hai
// ben con noi chuyen duoc voi nhau, nen bam sat no tung chu: dung offset,
// dung ten symbol, dung chu ky.
//
// Ban ngan gon. Object nao tren heap cung mo dau bang header 16 byte: type_id
// u32 o +0, flags u32 o +4, size u64 o +8, payload tu +16. type_id la chi so
// vao bang type descriptor tinh ma chuong trinh dua cho pump_rt_init, va
// chinh cai bang do lam cho GC di duoc het heap mot cach chinh xac trong khi
// van quet stack kieu bao thu. Collection giu moi phan tu, moi key, moi value
// trong mot o 8 byte. Optional la con tro, null la so 0. Loi goi co the that
// bai thi bao qua bien toan cuc pump_error_slot chu khong qua gia tri tra ve.
//
// Mot luong, khong khoa cho nao het. 1.0 chua co dong thoi.

use std::cell::UnsafeCell;

pub mod alloc;
pub mod array;
pub mod gc;
pub mod iface;
pub mod map;
pub mod os;
pub mod panic;
pub mod set;
pub mod start;
pub mod string;

#[cfg(test)]
pub(crate) mod testing;

/// Header sitting in front of every heap object.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ObjectHeader {
    pub type_id: u32,
    pub flags: u32,
    pub size: u64,
}

/// Size of ObjectHeader in bytes.
pub const HEADER_SIZE: usize = 16;

/// Alignment of every heap object. Payload nho do cung can 16.
pub const OBJECT_ALIGN: usize = 16;

/// Header flag: GC da cham toi trong luot mark nay.
pub const FLAG_MARK: u32 = 1 << 0;

/// Header flag: object tinh, khong bao gio quet, khong bao gio giai phong.
pub const FLAG_IMMORTAL: u32 = 1 << 1;

/// O nao trong array, map hay set cung rong 8 byte, kieu gi cung the.
pub const SLOT_SIZE: usize = 8;

// may type id giu san. docs/abi.md muc 3.

/// Never a live object; a freed cell carries it.
pub const TYPE_ID_INVALID: u32 = 0;
/// A raw byte buffer, never traced through its own descriptor.
pub const TYPE_ID_BUFFER: u32 = 1;
/// A `string`.
pub const TYPE_ID_STRING: u32 = 2;
/// A box holding one non-pointer slot.
pub const TYPE_ID_BOX_SCALAR: u32 = 3;
/// A box holding one pointer slot.
pub const TYPE_ID_BOX_REF: u32 = 4;
/// An interface value.
pub const TYPE_ID_INTERFACE: u32 = 5;

/// An array whose element slots hold scalars, built by the runtime itself.
pub const TYPE_ID_ARRAY_SCALAR: u32 = 6;

/// An array whose element slots hold pointers, built by the runtime itself.
pub const TYPE_ID_ARRAY_REF: u32 = 7;

/// The first type id available to compiler-emitted descriptors.
pub const FIRST_USER_TYPE_ID: u32 = 16;

/// Rounds an object size up to `OBJECT_ALIGN`.
pub const fn align_object_size(size: u64) -> u64 {
    let align = OBJECT_ALIGN as u64;
    (size + align - 1) & !(align - 1)
}

impl ObjectHeader {
    pub fn is_marked(&self) -> bool {
        self.flags & FLAG_MARK != 0
    }

    pub fn is_immortal(&self) -> bool {
        self.flags & FLAG_IMMORTAL != 0
    }
}

/// One row of the static type descriptor table.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TypeDescriptor {
    pub kind: u32,
    pub flags: u32,
    pub size: u64,
    pub ref_count: u32,
    pub variant_count: u32,
    pub ref_offsets: *const u32,
    pub variants: *const VariantDescriptor,
    pub name: *const u8,
}

/// Ban do con tro cua mot variant, dung cho descriptor cua enum.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VariantDescriptor {
    pub ref_count: u32,
    pub reserved: u32,
    pub ref_offsets: *const u32,
    pub name: *const u8,
}

/// How the collector walks an object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum DescriptorKind {
    Struct = 0,
    Enum = 1,
    Tuple = 2,
    Array = 3,
    Map = 4,
    Set = 5,
    Closure = 6,
    String = 7,
    Box = 8,
    Interface = 9,
    Buffer = 10,
}

impl DescriptorKind {
    /// Doc tu `kind` cua descriptor ra. None neu no ngoai khoang.
    pub fn from_u32(value: u32) -> Option<DescriptorKind> {
        use DescriptorKind::*;
        Some(match value {
            0 => Struct,
            1 => Enum,
            2 => Tuple,
            3 => Array,
            4 => Map,
            5 => Set,
            6 => Closure,
            7 => String,
            8 => Box,
            9 => Interface,
            10 => Buffer,
            _ => return None,
        })
    }
}

/// Tag variant cua mot gia tri enum, chinh la chi so khai bao.
pub const ENUM_TAG_OFFSET: usize = HEADER_SIZE;

/// Descriptor flag: an array's, set's or box's slot holds a pointer.
pub const DESC_FLAG_ELEM_IS_REF: u32 = 1 << 0;
/// Descriptor flag: a map's key slot holds a pointer.
pub const DESC_FLAG_KEY_IS_REF: u32 = 1 << 1;
/// Descriptor flag: a map's value slot holds a pointer.
pub const DESC_FLAG_VALUE_IS_REF: u32 = 1 << 2;

// bien toan cuc, ca tien trinh mot luong.
//
// runtime chay mot luong, va day la tinh chat cua ngon ngu chu khong phai cua
// rieng ban cai dat nay: `spawn` voi channel deu nam ngoai 1.0, nen khong
// chuong trinh Pump nao goi vao day tu mot luong thu hai duoc.

pub(crate) struct Global<T> {
    cell: UnsafeCell<T>,
}

unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub(crate) const fn new(value: T) -> Global<T> {
        Global {
            cell: UnsafeCell::new(value),
        }
    }

    #[allow(clippy::mut_from_ref)]
    pub(crate) fn get(&self) -> &mut T {
        unsafe { &mut *self.cell.get() }
    }
}

// doc ghi field tho

#[inline]
pub(crate) unsafe fn header<'a>(object: *mut u8) -> &'a mut ObjectHeader {
    &mut *(object as *mut ObjectHeader)
}

#[inline]
pub(crate) unsafe fn type_id_of(object: *const u8) -> u32 {
    (*(object as *const ObjectHeader)).type_id
}

#[inline]
pub(crate) unsafe fn read_u64(object: *const u8, offset: usize) -> u64 {
    (object.add(offset) as *const u64).read()
}

#[inline]
pub(crate) unsafe fn write_u64(object: *mut u8, offset: usize, value: u64) {
    (object.add(offset) as *mut u64).write(value);
}

#[inline]
pub(crate) unsafe fn read_u32(object: *const u8, offset: usize) -> u32 {
    (object.add(offset) as *const u32).read()
}

#[inline]
pub(crate) unsafe fn write_u32(object: *mut u8, offset: usize, value: u32) {
    (object.add(offset) as *mut u32).write(value);
}

#[inline]
pub(crate) unsafe fn read_ptr(object: *const u8, offset: usize) -> *mut u8 {
    read_u64(object, offset) as *mut u8
}

#[inline]
pub(crate) unsafe fn write_ptr(object: *mut u8, offset: usize, value: *mut u8) {
    write_u64(object, offset, value as u64);
}

// dang ky symbol

/// Every runtime entry point with its address, for the JIT to hand to
/// JITBuilder::symbol.
pub fn runtime_symbols() -> Vec<(&'static str, *const u8)> {
    use crate::alloc::{pump_alloc, pump_alloc_buffer};
    use crate::array::{
        pump_array_concat, pump_array_get, pump_array_len, pump_array_new, pump_array_pop,
        pump_array_push, pump_array_reserve, pump_array_set, pump_array_slice,
        pump_array_with_length, pump_collection_modcount,
    };
    use crate::gc::{
        pump_gc_add_root, pump_gc_collect, pump_gc_disable, pump_gc_enable, pump_gc_remove_root,
    };
    use crate::iface::{pump_box_new, pump_closure_new, pump_iface_new};
    use crate::map::{
        pump_map_get, pump_map_has, pump_map_iter_next, pump_map_keys, pump_map_len,
        pump_map_lookup, pump_map_new, pump_map_remove, pump_map_set, pump_map_values,
    };
    use crate::panic::{
        pump_error_pending, pump_error_set, pump_error_take, pump_exit, pump_panic,
        pump_panic_concurrent_modification, pump_panic_cstr, pump_panic_divide_by_zero,
        pump_panic_index, pump_panic_missing_key, pump_panic_negative_shift, pump_panic_null,
    };
    use crate::os::{
        pump_os_args, pump_os_error, pump_os_run, pump_read_file_bytes, pump_read_file_text,
        pump_write_file_bytes, pump_write_file_text,
    };
    use crate::set::{
        pump_set_add, pump_set_has, pump_set_iter_next, pump_set_len, pump_set_new, pump_set_remove,
    };
    use crate::start::{pump_rt_init, pump_rt_shutdown};
    use crate::string::{
        pump_char_from_uint, pump_print, pump_print_error, pump_println, pump_string_byte_at,
        pump_string_char_count, pump_string_chars, pump_string_cmp, pump_string_concat,
        pump_string_eq, pump_string_from_bool, pump_string_from_char, pump_string_from_float,
        pump_string_from_int, pump_string_from_uint, pump_string_hash, pump_string_len,
        pump_string_new, pump_string_slice,
    };

    macro_rules! entries {
        ($($name:ident),* $(,)?) => {
            vec![$((stringify!($name), $name as *const () as *const u8)),*]
        };
    }

    let mut symbols: Vec<(&'static str, *const u8)> = entries![
        pump_rt_init,
        pump_rt_shutdown,
        pump_alloc,
        pump_alloc_buffer,
        pump_gc_collect,
        pump_gc_disable,
        pump_gc_enable,
        pump_gc_add_root,
        pump_gc_remove_root,
        pump_panic,
        pump_panic_cstr,
        pump_panic_index,
        pump_panic_divide_by_zero,
        pump_panic_null,
        pump_panic_negative_shift,
        pump_panic_missing_key,
        pump_panic_concurrent_modification,
        pump_exit,
        pump_error_set,
        pump_error_take,
        pump_error_pending,
        pump_string_new,
        pump_string_concat,
        pump_string_eq,
        pump_string_cmp,
        pump_string_hash,
        pump_string_len,
        pump_string_char_count,
        pump_string_byte_at,
        pump_string_slice,
        pump_string_chars,
        pump_string_from_char,
        pump_string_from_bool,
        pump_string_from_int,
        pump_string_from_uint,
        pump_string_from_float,
        pump_char_from_uint,
        pump_print,
        pump_println,
        pump_print_error,
        pump_array_new,
        pump_array_with_length,
        pump_array_len,
        pump_array_get,
        pump_array_set,
        pump_array_push,
        pump_array_pop,
        pump_array_reserve,
        pump_array_concat,
        pump_array_slice,
        pump_map_new,
        pump_map_len,
        pump_map_lookup,
        pump_map_get,
        pump_map_set,
        pump_map_remove,
        pump_map_has,
        pump_map_keys,
        pump_map_values,
        pump_map_iter_next,
        pump_set_new,
        pump_set_len,
        pump_set_add,
        pump_set_has,
        pump_set_remove,
        pump_set_iter_next,
        pump_collection_modcount,
        pump_closure_new,
        pump_box_new,
        pump_iface_new,
        pump_read_file_text,
        pump_read_file_bytes,
        pump_write_file_text,
        pump_write_file_bytes,
        pump_os_args,
        pump_os_run,
        pump_os_error,
    ];

    symbols.push((
        "pump_error_slot",
        std::ptr::addr_of!(crate::panic::pump_error_slot) as *const u8,
    ));
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<ObjectHeader>(), HEADER_SIZE);
    }

    #[test]
    fn a_type_descriptor_is_forty_eight_bytes() {
        assert_eq!(std::mem::size_of::<TypeDescriptor>(), 48);
        assert_eq!(std::mem::size_of::<VariantDescriptor>(), 24);
    }

    #[test]
    fn sizes_round_up_to_sixteen() {
        assert_eq!(align_object_size(1), 16);
        assert_eq!(align_object_size(16), 16);
        assert_eq!(align_object_size(17), 32);
        assert_eq!(align_object_size(33), 48);
    }

    #[test]
    fn every_runtime_symbol_is_distinct() {
        let symbols = runtime_symbols();
        let mut names: Vec<&str> = symbols.iter().map(|(name, _)| *name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate runtime symbol name");
        assert!(symbols.iter().all(|(_, address)| !address.is_null()));
    }
}
