// BO GOM RAC. Mark-sweep bao thu, dung ca the gioi lai, mot luong.
//
// Doc ky truoc khi sua bat cu dong nao trong file nay. Sai o day khong bao
// loi, khong panic, khong gi het: no chi lang le tra ve mot con tro toi mot
// object da chet, roi ba muoi giay sau chuong trinh do o mot cho hoan toan
// khac. T mat ...
// trong repo ma t viet chu thich nhieu hon code.
//
// docs/abi.md muc 9 la ban dac ta. Cho lech nhau quan trong nhat:
//
// cai nay  * stack
//    Cranelift chua du chin de tin, nen thay vao do minh soi tung tu 8 byte
//    can le nam giua con tro stack hien tai va cai `stack_bottom` ma
//    crate::start::pump_rt_init ghi lai. Tu nao tro vao mot trang heap dang
//    song VA roi dung ngay dau mot object thi coi la mot goc. Con tro noi bo
//    thi co y KHONG tinh; code sinh ra bi bat buoc khong duoc giu con tro noi
//    bo vat qua mot cho cap phat.
//  * HEAP thi di CHINH XAC, theo type descriptor cua docs/abi.md muc 8. Nho
//    the ma phan giu nham chi gioi han o dung cai ma mot tu tren stack tinh
//    co giong, chu khong lan ra ca do thi object.
//
// Truoc khi quet, pump_gc_collect do het thanh ghi callee-saved vao mot vung
// dem nam ngay tren stack, o trong khoang se quet. Cong voi quy uoc goi cua
// nen tang thi the la du: o bat ky cho goi nao, mot con tro dang song hoac
// dang nam trong thanh ghi callee-saved (nen se bi do ra), hoac da nam san
// tren stack roi.
//
// Goc, ke het: ...
// pump_error_slot, moi thu dang ky qua pump_gc_add_root, va may bien tam
// RootScope cua chinh runtime.
//
// -- CAI GIA CUA VIEC BAO THU --
//
// GC bao thu khong the phan biet mot con tro voi mot so nguyen tinh co trong
// giong con tro. Mot tu cu con sot lai tren stack ma trong giong dia chi cua
// mot object da chet se giu object do, VA MOI THU NO TRO TOI, song them it
// nhat mot chu ky nua. Phan giu nham co gioi han - heap duoc di chinh xac,
// nen mot goc gia chi giu lai mot nhanh chu khong nhoe ra ca heap - nhung no
// cai nay co that. chuong
// qua mot frame da chet o rat sau thi co the thay no song qua may lan gom.
//
// Duong con lai la GC chinh xac, tuc
// safe point. Cranelift lam duoc, nhung phan do con non, va mot cai stack map
// sai o dung mot ham la use-after-free ma khong co lay mot dong bao loi. Giu
// nham co gioi ...
//
// Xem them TODO.txt, dong "GC con bao thu".

use crate::alloc::heap;
use crate::{
  read_ptr, read_u32, read_u64, DescriptorKind, Global, TypeDescriptor, DESC_FLAG_ELEM_IS_REF,
  DESC_FLAG_KEY_IS_REF, DESC_FLAG_VALUE_IS_REF, FIRST_USER_TYPE_ID, FLAG_MARK, HEADER_SIZE,
  TYPE_ID_ARRAY_REF, TYPE_ID_ARRAY_SCALAR, TYPE_ID_BOX_REF, TYPE_ID_INTERFACE,
};

const SPILL_SLOTS: usize = 8;

// trang thai cua bo gom rac

pub(crate) struct Collector {
  pub(crate) stack_bottom: usize,
  pub(crate) type_table: *const TypeDescriptor,
  pub(crate) type_count: u64,
  pub(crate) global_roots: *mut *mut u8,
  pub(crate) root_count: u64,
  extra_roots: Vec<*mut *mut u8>,
  temp_roots: Vec<*mut u8>,
  work: Vec<*mut u8>,
  marked_immortals: Vec<*mut u8>,
  collections: u64,
}

impl Collector {
  const fn new() -> Collector {
    Collector {
      stack_bottom: 0,
      type_table: std::ptr::null(),
      type_count: 0,
      global_roots: std::ptr::null_mut(),
      root_count: 0,
      extra_roots: Vec::new(),
      temp_roots: Vec::new(),
      work: Vec::new(),
      marked_immortals: Vec::new(),
      collections: 0,
    }
  }
}

