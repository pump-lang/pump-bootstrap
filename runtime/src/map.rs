// map va set. Mot cai bang bam mo dia chi lam ca hai viec.
//
// docs/abi.md 4.3, 4.4 va 10. Set co dung cai hinh 80 byte y het map, cot
// value van nam do nhung khong dung bao gio, nen mot ban cai dat gach ca hai
// cai nay va crate::set chi
//
// Hinh dang: mot entry buffer day dac theo thu tu them vao, cong mot index
// buffer mo dia chi gom cac o i64. Chinh cai do lam cho thu tu duyet la thu
// tu them vao, va giu nguyen qua moi lan chay, moi lan build: cu di tu entry
// 0 den entry_used roi bo qua cai nao co tu hash bang 0.
//
// Buoc entry la 24 - hash o +0, key o +8, value o +16 - bat dau tu byte thu
// 16 cua object entry buffer. O index bang -1 la trong, -2 la bia mo, con lai
// la vi tri cua mot entry. Hash luu bang 0 nghia la trong hoac da xoa, nen
// hash that ma ra 0 thi luu thanh 1.
//
// Index luon la luy thua cua 2 va luon it nhat gap doi suc chua entry, nen
// nhieu nhat chi mot nua so o co the day hoac bia mo. Vi vay lan nao do cung
// gap mot o trong, va vong do khong can chan tren cua rieng no.
//
// Xoa thi cam bia mo vao o index va xoa trang entry nhung khong dong vao
// entry_used. May cai bia mo se duoc don sach o lan entry buffer day tiep
// theo, vi luc do_rehash chi chep entry con song, the la don goi ca the va van
// giu thu
//
// key_kind o +72 chon cach bam va cach so bang: so tung bit tren o, so theo
// noi dung UTF-8 neu la chuoi, hoac so theo dia chi object. Float khong bao
// cai nay  gio duoc

use crate::alloc::{pump_alloc, pump_alloc_buffer};
use crate::array::{empty_array, pump_array_push};
use crate::gc::{descriptor, RootScope};
use crate::panic::{abort_runtime, pump_panic_missing_key};
use crate::string::{pump_string_eq, pump_string_hash};
use crate::{
    read_ptr, read_u32, read_u64, type_id_of, write_ptr, write_u32, write_u64,
    DESC_FLAG_ELEM_IS_REF, DESC_FLAG_KEY_IS_REF, DESC_FLAG_VALUE_IS_REF, HEADER_SIZE,
    TYPE_ID_ARRAY_REF, TYPE_ID_ARRAY_SCALAR, TYPE_ID_STRING,
};

// hat giong cua ham bam. FNV-1a chinh goc lay 0xcbf29ce484222325, t doi
// sang ngay sinh ...
// no la cua minh. Doi so nay cung khong sao, khong cho nao phu thuoc vao gia
// tri hash cu the ca, thu tu duyet la thu tu them vao chu khong phai thu tu
// bam.
pub const HASH_SEED: u64 = 0x2004_0417_5055_4d50;

// ===== hinh dang =====

/// Live entry count.
pub const LENGTH_OFFSET: usize = HEADER_SIZE;

/// The entry buffer object, or null.
pub const ENTRIES_OFFSET: usize = HEADER_SIZE;

/// Entry slots the buffer can hold.
pub const ENTRY_CAPACITY_OFFSET: usize = HEADER_SIZE + 16;

/// Entry slots consumed, tombstones included.
pub const ENTRY_USED_OFFSET: usize = HEADER_SIZE + 24;

/// The index buffer object, or null.
pub const INDEX_OFFSET: usize = HEADER_SIZE + 32;

/// Index slots, always a power of two.
pub const INDEX_CAPACITY_OFFSET: usize = HEADER_SIZE + 40;

/// Modification counter.
pub const MODCOUNT_OFFSET: usize = HEADER_SIZE + 48;

/// One of the `KEY_KIND_*` constants.
pub const KEY_KIND_OFFSET: usize = HEADER_SIZE + 56;

