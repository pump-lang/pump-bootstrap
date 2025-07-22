// runtime cua Pump.
//
// link tinh vao moi chuong trinh Pump da dich, va link luon vao ca pumpc de
// `pump run` con dua duoc may dia chi nay cho JIT.
//
// Moi cua vao ...
// docs/abi.md. Nua kia la src/clif.rs. Chi co moi cai tai lieu do giu cho hai
// ben con noi ...
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
