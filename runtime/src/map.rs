// map va set. Mot cai bang bam mo dia chi lam ca hai viec.
//
// docs/abi.md 4.3, 4.4 va 10. Set co dung cai hinh 80 byte y het map, cot
// value van nam do nhung khong dung bao gio, nen mot ban cai dat gach ca hai
// va crate::set chi la mot lop da mong phu len file nay.
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
// giu thu tu them vao.
//
// key_kind o +72 chon cach bam va cach so bang: so tung bit tren o, so theo
// noi dung UTF-8 neu la chuoi, hoac so theo dia chi object. Float khong bao
// gio duoc lam key.

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
// sang ngay sinh cua t viet nguoc lai cong voi bon chu PUMP trong ascii, cho
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

#[inline]
unsafe fn index_slot(index: *const u8, slot: usize) -> i64 {
    read_u64(index, index_slot_offset(slot)) as i64
}

#[inline]
unsafe fn set_index_slot(index: *mut u8, slot: usize, value: i64) {
    write_u64(index, index_slot_offset(slot), value as u64);
}

// ===== bam =====

fn mix(value: u64) -> u64 {
    let mut mixed = value ^ HASH_SEED;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 54)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 62)
}

unsafe fn hash_key(kind: u32, key: u64) -> u64 {
    let hash = match kind {
        KEY_KIND_STRING if key != 0 => pump_string_hash(key as *mut u8),
        _ => mix(key),
    };
    hash.max(1)
}

unsafe fn keys_equal(kind: u32, a: u64, b: u64) -> bool {
    match kind {
        KEY_KIND_STRING => pump_string_eq(a as *const u8, b as *const u8) != 0,
        _ => a == b,
    }
}

unsafe fn resolve_key_kind(table: *const u8, key: u64) -> u32 {
    if slot_flags(table) & SLOT_FLAG_KEY_IS_REF == 0 {
        return KEY_KIND_SCALAR;
    }
    if key != 0 && type_id_of(key as *const u8) == TYPE_ID_STRING {
        return KEY_KIND_STRING;
    }
    match read_u32(table, KEY_KIND_OFFSET) {
        KEY_KIND_STRING => KEY_KIND_STRING,
        _ => KEY_KIND_REFERENCE,
    }
}

// ===== do tim =====

struct Probe {
    entry: Option<u64>,
    slot: usize,
}

unsafe fn probe(table: *const u8, kind: u32, hash: u64, key: u64) -> Probe {
    let index = read_ptr(table, INDEX_OFFSET);
    let entries = read_ptr(table, ENTRIES_OFFSET);
    let mask = (index_capacity(table) - 1) as usize;

    let mut slot = (hash as usize) & mask;
    let mut vacancy = None;
    loop {
        match index_slot(index, slot) {
            INDEX_EMPTY => {
                return Probe {
                    entry: None,
                    slot: vacancy.unwrap_or(slot),
                }
            }
            INDEX_TOMBSTONE => vacancy = vacancy.or(Some(slot)),
            position => {
                let position = position as u64;
                if entry_field(entries, position, ENTRY_HASH_OFFSET) == hash
                    && keys_equal(kind, entry_field(entries, position, ENTRY_KEY_OFFSET), key)
                {
                    return Probe {
                        entry: Some(position),
                        slot,
                    };
                }
            }
        }
        slot = (slot + 1) & mask;
    }
}

pub(crate) unsafe fn lookup(table: *const u8, key: u64) -> Option<u64> {
    if length(table) == 0 || read_ptr(table, INDEX_OFFSET).is_null() {
        return None;
    }
    let kind = resolve_key_kind(table, key);
    probe(table, kind, hash_key(kind, key), key).entry
}

// ===== no ra =====