static COLLECTOR: Global<Collector> = Global::new(Collector::new());

pub(crate) fn collector() -> &'static mut Collector {
  COLLECTOR.get()
}

pub(crate) fn can_collect() -> bool {
  collector().stack_bottom != 0 && heap().disable_depth == 0 && !heap().collecting
}

// tra descriptor theo type_id. Tra ve None neu id nam ngoai bang, va cho goi
// PHAI chiu duoc None: type_id hong thi
// day la doc lung tung trong bo nho.
pub(crate) fn descriptor(type_id: u32) -> Option<&'static TypeDescriptor> {
  let collector = collector();
  if type_id < FIRST_USER_TYPE_ID
    || collector.type_table.is_null()
    || u64::from(type_id) >= collector.type_count
  {
    return None;
  }
  // an toan: bang co dung `type_count` o va `type_id` vua duoc kiem o tren.
  Some(unsafe { &*collector.type_table.add(type_id as usize) })
}

// di het do thi object

/// How the collector walks the references going out of one object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TracePlan {
  Opaque,
  Fields {
    offsets: *const u32,
    count: u32,
  },
  Enum {
    variants: *const crate::VariantDescriptor,
    count: u32,
  },
  Array {
    elements_are_refs: bool,
  },
  Table {
    keys_are_refs: bool,
    values_are_refs: bool,
  },
  Closure,
  BoxedSlot,
  Interface,
}

/// Ke hoach di, cho mot object co type_id nay.
pub fn trace_plan(type_id: u32) -> TracePlan {
  match type_id {
    crate::TYPE_ID_INVALID
    | crate::TYPE_ID_BUFFER
    | crate::TYPE_ID_STRING
    | crate::TYPE_ID_BOX_SCALAR => return TracePlan::Opaque,
    TYPE_ID_BOX_REF => return TracePlan::BoxedSlot,
    TYPE_ID_INTERFACE => return TracePlan::Interface,
    TYPE_ID_ARRAY_SCALAR => {
      return TracePlan::Array {
        elements_are_refs: false,
      }
    }
    TYPE_ID_ARRAY_REF => {
      return TracePlan::Array {
        elements_are_refs: true,
      }
    }
    _ => {}
  }

  let Some(descriptor) = descriptor(type_id) else {
    // hoac la mot id giu san ma khong mang y nghia gi san, hoac la mot
    // id nam ngoai bang. Khong di gi ca la cau tra loi an toan duy nhat, va
    // sweep van thu lai duoc object khi khong con ai tro toi no.
    return TracePlan::Opaque;
  };
  let flags = descriptor.flags;
  match DescriptorKind::from_u32(descriptor.kind) {
    Some(DescriptorKind::Struct) | Some(DescriptorKind::Tuple) => TracePlan::Fields {
      offsets: descriptor.ref_offsets,
      count: descriptor.ref_count,
    },
    Some(DescriptorKind::Enum) => TracePlan::Enum {
      variants: descriptor.variants,
      count: descriptor.variant_count,
    },
    Some(DescriptorKind::Array) => TracePlan::Array {
      elements_are_refs: flags & DESC_FLAG_ELEM_IS_REF != 0,
    },
    Some(DescriptorKind::Map) => TracePlan::Table {
      keys_are_refs: flags & DESC_FLAG_KEY_IS_REF != 0,
      values_are_refs: flags & DESC_FLAG_VALUE_IS_REF != 0,
    },
    // set giu phan tu o cot key. Trong tai lieu muc 8 thi co cho o cua
    // set ten la ELEM_IS_REF, ma duong di lai qua luat cua map, tuc la doc
    // cai nay key_is_ref. nhan ca
    Some(DescriptorKind::Set) => TracePlan::Table {
      keys_are_refs: flags & (DESC_FLAG_ELEM_IS_REF | DESC_FLAG_KEY_IS_REF) != 0,
      values_are_refs: false,
    },
    Some(DescriptorKind::Closure) => TracePlan::Closure,
    Some(DescriptorKind::Box) => {
      if flags & DESC_FLAG_ELEM_IS_REF != 0 {
        TracePlan::BoxedSlot
      } else {
        TracePlan::Opaque
      }
    }
    Some(DescriptorKind::Interface) => TracePlan::Interface,
    Some(DescriptorKind::String) | Some(DescriptorKind::Buffer) | None => TracePlan::Opaque,
  }
}

