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