fn do_rehash(table: *mut u8, wanted: u64) {
    unsafe {
        let entry_capacity = wanted.next_power_of_two().max(MIN_ENTRY_CAPACITY);
        let index_capacity = entry_capacity * 2;
        let Some(entry_bytes) = entry_capacity.checked_mul(ENTRY_SIZE as u64) else {
            abort_runtime("a map grew past the addressable range");
        };

        let scope = RootScope::new();
        scope.keep(table);
        let new_entries = scope.keep(pump_alloc_buffer(entry_bytes));
        let new_index = scope.keep(pump_alloc_buffer(index_capacity * INDEX_SLOT_SIZE as u64));

        // cai nay o index trong
        // vua xoa trang phai dien lai truoc khi do tim duoc.
        std::ptr::write_bytes(
            new_index.add(HEADER_SIZE),
            0xff,
            index_capacity as usize * INDEX_SLOT_SIZE,
        );

        // buffer co the lam tron len thanh nhieu o entry hon minh xin, nhung
        // index la mot luy thua cua 2 co dinh, an luon cho thua se pha cai he
        // so tai ma vong do tim dua vao, nen de phan thua nam khong.
        let old_entries = read_ptr(table, ENTRIES_OFFSET);
        let old_used = entry_used(table);
        let mask = (index_capacity - 1) as usize;
        let mut live = 0u64;

        if !old_entries.is_null() {
            for position in 0..old_used {
                let hash = entry_field(old_entries, position, ENTRY_HASH_OFFSET);
                if hash == 0 {
                    continue;
                }
                let key = entry_field(old_entries, position, ENTRY_KEY_OFFSET);
                let value = entry_field(old_entries, position, ENTRY_VALUE_OFFSET);
                set_entry_field(new_entries, live, ENTRY_HASH_OFFSET, hash);
                set_entry_field(new_entries, live, ENTRY_KEY_OFFSET, key);
                set_entry_field(new_entries, live, ENTRY_VALUE_OFFSET, value);

                let mut slot = (hash as usize) & mask;
                while index_slot(new_index, slot) != INDEX_EMPTY {
                    slot = (slot + 1) & mask;
                }
                set_index_slot(new_index, slot, live as i64);
                live += 1;
            }
        }

        write_ptr(table, ENTRIES_OFFSET, new_entries);
        set_entry_capacity(table, entry_capacity);
        set_entry_used(table, live);
        write_ptr(table, INDEX_OFFSET, new_index);
        set_index_capacity(table, index_capacity);
    }
}

unsafe fn reserve_one(table: *mut u8) {
    if entry_used(table) < entry_capacity(table) {
        return;
    }
    do_rehash(table, (length(table) + 1) * 2);
}

// ===== cai bang dung chung =====

pub(crate) fn new_table(type_id: u32, is_set: bool) -> *mut u8 {
    let table = pump_alloc(type_id, SIZE);

    let mut flags = 0;
    if let Some(descriptor) = descriptor(type_id) {
        let key_is_ref = if is_set {
            descriptor.flags & (DESC_FLAG_ELEM_IS_REF | DESC_FLAG_KEY_IS_REF) != 0
        } else {
            descriptor.flags & DESC_FLAG_KEY_IS_REF != 0
        };
        if key_is_ref {
            flags |= SLOT_FLAG_KEY_IS_REF;
        }
        if !is_set && descriptor.flags & DESC_FLAG_VALUE_IS_REF != 0 {
            flags |= SLOT_FLAG_VALUE_IS_REF;
        }
    }

    let kind = if flags & SLOT_FLAG_KEY_IS_REF != 0 {
        KEY_KIND_REFERENCE
    } else {
        KEY_KIND_SCALAR
    };

    unsafe {
        write_u32(table, SLOT_FLAGS_OFFSET, flags);
        write_u32(table, KEY_KIND_OFFSET, kind);
    }
    table
}