// Danh dau mot object la con song, roi day no vao hang doi de di tiep.
//
// Vao bang null la binh thuong, khong phai loi: optional trong Pump chinh la
// con tro, nen null chay den day suot ngay.
fn mark(object: *mut u8) {
  if object.is_null() {
    return;
  }
  let header = unsafe { crate::header(object) };
  if header.is_marked() {
    return;
  }
  header.flags |= FLAG_MARK;
  let collector = collector();
  if header.is_immortal() {
    collector.marked_immortals.push(object);
  }
  collector.work.push(object);
}

// Mot tu tren stack. Hoi cai heap xem no co dung la dia chi bat dau cua mot
// object dang song khong. Khong phai thi bo, va bo im lang - day chinh la cho
// "bao thu" nam: minh khong biet tu do la con tro hay la mot so nguyen tinh
// co giong con tro, va khong co cach nao biet.
fn mark_conservatively(word: usize) {
  if let Some(object) = heap().object_at(word) {
    mark(object);
  }
}

// Di vao ruot mot object DA duoc danh dau, theo dung type descriptor cua no.
//
// Doan nay la doan phai khop tung byte voi src/abi.rs. Sai mot offset o day
// thi GC doc nham mot field khong phai con tro, coi no la dia chi, roi hoac
// la giu nham mot dong rac, hoac la di vao mot cho khong phai object. Truong
// hop sau la do ngay.
fn trace(object: *mut u8) {
  let type_id = unsafe { crate::type_id_of(object) };
  match trace_plan(type_id) {
    TracePlan::Opaque => {}
    TracePlan::Fields { offsets, count } => trace_fields(object, offsets, count),
    TracePlan::Enum { variants, count } => {
      let tag = unsafe { read_u32(object, crate::ENUM_TAG_OFFSET) };
      if tag < count && !variants.is_null() {
        let variant = unsafe { &*variants.add(tag as usize) };
        trace_fields(object, variant.ref_offsets, variant.ref_count);
      }
    }
    TracePlan::Array { elements_are_refs } => {
      // array: `length` o +16, `data` o +32. Buffer phan tu la mot
      // object rieng, nen phai mark ca no lan tung o ben trong.
      let (length, data) = unsafe {
        (
          read_u64(object, crate::array::LENGTH_OFFSET),
          read_ptr(object, crate::array::DATA_OFFSET),
        )
      };
      if data.is_null() {
        return;
      }
      mark(data);
      if elements_are_refs {
        for index in 0..length {
          let offset = HEADER_SIZE + index as usize * crate::SLOT_SIZE;
          mark(unsafe { read_ptr(data, offset) });
        }
      }
    }
    TracePlan::Table {
      keys_are_refs,
      values_are_refs,
    } => table(object, keys_are_refs, values_are_refs),
    TracePlan::Closure => {
      // closure: `capture_count` o +24, may o capture tu +32. Con tro
      // code o +16 KHONG phai tham chieu GC, co y bo qua no. Mark cai do
      // la dua mot dia chi code cho heap, chac chan do.
      let count = unsafe { read_u64(object, crate::iface::CAPTURE_COUNT_OFFSET) };
      for index in 0..count {
        let offset = crate::iface::CAPTURES_OFFSET + index as usize * crate::SLOT_SIZE;
        mark(unsafe { read_ptr(object, offset) });
      }
    }
    TracePlan::BoxedSlot => {
      mark(unsafe { read_ptr(object, crate::iface::BOX_VALUE_OFFSET) });
    }
    TracePlan::Interface => {
      // gia tri interface: `data` o +24. itable o +16 la du lieu tinh,
      // co y bo qua.
      mark(unsafe { read_ptr(object, crate::iface::IFACE_DATA_OFFSET) });
    }
  }
}

// Di theo bang offset ma descriptor dua ra. Bang do do lower.rs sinh, tuyet
// doi khong go tay o day.
fn trace_fields(object: *mut u8, offsets: *const u32, count: u32) {
  if offsets.is_null() {
    return;
  }
  for index in 0..count as usize {
    let offset = unsafe { *offsets.add(index) } as usize;
    mark(unsafe { read_ptr(object, offset) });
  }
}

