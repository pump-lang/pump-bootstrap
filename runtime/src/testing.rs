// may ham phu ...
//
// cai nay khong thuoc abi,

use std::ptr;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::{DescriptorKind, ObjectHeader, TypeDescriptor, FIRST_USER_TYPE_ID, FLAG_IMMORTAL};

// xep hang de vao runtime, khong cho hai test vao cung luc

static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

pub(crate) fn guard() -> Guard {
    let lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::gc::reset();
    Guard(lock)
}

pub(crate) fn init_runtime(bottom: &mut usize) {
    let table = TYPE_TABLE.get().copied().unwrap_or(0) as *const TypeDescriptor;
    let count = if table.is_null() {
        0
    } else {
        TYPE_COUNT as u64
    };
    crate::start::pump_rt_init(
        bottom as *mut usize as *const u8,
        table,
        count,
        ptr::null_mut(),
        0,
        0,
        ptr::null(),
    );
}

// mot bang type bia ra

#[derive(Clone, Copy, Debug)]
pub(crate) struct TestTypes {
    pub(crate) node: u32,
    pub(crate) closure: u32,
    pub(crate) scalar_array: u32,
    pub(crate) ref_array: u32,
    pub(crate) scalar_map: u32,
    pub(crate) string_map: u32,
    pub(crate) scalar_set: u32,
    pub(crate) string_set: u32,
}

const TEST_TYPES: TestTypes = TestTypes {
    node: FIRST_USER_TYPE_ID,
    closure: FIRST_USER_TYPE_ID + 1,
    scalar_array: FIRST_USER_TYPE_ID + 2,
    ref_array: FIRST_USER_TYPE_ID + 3,
    scalar_map: FIRST_USER_TYPE_ID + 4,
    string_map: FIRST_USER_TYPE_ID + 5,
    scalar_set: FIRST_USER_TYPE_ID + 6,
    string_set: FIRST_USER_TYPE_ID + 7,
};

const TYPE_COUNT: usize = FIRST_USER_TYPE_ID as usize + 8;

static NODE_REF_OFFSETS: [u32; 1] = [16];

static TYPE_TABLE: OnceLock<usize> = OnceLock::new();

const fn inert() -> TypeDescriptor {
    TypeDescriptor {
        kind: DescriptorKind::Buffer as u32,
        flags: 0,
        size: 0,
        ref_count: 0,
        variant_count: 0,
        ref_offsets: ptr::null(),
        variants: ptr::null(),
        name: ptr::null(),
    }
}

fn describe(
    kind: DescriptorKind,
    flags: u32,
    size: u64,
    ref_offsets: &'static [u32],
    name: &'static [u8],
) -> TypeDescriptor {
    TypeDescriptor {
        kind: kind as u32,
        flags,
        size,
        ref_count: ref_offsets.len() as u32,
        variant_count: 0,
        ref_offsets: ref_offsets.as_ptr(),
        variants: ptr::null(),
        name: name.as_ptr(),
    }
}

fn build_type_table() -> usize {
    use crate::{DESC_FLAG_ELEM_IS_REF, DESC_FLAG_KEY_IS_REF};

    let mut entries = vec![inert(); FIRST_USER_TYPE_ID as usize];
    entries.push(describe(
        DescriptorKind::Struct,
        0,
        32,
        &NODE_REF_OFFSETS,
        b"Node\0",
    ));
    entries.push(describe(DescriptorKind::Closure, 0, 0, &[], b"closure\0"));
    entries.push(describe(DescriptorKind::Array, 0, 48, &[], b"[int]\0"));
    entries.push(describe(
        DescriptorKind::Array,
        DESC_FLAG_ELEM_IS_REF,
        48,
        &[],
        b"[Node]\0",
    ));
    entries.push(describe(DescriptorKind::Map, 0, 80, &[], b"[int: int]\0"));
    entries.push(describe(
        DescriptorKind::Map,
        DESC_FLAG_KEY_IS_REF,
        80,
        &[],
        b"[string: int]\0",
    ));
    entries.push(describe(DescriptorKind::Set, 0, 80, &[], b"set<int>\0"));
    entries.push(describe(
        DescriptorKind::Set,
        DESC_FLAG_ELEM_IS_REF,
        80,
        &[],
        b"set<string>\0",
    ));

    assert_eq!(entries.len(), TYPE_COUNT);
    Vec::leak(entries).as_ptr() as usize
}

pub(crate) fn install_type_table() -> TestTypes {
    let address = *TYPE_TABLE.get_or_init(build_type_table);
    let collector = crate::gc::collector();
    collector.type_table = address as *const TypeDescriptor;
    collector.type_count = TYPE_COUNT as u64;
    TEST_TYPES
}

// mot object tinh

#[repr(C, align(16))]
pub(crate) struct StaticNode {
    header: ObjectHeader,
    next: *mut u8,
    padding: u64,
}

impl StaticNode {
    pub(crate) fn new(type_id: u32) -> StaticNode {
        StaticNode {
            header: ObjectHeader {
                type_id,
                flags: FLAG_IMMORTAL,
                size: std::mem::size_of::<StaticNode>() as u64,
            },
            next: ptr::null_mut(),
            padding: 0,
        }
    }

    pub(crate) fn as_ptr(&mut self) -> *mut u8 {
        self as *mut StaticNode as *mut u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_static_node_has_a_nodes_shape() {
        assert_eq!(std::mem::size_of::<StaticNode>(), 32);
        assert_eq!(std::mem::align_of::<StaticNode>(), 16);

        let mut node = StaticNode::new(TEST_TYPES.node);
        let pointer = node.as_ptr();
        unsafe {
            assert_eq!(crate::type_id_of(pointer), TEST_TYPES.node);
            assert!(crate::header(pointer).is_immortal());
            assert!(crate::read_ptr(pointer, 16).is_null());
        }
    }

    #[test]
    fn the_reserved_ids_are_present_and_inert() {
        let _guard = guard();
        install_type_table();

        // GC tu biet may id co san va khong bao gio tra bang o duoi
        // FIRST_USER_TYPE_ID, chinh vi the ma may o do de nam khong cung duoc.
        for type_id in 0..FIRST_USER_TYPE_ID {
            assert!(crate::gc::descriptor(type_id).is_none());
        }
        assert!(crate::gc::descriptor(TEST_TYPES.node).is_some());
        assert!(crate::gc::descriptor(TYPE_COUNT as u32).is_none());
    }
}