pub(crate) fn insert(table: *mut u8, key: u64, value: u64) -> bool {
    unsafe {
        let scope = RootScope::new();
        scope.keep(table);
        let flags = slot_flags(table);
        scope.keep_slot(key, flags & SLOT_FLAG_KEY_IS_REF != 0);
        scope.keep_slot(value, flags & SLOT_FLAG_VALUE_IS_REF != 0);

        let kind = resolve_key_kind(table, key);
        write_u32(table, KEY_KIND_OFFSET, kind);
        let hash = hash_key(kind, key);

        reserve_one(table);
        let found = probe(table, kind, hash, key);
        let entries = read_ptr(table, ENTRIES_OFFSET);

        if let Some(position) = found.entry {
            set_entry_field(entries, position, ENTRY_VALUE_OFFSET, value);
            return false;
        }

        let position = entry_used(table);
        set_entry_field(entries, position, ENTRY_HASH_OFFSET, hash);
        set_entry_field(entries, position, ENTRY_KEY_OFFSET, key);
        set_entry_field(entries, position, ENTRY_VALUE_OFFSET, value);
        set_index_slot(read_ptr(table, INDEX_OFFSET), found.slot, position as i64);
        set_entry_used(table, position + 1);
        set_length(table, length(table) + 1);
        bump_modcount(table);
        true
    }
}

// xoa den khi con duoi mot phan tu bang so o thi bang tu co lai, nen mot cai
// map to roi xoa gan het khong giu mai cho trong.
pub(crate) fn remove(table: *mut u8, key: u64) -> bool {
    unsafe {
        let kind = resolve_key_kind(table, key);
        if length(table) == 0 || read_ptr(table, INDEX_OFFSET).is_null() {
            return false;
        }
        let found = probe(table, kind, hash_key(kind, key), key);
        let Some(position) = found.entry else {
            return false;
        };

        let entries = read_ptr(table, ENTRIES_OFFSET);
        set_entry_field(entries, position, ENTRY_HASH_OFFSET, 0);
        set_entry_field(entries, position, ENTRY_KEY_OFFSET, 0);
        set_entry_field(entries, position, ENTRY_VALUE_OFFSET, 0);
        set_index_slot(read_ptr(table, INDEX_OFFSET), found.slot, INDEX_TOMBSTONE);
        set_length(table, length(table) - 1);
        bump_modcount(table);
        true
    }
}

pub(crate) unsafe fn iter_next(
    table: *const u8,
    cursor: *mut u64,
    out_key: *mut u64,
    out_value: *mut u64,
) -> bool {
    if cursor.is_null() {
        return false;
    }
    let entries = read_ptr(table, ENTRIES_OFFSET);
    if entries.is_null() {
        return false;
    }

    let used = entry_used(table);
    let mut position = *cursor;
    while position < used {
        if entry_field(entries, position, ENTRY_HASH_OFFSET) != 0 {
            if !out_key.is_null() {
                out_key.write(entry_field(entries, position, ENTRY_KEY_OFFSET));
            }
            if !out_value.is_null() {
                out_value.write(entry_field(entries, position, ENTRY_VALUE_OFFSET));
            }
            cursor.write(position + 1);
            return true;
        }
        position += 1;
    }
    cursor.write(used);
    false
}

fn collect_column(table: *const u8, field: usize, slot_is_ref: bool) -> *mut u8 {
    let type_id = if slot_is_ref {
        TYPE_ID_ARRAY_REF
    } else {
        TYPE_ID_ARRAY_SCALAR
    };

    let scope = RootScope::new();
    scope.keep(table as *mut u8);
    let array = scope.keep(empty_array(type_id));

    unsafe {
        let used = entry_used(table);
        for position in 0..used {
            let entries = read_ptr(table, ENTRIES_OFFSET);
            if entry_field(entries, position, ENTRY_HASH_OFFSET) == 0 {
                continue;
            }
            pump_array_push(array, entry_field(entries, position, field));
        }
    }
    array
}

// ===== map entry points =====

/// A new empty map.
#[no_mangle]
pub extern "C" fn pump_map_new(type_id: u32) -> *mut u8 {
    new_table(type_id, false)
}

/// The number of live entries.
#[no_mangle]
pub extern "C" fn pump_map_len(m: *const u8) -> u64 {
    unsafe { length(m) }
}