// map va set. Ca hai cung mot hinh, chi khac o chuyen cot value co duoc dung
// hay khong. Phai di tung entry mot chu khong di theo index buffer, vi index
// buffer co the con bia mo tro toi entry da xoa.
fn table(object: *mut u8, keys_are_refs: bool, values_are_refs: bool) {
  use crate::map::{
    ENTRIES_OFFSET, ENTRY_HASH_OFFSET, ENTRY_KEY_OFFSET, ENTRY_SIZE, ENTRY_USED_OFFSET,
    ENTRY_VALUE_OFFSET, INDEX_OFFSET,
  };

  let (entries, index, used) = unsafe {
    ( read_ptr(object, ENTRIES_OFFSET), read_ptr(object, INDEX_OFFSET), read_u64(object, ENTRY_USED_OFFSET), )
  };
  mark(index);
  if entries.is_null() {
    return;
  }
  mark(entries);
  if !keys_are_refs && !values_are_refs {
    return;
  }
  for slot in 0..used as usize {
    let entry = HEADER_SIZE + slot * ENTRY_SIZE;
    unsafe {
      if read_u64(entries, entry + ENTRY_HASH_OFFSET) == 0 {
        continue;
      }
      if keys_are_refs {
        mark(read_ptr(entries, entry + ENTRY_KEY_OFFSET));
      }
      if values_are_refs {
        mark(read_ptr(entries, entry + ENTRY_VALUE_OFFSET));
      }
    }
  }
}

// goc

#[cfg(target_arch = "x86_64")]
#[inline(never)]
// Do het thanh ghi callee-saved vao `slots`.
//
// Day la mieng asm duy nhat trong ca repo va t khong thich no ti nao, nhung
// khong co no thi mot con tro dang nam yen trong rbx se khong ai nhin thay,
// va object cua no bi gom mat trong khi van con dung. Loi do thi khong bao
// gio lan ra duoc.
fn spill_callee_saved(slots: &mut [usize; SPILL_SLOTS]) {
  let base = slots.as_mut_ptr();
  unsafe {
    std::arch::asm!(
      "mov [rax + 0], rbx",
      "mov [rax + 8], rbp",
      "mov [rax + 16], rdi",
      "mov [rax + 24], rsi",
      "mov [rax + 32], r12",
      "mov [rax + 80], r13",
      "mov [rax + 96], r14",
      "mov [rax + 112], r15",
      in("rax") base,
      options(nostack, preserves_flags),
    );
  }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(never)]
fn spill_callee_saved(_slots: &mut [usize; SPILL_SLOTS]) {}

// Quet mot khoang stack. Di tung tu 8 byte mot, tu duoi len tren.
fn scan_range(low: usize, high: usize) {
  let mut address = (low + 7) & !7usize;
  while address + 8 <= high {
    let word = unsafe { (address as *const usize).read() };
    mark_conservatively(word);
    address += 8;
  }
}

// Gom het goc lai. Thu tu o day khong quan trong, nhung thieu MOT nguon goc
// thoi la du de chuong trinh do ve
// ghi, bang global_roots, o loi dang treo, may root dang ky tay, va RootScope.
fn mark2(stack_top: usize) {
  // `mark` cung cham vao collector, nen o day tuyet doi khong duoc giu
  // borrow cua no vat qua mot loi goi: chep tung field ra truoc, va list nao
  // cung phai tra qua mot borrow moi. Cho nay t bi borrow checker chui may
  // chuc lan.
  let (bottom, global_roots, root_count) = {
    let collector = collector();
    (
      collector.stack_bottom,
      collector.global_roots,
      collector.root_count as usize,
    )
  };

  if bottom > stack_top {
    scan_range(stack_top, bottom);
  } else {
    scan_range(bottom, stack_top);
  }

  if !global_roots.is_null() {
    for index in 0..root_count {
      mark(unsafe { *global_roots.add(index) });
    }
  }

  mark(unsafe { crate::panic::pump_error_slot });

  for index in 0..collector().extra_roots.len() {
    let slot = collector().extra_roots[index];
    if !slot.is_null() {
      mark(unsafe { *slot });
    }
  }

  for index in 0..collector().temp_roots.len() {
    let object = collector().temp_roots[index];
    mark(object);
  }
}

