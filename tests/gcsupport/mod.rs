// gian giao cho may test GC o muc runtime.
//
// Hai dieu chi phoi moi thu o duoi:
//
//  * runtime la mot dong bien toan cuc va khong khoa gi ca. Harness cua cargo
//    chay test tren nhieu luong, nen with_runtime giu mot cai khoa ca tien
//    trinh, va than cua moi test chay ben trong no.
//  * GC quet stack kieu bao thu tu con tro stack hien tai den cai
//    stack_bottom no duoc dua. Cai bottom do bat buoc phai la bien cuc bo cua
//    mot frame nam TREN moi thu ma test lam, va do la ly do with_runtime nhan
//    mot closure chu khong tra ve mot guard.

#![allow(dead_code)]

use std::ptr;
use std::sync::{Mutex, OnceLock};

use pump_runtime::alloc::pump_alloc;
use pump_runtime::gc::{pump_gc_add_root, pump_gc_remove_root};
use pump_runtime::start::{pump_rt_init, pump_rt_shutdown};
use pump_runtime::{DescriptorKind, TypeDescriptor, FIRST_USER_TYPE_ID};

// ===== hai hinh object =====

/// A `Node`: two pointer fields, then two scalars.
pub const NODE: u32 = FIRST_USER_TYPE_ID;
/// Total size of a `Node`, header included.
pub const NODE_SIZE: u64 = 48;
/// A `Node`'s first pointer field.
pub const NEXT: usize = 16;
/// A `Node`'s second pointer field.
pub const CHILD: usize = 24;
/// A `Node`'s payload word, set to whatever a test wants to read back.
pub const VALUE: usize = 32;
/// A `Node`'s witness word, set to `magic` and checked after every
/// collection.
pub const MAGIC: usize = 40;

/// A `Leaf`: no pointer fields at all, which is what pure garbage is made of.
pub const LEAF: u32 = FIRST_USER_TYPE_ID + 1;
/// Total size of a `Leaf`, header included.
pub const LEAF_SIZE: u64 = 32;

/// The witness word a node built for `index` must still carry.
pub const fn magic(index: u64) -> u64 {
    0x00c0_ffee_0000_0000 | index
}

// ===== bang type bia ra =====

static NODE_REF_OFFSETS: [u32; 2] = [NEXT as u32, CHILD as u32];

const TYPE_COUNT: usize = FIRST_USER_TYPE_ID as usize + 2;

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

fn structure(size: u64, ref_offsets: &'static [u32], name: &'static [u8]) -> TypeDescriptor {
    TypeDescriptor {
        kind: DescriptorKind::Struct as u32,
        flags: 0,
        size,
        ref_count: ref_offsets.len() as u32,
        variant_count: 0,
        ref_offsets: ref_offsets.as_ptr(),
        variants: ptr::null(),
        name: name.as_ptr(),
    }
}

fn build_type_table() -> usize {
    let mut entries = vec![inert(); FIRST_USER_TYPE_ID as usize];
    entries.push(structure(NODE_SIZE, &NODE_REF_OFFSETS, b"Node\0"));
    entries.push(structure(LEAF_SIZE, &[], b"Leaf\0"));
    assert_eq!(entries.len(), TYPE_COUNT);
    Vec::leak(entries).as_ptr() as usize
}

// ===== di vao runtime =====

static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

/// Runs `body` with the process runtime to itself: an empty heap, the
/// synthetic type table installed, and a stack bottom above every frame
/// `body` will occupy.
#[inline(never)]
pub fn with_runtime(body: impl FnOnce()) {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // This local is the far end of the conservatively scanned range. `body`
    // runs in a deeper frame, so everything it holds lies inside that range.
    let mut bottom = 0usize;
    let table = *TYPE_TABLE.get_or_init(build_type_table);
    pump_rt_init(
        &mut bottom as *mut usize as *const u8,
        table as *const TypeDescriptor,
        TYPE_COUNT as u64,
        ptr::null_mut(),
        0,
        0,
        ptr::null(),
    );

    let outcome = run_body(body);
    release_all_roots();
    pump_rt_shutdown(0);

    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

#[inline(never)]
fn run_body(body: impl FnOnce()) -> std::thread::Result<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
}

// ===== object =====

/// Allocates a `Node` carrying `index` in both its scalar words.
pub fn node(index: u64) -> *mut u8 {
    let object = pump_alloc(NODE, NODE_SIZE);
    set_word(object, VALUE, index);
    set_word(object, MAGIC, magic(index));
    object
}

/// Allocates a `Node` of `size` bytes rather than the usual `NODE_SIZE`.
pub fn node_of_size(index: u64, size: u64) -> *mut u8 {
    assert!(size >= NODE_SIZE);
    let object = pump_alloc(NODE, size);
    set_word(object, VALUE, index);
    set_word(object, MAGIC, magic(index));
    object
}

/// Allocates a `Leaf`, the shape used for pure garbage.
pub fn leaf(index: u64) -> *mut u8 {
    let object = pump_alloc(LEAF, LEAF_SIZE);
    set_word(object, 16, index);
    object
}

/// Reads the word at `object + offset`.
pub fn word(object: *const u8, offset: usize) -> u64 {
    // SAFETY: the caller passes a live object and an offset inside it.
    unsafe { (object.add(offset) as *const u64).read() }
}

/// Writes the word at `object + offset`.
pub fn set_word(object: *mut u8, offset: usize, value: u64) {
    // SAFETY: the caller passes a live object and an offset inside it.
    unsafe { (object.add(offset) as *mut u64).write(value) }
}

/// Reads the pointer slot at `object + offset`.
pub fn slot(object: *const u8, offset: usize) -> *mut u8 {
    word(object, offset) as *mut u8
}