/// Writes the value for `key` through `out_value` and returns 1, or returns 0
/// and leaves `out_value` untouched.
#[no_mangle]
pub extern "C" fn pump_map_lookup(m: *const u8, key: u64, out_value: *mut u64) -> i8 {
    unsafe {
        let Some(position) = lookup(m, key) else {
            return 0;
        };
        if !out_value.is_null() {
            let entries = read_ptr(m, ENTRIES_OFFSET);
            out_value.write(entry_field(entries, position, ENTRY_VALUE_OFFSET));
        }
        1
    }
}

/// The value for `key`.
#[no_mangle]
pub extern "C" fn pump_map_get(m: *const u8, key: u64) -> u64 {
    unsafe {
        match lookup(m, key) {
            Some(position) => {
                entry_field(read_ptr(m, ENTRIES_OFFSET), position, ENTRY_VALUE_OFFSET)
            }
            None => pump_panic_missing_key(),
        }
    }
}

/// Inserts or overwrites.
#[no_mangle]
pub extern "C" fn pump_map_set(m: *mut u8, key: u64, value: u64) {
    insert(m, key, value);
}

/// Removes `key`, tombstoning its entry.
#[no_mangle]
pub extern "C" fn pump_map_remove(m: *mut u8, key: u64) -> i8 {
    i8::from(remove(m, key))
}

/// Whether `key` is present.
#[no_mangle]
pub extern "C" fn pump_map_has(m: *const u8, key: u64) -> i8 {
    i8::from(unsafe { lookup(m, key) }.is_some())
}

/// The keys as an array, in insertion order.
#[no_mangle]
pub extern "C" fn pump_map_keys(m: *const u8) -> *mut u8 {
    let key_is_ref = unsafe { slot_flags(m) } & SLOT_FLAG_KEY_IS_REF != 0;
    collect_column(m, ENTRY_KEY_OFFSET, key_is_ref)
}

/// The values as an array, in insertion order.
#[no_mangle]
pub extern "C" fn pump_map_values(m: *const u8) -> *mut u8 {
    let value_is_ref = unsafe { slot_flags(m) } & SLOT_FLAG_VALUE_IS_REF != 0;
    collect_column(m, ENTRY_VALUE_OFFSET, value_is_ref)
}