// goc tam

pub(crate) struct RootScope {
  depth: usize,
}

impl RootScope {
  pub(crate) fn new() -> RootScope {
    RootScope {
      depth: collector().temp_roots.len(),
    }
  }

  pub(crate) fn keep(&self, object: *mut u8) -> *mut u8 {
    if !object.is_null() {
      collector().temp_roots.push(object);
    }
    object
  }

  pub(crate) fn keep_slot(&self, slot: u64, is_reference: bool) {
    if is_reference {
      self.keep(slot as *mut u8);
    }
  }
}

impl Drop for RootScope {
  fn drop(&mut self) {
    collector().temp_roots.truncate(self.depth);
  }
}

// cua vao

/// Run one whole collection: mark from every root, then sweep.
#[no_mangle]
pub extern "C" fn pump_gc_collect() {
  if !can_collect() {
    return;
  }
  heap().collecting = true;

  // vung dem do thanh ghi BAT BUOC phai la bien cuc bo cua chinh frame
  // nay: lan quet bat dau tu dia chi cua no, nen moi thu do vao day va moi
  // frame cua nguoi goi nam tren deu duoc phu. Dat no ra cho khac la mat
  // goc, ma mat goc thi khong ai bao cho minh biet.
  let mut registers = [0usize; SPILL_SLOTS];
  spill_callee_saved(&mut registers);
  let stack_top = registers.as_ptr() as usize;

  mark2(stack_top);
  while let Some(object) = collector().work.pop() {
    trace(object);
  }

  let live = heap().sweep();
  heap().reschedule(live);

  // sweep di tren heap, ma object tinh thi khong nam trong heap, nen bit
  // mark cua may cai immortal phai tu tay xoa theo danh sach ma luot mark
  // vua dung ra. De nguyen bit do thi chu ky sau se bo qua khong di vao
  // object do nua, va gom sach moi thu no dang giu. Bug nay t dinh mot lan
  // roi, mat ba toi moi ra.
  let collector = collector();
  for &object in &collector.marked_immortals {
    unsafe { crate::header(object) }.flags &= !FLAG_MARK;
  }
  collector.marked_immortals.clear();
  collector.collections += 1;

  heap().collecting = false;
}

/// Stop collecting for a while.
#[no_mangle]
pub extern "C" fn pump_gc_disable() {
  heap().disable_depth += 1;
}

/// Start collecting again.
#[no_mangle]
pub extern "C" fn pump_gc_enable() {
  let heap = heap();
  heap.disable_depth = heap.disable_depth.saturating_sub(1);
}

/// Register a pointer slot as a root that never goes away.
#[no_mangle]
pub extern "C" fn pump_gc_add_root(slot: *mut *mut u8) {
  if !slot.is_null() {
    collector().extra_roots.push(slot);
  }
}

/// Take back a slot that pump_gc_add_root registered before.
#[no_mangle]
pub extern "C" fn pump_gc_remove_root(slot: *mut *mut u8) {
  let roots = &mut collector().extra_roots;
  if let Some(position) = roots.iter().position(|&existing| existing == slot) {
    roots.remove(position);
  }
}

/// How many collections have finished.
pub fn collection_count() -> u64 {
  collector().collections
}