/// `SLOT_FLAG_*` bits.
pub const SLOT_FLAGS_OFFSET: usize = HEADER_SIZE + 60;

/// Total size of a map or set object, which never varies.
pub const SIZE: u64 = HEADER_SIZE as u64 + 64;

/// Bytes per entry in the entry buffer.
pub const ENTRY_SIZE: usize = 24;

/// The entry's hash, within an entry.
pub const ENTRY_HASH_OFFSET: usize = 0;

/// The key slot, within an entry.
pub const ENTRY_KEY_OFFSET: usize = 8;

/// The value slot, within an entry.
pub const ENTRY_VALUE_OFFSET: usize = 16;

/// Bytes per slot in the index buffer.
pub const INDEX_SLOT_SIZE: usize = 8;

/// Index slot value meaning "no entry".
pub const INDEX_EMPTY: i64 = -1;

/// Index slot value meaning "an entry was here and was removed".
pub const INDEX_TOMBSTONE: i64 = -2;

/// Key compared bitwise on its 8-byte slot: `int`, `uint`, `char`, `bool`,
/// and payload-free enums.
pub const KEY_KIND_SCALAR: u32 = 0;

/// Key is a `string`, hashed and compared by UTF-8 content.
pub const KEY_KIND_STRING: u32 = 1;

/// Key compared by object identity.
pub const KEY_KIND_REFERENCE: u32 = 2;

/// Key is a tuple, hashed and compared structurally.
pub const KEY_KIND_TUPLE: u32 = 3;

/// `slot_flags` bit: the value slot holds a pointer.
pub const SLOT_FLAG_VALUE_IS_REF: u32 = 1 << 0;

/// `slot_flags` bit: the key slot holds a pointer.
pub const SLOT_FLAG_KEY_IS_REF: u32 = 1 << 1;

/// The smallest entry buffer the runtime allocates.
pub const MIN_ENTRY_CAPACITY: u64 = 8;

/// Byte offset of entry `index` within the entry buffer object.
pub const fn entry_offset(index: u64) -> usize {
    HEADER_SIZE + index as usize * ENTRY_SIZE
}

/// Byte offset of index slot `slot` within the index buffer object.
pub const fn index_slot_offset(slot: usize) -> usize {
    HEADER_SIZE + slot * INDEX_SLOT_SIZE
}

// ===== doc ghi field =====

macro_rules! table_field {
    ($read:ident, $write:ident, $offset:ident, $doc:literal) => {
        #[doc = $doc]
        #[inline]
        pub(crate) unsafe fn $read(table: *const u8) -> u64 {
            read_u64(table, $offset)
        }

        #[doc = $doc]
        #[inline]
        unsafe fn $write(table: *mut u8, value: u64) {
            write_u64(table, $offset, value);
        }
    };
}

table_field!(length, set_length, LENGTH_OFFSET, "The live entry count.");
table_field!(
    entry_capacity,
    set_entry_capacity,
    ENTRY_CAPACITY_OFFSET,
    "The entry slots the buffer can hold."
);
table_field!(
    entry_used,
    set_entry_used,
    ENTRY_USED_OFFSET,
    "The entry slots consumed, tombstones included."
);
table_field!(
    index_capacity,
    set_index_capacity,
    INDEX_CAPACITY_OFFSET,
    "The index slot count, always a power of two."
);

#[inline]
unsafe fn slot_flags(table: *const u8) -> u32 {
    read_u32(table, SLOT_FLAGS_OFFSET)
}

#[inline]
unsafe fn bump_modcount(table: *mut u8) {
    write_u64(
        table,
        MODCOUNT_OFFSET,
        read_u64(table, MODCOUNT_OFFSET).wrapping_add(1),
    );
}

#[inline]
unsafe fn entry_field(entries: *const u8, position: u64, field: usize) -> u64 {
    read_u64(entries, entry_offset(position) + field)
}

#[inline]
unsafe fn set_entry_field(entries: *mut u8, position: u64, field: usize, value: u64) {
    write_u64(entries, entry_offset(position) + field, value);
}
