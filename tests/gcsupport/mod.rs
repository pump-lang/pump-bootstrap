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

/// Writes the pointer slot at `object + offset`.
pub fn set_slot(object: *mut u8, offset: usize, value: *mut u8) {
    set_word(object, offset, value as u64);
}

/// The `type_id` in an object's header.
pub fn type_id(object: *const u8) -> u32 {
    // SAFETY: the caller passes a live object, so its header is readable.
    unsafe { (object as *const u32).read() }
}

/// The `flags` word in an object's header.
pub fn flags(object: *const u8) -> u32 {
    // SAFETY: the caller passes a live object, so its header is readable.
    unsafe { (object.add(4) as *const u32).read() }
}

/// Asserts that `object` is still the node built for `index`, with both
/// scalar words intact.
pub fn assert_intact(object: *const u8, index: u64, context: &str) {
    let found_type = type_id(object);
    let found_value = word(object, VALUE);
    let found_magic = word(object, MAGIC);
    assert!(
        found_type == NODE && found_value == index && found_magic == magic(index),
        "{context}: node {index} at {object:p} was corrupted - type_id {found_type} \
         (want {NODE}), value {found_value} (want {index}), witness {found_magic:#x} \
         (want {:#x})",
        magic(index)
    );
}

// ===== goc =====

static REGISTERED_ROOTS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// A fixed table of pointer slots registered with the collector, which is how
/// a test holds a reference without relying on the conservative stack scan to
/// find it.
pub struct Roots {
    base: *mut *mut u8,
    count: usize,
}

impl Roots {
    pub fn new(count: usize) -> Roots {
        let slots: Vec<*mut u8> = vec![ptr::null_mut(); count];
        let base = Box::leak(slots.into_boxed_slice()).as_mut_ptr();
        for index in 0..count {
            // SAFETY: the leaked slice has `count` slots and outlives the
            // registration.
            pump_gc_add_root(unsafe { base.add(index) });
        }
        REGISTERED_ROOTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((base as usize, count));
        Roots { base, count }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, index: usize) -> *mut u8 {
        assert!(index < self.count);
        // SAFETY: `index` is in range and the slots are live.
        unsafe { *self.base.add(index) }
    }

    pub fn set(&mut self, index: usize, object: *mut u8) {
        assert!(index < self.count);
        // SAFETY: `index` is in range and the slots are live.
        unsafe { *self.base.add(index) = object };
    }

    /// Drops every reference the table holds, leaving it registered.
    pub fn clear(&mut self) {
        for index in 0..self.count {
            self.set(index, ptr::null_mut());
        }
    }
}

fn release_all_roots() {
    let mut registered = REGISTERED_ROOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (base, count) in registered.drain(..) {
        let base = base as *mut *mut u8;
        for index in 0..count {
            // SAFETY: the storage is leaked, so these slots are still live.
            unsafe { *base.add(index) = ptr::null_mut() };
            pump_gc_remove_root(unsafe { base.add(index) });
        }
    }
}

// ===== don stack cho sach =====

/// Overwrites the stack the helpers above just used.
#[inline(never)]
pub fn clobber_stack() {
    let mut scratch = [0usize; 4096];
    for (index, word) in scratch.iter_mut().enumerate() {
        *word = index;
    }
    std::hint::black_box(&scratch);
}

/// A deterministic xorshift generator, so a randomised test that fails fails
/// the same way twice.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `0..bound`.
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}