#[cfg(test)]
pub(crate) fn reset() {
  let heap = heap();
  heap.collecting = false;
  heap.disable_depth = 0;
  heap.release();
  *collector() = Collector::new();
  unsafe { crate::panic::pump_error_slot = std::ptr::null_mut() };
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::alloc::{heap_bytes_reserved, heap_object_count, pump_alloc};
  use crate::testing::{self, TestTypes};

  #[inline(never)]
  fn clobber_stack() {
    let mut scratch = [0usize; 1024];
    for (index, slot) in scratch.iter_mut().enumerate() {
      *slot = index;
    }
    std::hint::black_box(&scratch);
  }

  #[inline(never)]
  fn build_garbage(types: &TestTypes, count: usize) {
    for _ in 0..count {
      let node = pump_alloc(types.node, 32);
      std::hint::black_box(node);
    }
  }

  #[inline(never)]
  fn build_cycle(types: &TestTypes, count: usize) {
    let scope = RootScope::new();
    let first = scope.keep(pump_alloc(types.node, 32));
    let mut previous = first;
    for _ in 1..count {
      let node = scope.keep(pump_alloc(types.node, 32));
      unsafe { crate::write_ptr(previous, 16, node) };
      previous = node;
    }
    unsafe { crate::write_ptr(previous, 16, first) };
    drop(scope);
  }

  #[test]
  fn a_reachable_chain_survives_collection() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    let scope = RootScope::new();
    let head = scope.keep(pump_alloc(types.node, 32));
    let mut previous = head;
    for _ in 0..64 {
      let node = pump_alloc(types.node, 32);
      unsafe { crate::write_ptr(previous, 16, node) };
      previous = node;
    }

    pump_gc_collect();

    // di het day: mat xich nao cung phai con la mot node dang song.
    let mut cursor = head;
    for _ in 0..65 {
      unsafe {
        assert_eq!(crate::type_id_of(cursor), types.node);
        let next = crate::read_ptr(cursor, 16);
        if next.is_null() {
          break;
        }
        cursor = next;
      }
    }
    drop(scope);
  }

  #[test]
  fn a_reference_cycle_is_collected() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    pump_gc_collect();
    let before = heap_object_count();

    build_cycle(&types, 32);
    assert!(heap_object_count() >= before + 32);

    clobber_stack();
    pump_gc_collect();

    // vong tron chinh la cai ma dem tham chieu khong doi lai duoc. Chua
    // mot chut du cho cai tu cu con giong dia chi node: quet bao thu khong
    // the hua bang khong duoc.
    let after = heap_object_count();
    assert!( after <= before + 4, "a 32-node cycle was not collected: {before} objects before, {after} after" );
  }

  #[test]
  fn allocation_under_pressure_collects_rather_than_growing() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    // gap muoi sau lan nguong gom, va tat ca deu la rac.
    let objects = crate::alloc::MIN_HEAP_BYTES / 32 * 16;
    build_garbage(&types, objects);
    clobber_stack();

    assert!(
      collection_count() > 0,
      "allocating {objects} objects never triggered a collection"
    );
    let reserved = heap_bytes_reserved();
    assert!(
      reserved <= crate::alloc::MIN_HEAP_BYTES * 4,
      "the heap grew to {reserved} bytes to hold nothing but garbage"
    );
  }

  #[test]
  fn a_long_allocate_and_drop_loop_does_not_grow_without_bound() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    build_garbage(&types, 20_000);
    clobber_stack();
    pump_gc_collect();
    let settled = heap_bytes_reserved();

    for _ in 0..20 {
      build_garbage(&types, 20_000);
    }
    clobber_stack();
    pump_gc_collect();

    assert_eq!( heap_bytes_reserved(), settled, "twenty more rounds of pure garbage grew the heap" );
  }

  #[test]
  fn an_immortal_object_survives_and_ends_unmarked() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    // mot object tinh, dung kieu compiler sinh ra cho chuoi literal:
    // khong nam trong heap, va co deo FLAG_IMMORTAL.
    let mut statics = testing::StaticNode::new(types.node);
    let node = statics.as_ptr();

    let scope = RootScope::new();
    let target = pump_alloc(types.node, 32);
    unsafe { crate::write_ptr(node, 16, target) };
    scope.keep(node);

    pump_gc_collect();

    unsafe {
      assert_eq!(crate::header(node).flags, crate::FLAG_IMMORTAL);
      assert_eq!(crate::type_id_of(target), types.node);
    }
    drop(scope);
  }

  #[test]
  fn disabling_the_collector_suppresses_collection() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    pump_gc_disable();
    let before = collection_count();
    build_garbage(&types, crate::alloc::MIN_HEAP_BYTES / 32 * 2);
    assert_eq!(collection_count(), before);
    pump_gc_enable();

    pump_gc_collect();
    assert_eq!(collection_count(), before + 1);
  }

  #[test]
  fn a_registered_root_keeps_its_object_alive() {
    let _guard = testing::guard();
    let types = testing::install_type_table();
    let mut bottom = 0usize;
    testing::init_runtime(&mut bottom);

    let mut slot: *mut u8 = pump_alloc(types.node, 32);
    pump_gc_add_root(&mut slot);
    clobber_stack();
    pump_gc_collect();
    assert_eq!(unsafe { crate::type_id_of(slot) }, types.node);
    pump_gc_remove_root(&mut slot);
  }
}