/// Advances an iteration.
#[no_mangle]
pub extern "C" fn pump_map_iter_next( m: *const u8, cursor: *mut u64, out_key: *mut u64, out_value: *mut u64, ) -> i8 {
    i8::from(unsafe { iter_next(m, cursor, out_key, out_value) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::array::pump_array_get;
    use crate::string::pump_string_new;
    use crate::testing;

    fn string(text: &str) -> u64 {
        pump_string_new(text.as_ptr(), text.len() as u64) as u64
    }

    fn keys_in_order(map: *const u8) -> Vec<u64> {
        let mut cursor = 0u64;
        let mut key = 0u64;
        let mut value = 0u64;
        let mut collected = Vec::new();
        while pump_map_iter_next(map, &mut cursor, &mut key, &mut value) != 0 {
            collected.push(key);
        }
        collected
    }

    #[test]
    fn the_layout_constants_match_the_document() {
        assert_eq!(LENGTH_OFFSET, 16);
        assert_eq!(ENTRIES_OFFSET, 24);
        assert_eq!(ENTRY_CAPACITY_OFFSET, 32);
        assert_eq!(ENTRY_USED_OFFSET, 40);
        assert_eq!(INDEX_OFFSET, 48);
        assert_eq!(INDEX_CAPACITY_OFFSET, 56);
        assert_eq!(MODCOUNT_OFFSET, 64);
        assert_eq!(KEY_KIND_OFFSET, 72);
        assert_eq!(SLOT_FLAGS_OFFSET, 76);
        assert_eq!(SIZE, 80);
        assert_eq!(entry_offset(0), 16);
        assert_eq!(entry_offset(2), 64);
        assert_eq!(index_slot_offset(3), 40);
    }

    #[test]
    fn an_empty_map_allocates_no_buffers() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let map = pump_map_new(types.scalar_map);
        assert_eq!(pump_map_len(map), 0);
        assert_eq!(pump_map_has(map, 1), 0);
        unsafe {
            assert!(read_ptr(map, ENTRIES_OFFSET).is_null());
            assert!(read_ptr(map, INDEX_OFFSET).is_null());
        }
    }

    #[test]
    fn scalar_keys_round_trip_and_overwrite() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let map = pump_map_new(types.scalar_map);
        for key in 0..500u64 {
            pump_map_set(map, key, key * 3);
        }
        assert_eq!(pump_map_len(map), 500);
        for key in 0..500u64 {
            assert_eq!(pump_map_get(map, key), key * 3);
        }

        pump_map_set(map, 7, 999);
        assert_eq!(pump_map_len(map), 500);
        assert_eq!(pump_map_get(map, 7), 999);

        let mut value = 0u64;
        assert_eq!(pump_map_lookup(map, 12, &mut value), 1);
        assert_eq!(value, 36);
        assert_eq!(pump_map_lookup(map, 5000, &mut value), 0);
        assert_eq!(value, 36);
    }

    #[test]
    fn string_keys_compare_by_content_not_identity() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let map = pump_map_new(types.string_map);
        pump_map_set(map, string("alpha"), 1);
        pump_map_set(map, string("beta"), 2);

        assert_eq!(pump_map_len(map), 2);
        assert_eq!(pump_map_get(map, string("alpha")), 1);
        assert_eq!(pump_map_has(map, string("beta")), 1);
        assert_eq!(pump_map_has(map, string("gamma")), 0);

        // A distinct object with the same text overwrites rather than adding.
        pump_map_set(map, string("alpha"), 11);
        assert_eq!(pump_map_len(map), 2);
        assert_eq!(pump_map_get(map, string("alpha")), 11);
        unsafe { assert_eq!(read_u32(map, KEY_KIND_OFFSET), KEY_KIND_STRING) };
    }

    #[test]
    fn iteration_is_insertion_order_even_after_removal() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let map = pump_map_new(types.scalar_map);
        for key in [50u64, 10, 90, 30, 70] {
            pump_map_set(map, key, key);
        }
        assert_eq!(keys_in_order(map), vec![50, 10, 90, 30, 70]);

        assert_eq!(pump_map_remove(map, 90), 1);
        assert_eq!(pump_map_remove(map, 90), 0);
        assert_eq!(keys_in_order(map), vec![50, 10, 30, 70]);

        pump_map_set(map, 90, 90);
        assert_eq!(keys_in_order(map), vec![50, 10, 30, 70, 90]);

        let keys = pump_map_keys(map);
        let values = pump_map_values(map);
        for (position, expected) in [50u64, 10, 30, 70, 90].iter().enumerate() {
            assert_eq!(pump_array_get(keys, position as i64), *expected);
            assert_eq!(pump_array_get(values, position as i64), *expected);
        }
    }

    #[test]
    fn churn_reclaims_tombstones_instead_of_growing_forever() {
        let _guard = testing::guard();
        let types = testing::install_type_table();

        let map = pump_map_new(types.scalar_map);
        for round in 0..10_000u64 {
            pump_map_set(map, round, round);
            pump_map_remove(map, round);
        }

        assert_eq!(pump_map_len(map), 0);
        unsafe {
            assert!( entry_capacity(map) <= 64, "a map that never held more than one entry grew to {} slots", entry_capacity(map) );
        }
    }

    #[test]
    fn a_map_keeps_its_keys_and_values_alive() {
        let _guard = testing::guard();
        let types = testing::install_type_table();
        let mut bottom = 0usize;
        testing::init_runtime(&mut bottom);

        let scope = RootScope::new();
        let map = scope.keep(pump_map_new(types.string_map));
        for index in 0..200u64 {
            pump_map_set(map, string(&format!("key {index}")), index);
        }

        crate::gc::pump_gc_collect();

        assert_eq!(pump_map_len(map), 200);
        for index in 0..200u64 {
            assert_eq!(pump_map_get(map, string(&format!("key {index}"))), index);
        }
        drop(scope);
    }
}
