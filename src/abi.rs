// ABI. Hinh dang object, itable, ten symbol, quy uoc goi ham.
//
// file nay va docs/abi.md la cung mot tai lieu viet ra hai lan. clif.rs lam
// mot nua, runtime/src/gc.rs lam nua kia. Hai nua ma noi khac nhau la
// segfault, ma segfault kieu do khong lan ra duoc la loi cua ai. Sua thi sua
// ca hai, khong thi dung dung vao.
//
// RIENG file nay t co y ghi chu thich bang tieng Anh, de doi chieu tung dong
// voi docs/abi.md cho de. May file khac thi khong can.
//
// x86_64-pc-windows-msvc, little endian, con tro 8 byte, dung quy uoc goi C
// cua nen tang o moi cho.
//
// Object nao tren heap cung bat dau bang cung mot header 16 byte:
//
//   +0   u32  type_id   chi so vao bang type descriptor
//   +4   u32  flags     bit 0 MARK, bit 1 IMMORTAL, con lai bang 0
//   +8   u64  size      ca object tinh bang byte, ke ca header, boi cua 16
//   +16  ...  payload
//
// Object tra ve luon can 16 byte, nen payload cung can theo va mot field f64
// khong bao gio nam vat qua hai cache line.

use crate::types::{TypeContext, TypeId, TypeKind};

// -- kieu cua may

/// The machine-level types the backend deals in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IrType {
    I8,
    I32,
    I64,
    F64,
    Ptr,
}

impl IrType {
    pub fn size(self) -> u32 {
        match self {
            IrType::I8 => 1,
            IrType::I32 => 4,
            IrType::I64 | IrType::F64 | IrType::Ptr => 8,
        }
    }

    pub fn align(self) -> u32 {
        self.size()
    }

    pub fn is_pointer(self) -> bool {
        self == IrType::Ptr
    }

    /// The machine representation of a resolved Pump type.
    pub fn of(context: &TypeContext, ty: TypeId) -> IrType {
        match context.kind(context.shallow_resolve(ty)) {
            TypeKind::Bool => IrType::I8,
            TypeKind::Char => IrType::I32,
            TypeKind::Int | TypeKind::Uint | TypeKind::UntypedInt => IrType::I64,
            TypeKind::Float | TypeKind::UntypedFloat => IrType::F64,
            TypeKind::String
            | TypeKind::Array(_)
            | TypeKind::Map { .. }
            | TypeKind::Set(_)
            | TypeKind::Tuple(_)
            | TypeKind::Optional(_)
            | TypeKind::Function(_)
            | TypeKind::Named { .. } => IrType::Ptr,
            // `Never` voi `Error` chi toi duoc backend tren nhung duong
            // khong bao gio sinh ra gia tri, nen de tam mot cai cho trong to
            // bang con tro thi cung khong sao.
            TypeKind::Never | TypeKind::Error => IrType::Ptr,
            other => panic!("type {other:?} has no machine representation"),
        }
    }

    /// The machine representation of a return type, `None` for `void`.
    pub fn of_return(context: &TypeContext, ty: TypeId) -> Option<IrType> {
        match context.kind(context.shallow_resolve(ty)) {
            TypeKind::Void => None,
            _ => Some(IrType::of(context, ty)),
        }
    }
}

// -- header cua object

/// Size of the header that precedes every heap object, in bytes.
pub const HEADER_SIZE: u32 = 16;

/// Alignment of every heap object.
pub const OBJECT_ALIGN: u32 = 16;

/// Byte offset of the `type_id` field within the header.
pub const HEADER_TYPE_ID_OFFSET: u32 = 0;
/// Byte offset of the `flags` field within the header.
pub const HEADER_FLAGS_OFFSET: u32 = 4;
/// Byte offset of the `size` field within the header.
pub const HEADER_SIZE_OFFSET: u32 = 8;

/// Header flag: the collector has reached this object during the current mark
/// phase.
pub const FLAG_MARK: u32 = 1 << 0;
/// Header flag: the object lives in static data or is otherwise never swept.
pub const FLAG_IMMORTAL: u32 = 1 << 1;

/// Rounds an object size up to `OBJECT_ALIGN`.
pub const fn align_object_size(size: u64) -> u64 {
    let align = OBJECT_ALIGN as u64;
    (size + align - 1) & !(align - 1)
}

/// Rounds `offset` up to `align`, which must be a power of two.
pub const fn align_to(offset: u64, align: u32) -> u64 {
    let align = align as u64;
    (offset + align - 1) & !(align - 1)
}

// -- may type id giu san

/// Never a valid object.
pub const TYPE_ID_INVALID: u32 = 0;
/// A raw byte buffer.
pub const TYPE_ID_BUFFER: u32 = 1;
/// A `string`.
pub const TYPE_ID_STRING: u32 = 2;
/// A box holding one non-pointer value: `int?`, `float?`, `bool?`, `char?`,
/// and a captured scalar binding.
pub const TYPE_ID_BOX_SCALAR: u32 = 3;
/// A box holding one pointer value: a captured reference binding.
pub const TYPE_ID_BOX_REF: u32 = 4;
/// An interface value: `{ itable, data }`.
pub const TYPE_ID_INTERFACE: u32 = 5;

/// The first type id available to compiler-emitted descriptors.
pub const FIRST_USER_TYPE_ID: u32 = 16;

// -- hinh dang cua may object co san

pub mod string {
    use super::HEADER_SIZE;

    /// Byte count of the UTF-8 contents.
    pub const LENGTH_OFFSET: u32 = HEADER_SIZE;
    /// Cached FNV-1a 64 hash of the contents.
    pub const HASH_OFFSET: u32 = HEADER_SIZE + 8;
    /// First content byte.
    pub const BYTES_OFFSET: u32 = HEADER_SIZE + 16;

    /// Total object size for a string of `length` bytes, before alignment.
    pub const fn unaligned_size(length: u64) -> u64 {
        BYTES_OFFSET as u64 + length + 1
    }
}

pub mod array {
    use super::HEADER_SIZE;

    /// Number of live elements.
    pub const LENGTH_OFFSET: u32 = HEADER_SIZE;
    /// Number of element slots the buffer can hold.
    pub const CAPACITY_OFFSET: u32 = HEADER_SIZE + 8;
    /// Pointer to the element buffer object, or null when capacity is zero.
    pub const DATA_OFFSET: u32 = HEADER_SIZE + 16;
    /// Modification counter, incremented by every structural change.
    pub const MODCOUNT_OFFSET: u32 = HEADER_SIZE + 24;

    /// Total size of an array object.
    pub const SIZE: u64 = HEADER_SIZE as u64 + 32;

    /// Byte offset of element `index` within the element buffer object, i.e.
    pub const fn element_offset(index: u64) -> u64 {
        HEADER_SIZE as u64 + index * super::SLOT_SIZE as u64
    }
}

pub mod map {
    use super::HEADER_SIZE;

    /// Number of live entries.
    pub const LENGTH_OFFSET: u32 = HEADER_SIZE;
    /// Pointer to the entry buffer object, or null.
    pub const ENTRIES_OFFSET: u32 = HEADER_SIZE + 8;
    /// Entry slots the buffer can hold.
    pub const ENTRY_CAPACITY_OFFSET: u32 = HEADER_SIZE + 16;
    /// Entry slots used so far, tombstones included.
    pub const ENTRY_USED_OFFSET: u32 = HEADER_SIZE + 24;
    /// Pointer to the index buffer object, or null.
    pub const INDEX_OFFSET: u32 = HEADER_SIZE + 32;
    /// Index slots, always a power of two.
    pub const INDEX_CAPACITY_OFFSET: u32 = HEADER_SIZE + 40;
    /// Modification counter.
    pub const MODCOUNT_OFFSET: u32 = HEADER_SIZE + 48;
    /// How keys are hashed and compared; one of the `KEY_KIND_*` constants.
    pub const KEY_KIND_OFFSET: u32 = HEADER_SIZE + 56;
    /// Bit 0: the value slot holds a pointer.
    pub const SLOT_FLAGS_OFFSET: u32 = HEADER_SIZE + 120;

    /// Total size of a map object.
    pub const SIZE: u64 = HEADER_SIZE as u64 + 64;

    /// `slot_flags` bit: the value slot holds a pointer.
    pub const SLOT_FLAG_VALUE_IS_REF: u32 = 1 << 0;
    /// `slot_flags` bit: the key slot holds a pointer.
    pub const SLOT_FLAG_KEY_IS_REF: u32 = 1 << 1;

    /// Key is compared bitwise on its 8-byte slot: `int`, `uint`, `char`,
    /// `bool`, and payload-free enums.
    pub const KEY_KIND_SCALAR: u32 = 0;
    /// Key is a `string`; hashed and compared by UTF-8 content.
    pub const KEY_KIND_STRING: u32 = 1;
    /// Key is compared by object identity.
    pub const KEY_KIND_REFERENCE: u32 = 2;
    /// Key is a tuple, hashed and compared structurally.
    pub const KEY_KIND_TUPLE: u32 = 3;

    /// Bytes per entry in the entry buffer.
    pub const ENTRY_SIZE: u64 = 24;
    /// Offset of the entry's hash within an entry.
    pub const ENTRY_HASH_OFFSET: u64 = 0;
    /// Offset of the key slot within an entry.
    pub const ENTRY_KEY_OFFSET: u64 = 8;
    /// Offset of the value slot within an entry.
    pub const ENTRY_VALUE_OFFSET: u64 = 16;

    /// Bytes per slot in the index buffer.
    pub const INDEX_SLOT_SIZE: u64 = 8;
    /// Index slot value meaning "no entry".
    pub const INDEX_EMPTY: i64 = -1;
    /// Index slot value meaning "an entry was here and was removed".
    pub const INDEX_TOMBSTONE: i64 = -2;

    /// Byte offset of entry `index` within the entry buffer object.
    pub const fn entry_offset(index: u64) -> u64 {
        HEADER_SIZE as u64 + index * ENTRY_SIZE
    }
}

pub mod set {
    pub use super::map::{
        ENTRIES_OFFSET, ENTRY_CAPACITY_OFFSET, ENTRY_USED_OFFSET, INDEX_CAPACITY_OFFSET,
        INDEX_OFFSET, KEY_KIND_OFFSET, LENGTH_OFFSET, MODCOUNT_OFFSET, SIZE, SLOT_FLAGS_OFFSET,
    };
}

pub mod closure {
    use super::HEADER_SIZE;

    /// Address of the compiled closure body.
    pub const CODE_OFFSET: u32 = HEADER_SIZE;
    /// Number of captured bindings.
    pub const CAPTURE_COUNT_OFFSET: u32 = HEADER_SIZE + 8;
    /// First capture slot.
    pub const CAPTURES_OFFSET: u32 = HEADER_SIZE + 16;

    /// Total object size for a closure with `count` captures, before
    /// alignment.
    pub const fn unaligned_size(count: u64) -> u64 {
        CAPTURES_OFFSET as u64 + count * 8
    }

    /// Byte offset of capture `index` within the closure object.
    pub const fn capture_offset(index: u64) -> u64 {
        CAPTURES_OFFSET as u64 + index * 8
    }
}

pub mod boxed {
    use super::HEADER_SIZE;

    /// The boxed value, always occupying 8 bytes.
    pub const VALUE_OFFSET: u32 = HEADER_SIZE;
    /// Total size, before alignment.
    pub const UNALIGNED_SIZE: u64 = HEADER_SIZE as u64 + 8;
}

pub mod interface {
    use super::HEADER_SIZE;

    /// Pointer to the static itable.
    pub const ITABLE_OFFSET: u32 = HEADER_SIZE;
    /// Pointer to the underlying object, or to a box when the concrete type
    /// is a primitive.
    pub const DATA_OFFSET: u32 = HEADER_SIZE + 8;
    /// Total size, before alignment.
    pub const UNALIGNED_SIZE: u64 = HEADER_SIZE as u64 + 16;
}

pub mod enumeration {
    use super::HEADER_SIZE;

    /// The variant's declaration index.
    pub const TAG_OFFSET: u32 = HEADER_SIZE;
    /// First payload byte.
    pub const PAYLOAD_OFFSET: u32 = HEADER_SIZE + 8;
}

pub mod structure {
    use super::HEADER_SIZE;

    /// First field byte.
    pub const FIELDS_OFFSET: u32 = HEADER_SIZE;
}

/// Every slot in an array, a map or a set is 8 bytes wide, regardless of the
/// element type.
pub const SLOT_SIZE: u32 = 8;

// -- type descriptor

/// Discriminant of `TypeDescriptor::kind`.
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

/// Descriptor flag: an array's, a set's or a box's slot holds a pointer.
pub const DESC_FLAG_ELEM_IS_REF: u32 = 1 << 0;
/// Descriptor flag: a map's key slot holds a pointer.
pub const DESC_FLAG_KEY_IS_REF: u32 = 1 << 1;
/// Descriptor flag: a map's value slot holds a pointer.
pub const DESC_FLAG_VALUE_IS_REF: u32 = 1 << 2;

/// Size of one entry in the static type descriptor table, in bytes.
pub const TYPE_DESCRIPTOR_SIZE: u32 = 48;

pub mod descriptor {
    /// `super::DescriptorKind`.
    pub const KIND_OFFSET: u32 = 0;
    /// `DESC_FLAG_*` bits.
    pub const FLAGS_OFFSET: u32 = 4;
    /// Fixed instance size in bytes, or 0 when the size is per-instance and
    /// must be read from the header.
    pub const SIZE_OFFSET: u32 = 8;
    /// Number of entries in `ref_offsets`.
    pub const REF_COUNT_OFFSET: u32 = 16;
    /// Number of entries in `variants`.
    pub const VARIANT_COUNT_OFFSET: u32 = 20;
    /// Pointer to a `u32` array of pointer-field byte offsets, or null.
    pub const REF_OFFSETS_OFFSET: u32 = 24;
    /// Pointer to an array of variant descriptors, or null.
    pub const VARIANTS_OFFSET: u32 = 32;
    /// Pointer to a NUL-terminated type name, for diagnostics.
    pub const NAME_OFFSET: u32 = 40;
}

/// Size of one variant descriptor, in bytes.
pub const VARIANT_DESCRIPTOR_SIZE: u32 = 24;

pub mod variant_descriptor {
    /// Number of entries in `ref_offsets`.
    pub const REF_COUNT_OFFSET: u32 = 0;
    /// Must be zero.
    pub const RESERVED_OFFSET: u32 = 4;
    /// Pointer to a `u32` array of pointer-field byte offsets, or null.
    pub const REF_OFFSETS_OFFSET: u32 = 8;
    /// Pointer to a NUL-terminated variant name.
    pub const NAME_OFFSET: u32 = 16;
}

/// The compiler-side description of one entry in the static type table.
#[derive(Clone, Debug)]
pub struct TypeDescriptor {
    pub type_id: u32,
    pub kind: DescriptorKind,
    pub flags: u32,
    pub size: u64,
    pub ref_offsets: Vec<u32>,
    pub variants: Vec<VariantDescriptor>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct VariantDescriptor {
    pub ref_offsets: Vec<u32>,
    pub name: String,
}

// -- hinh dang cua struct, tuple, enum

/// Where one field lives inside its object.
#[derive(Clone, Copy, Debug)]
pub struct FieldLayout {
    pub offset: u32,
    pub ty: IrType,
    pub is_ref: bool,
}

/// The computed layout of a struct, a tuple, or one enum variant's payload.
#[derive(Clone, Debug)]
pub struct RecordLayout {
    pub size: u64,
    pub fields: Vec<FieldLayout>,
    pub ref_offsets: Vec<u32>,
}

/// Lays out `fields` starting at `start_offset`, in declaration order, each
/// at its natural alignment.
pub fn la_record(context: &TypeContext, fields: &[TypeId], start_offset: u32) -> RecordLayout {
    let mut offset = start_offset as u64;
    let mut laid_out = Vec::with_capacity(fields.len());
    let mut ref_offsets = Vec::new();

    for &field in fields {
        let ty = IrType::of(context, field);
        offset = align_to(offset, ty.align());
        let is_ref = ty.is_pointer();
        if is_ref {
            ref_offsets.push(offset as u32);
        }
        laid_out.push(FieldLayout {
            offset: offset as u32,
            ty,
            is_ref,
        });
        offset += ty.size() as u64;
    }

    RecordLayout {
        size: align_object_size(offset.max(HEADER_SIZE as u64)),
        fields: laid_out,
        ref_offsets,
    }
}

/// Lays out a struct instance: fields from `structure::FIELDS_OFFSET`.
pub fn layout_struct(context: &TypeContext, fields: &[TypeId]) -> RecordLayout {
    la_record(context, fields, structure::FIELDS_OFFSET)
}

/// Lays out a tuple, which is a struct whose fields are its elements.
pub fn layout_tuple(context: &TypeContext, elements: &[TypeId]) -> RecordLayout {
    la_record(context, elements, structure::FIELDS_OFFSET)
}

/// Lays out one enum variant's payload, from `enumeration::PAYLOAD_OFFSET`.
pub fn layout_variant(context: &TypeContext, payload: &[TypeId]) -> RecordLayout {
    let layout = la_record(context, payload, enumeration::PAYLOAD_OFFSET);
    RecordLayout {
        size: layout
            .size
            .max(align_object_size(enumeration::PAYLOAD_OFFSET as u64)),
        ..layout
    }
}

// -- itable

/// Byte offset of the interface's `DefId` within an itable.
pub const ITABLE_INTERFACE_ID_OFFSET: u32 = 0;
/// Byte offset of the concrete type's runtime type id within an itable.
pub const ITABLE_TYPE_ID_OFFSET: u32 = 8;
/// Byte offset of the method count within an itable.
pub const ITABLE_METHOD_COUNT_OFFSET: u32 = 16;
/// Byte offset of the first method pointer within an itable.
pub const ITABLE_METHODS_OFFSET: u32 = 24;

/// Byte offset of the method pointer for interface slot `slot`.
pub const fn itable_method_offset(slot: u32) -> u32 {
    ITABLE_METHODS_OFFSET + slot * 8
}

/// Total size of an itable with `method_count` methods.
pub const fn itable_size(method_count: u32) -> u64 {
    ITABLE_METHODS_OFFSET as u64 + method_count as u64 * 8
}

// -- symbol ma compiler bat buoc phai sinh ra

/// The C entry point.
pub const SYMBOL_C_MAIN: &str = "main";

/// `void pump_module_init(void)` - compiler-emitted.
pub const SYMBOL_MODULE_INIT: &str = "pump_module_init";

/// `int32_t pump_program_main(void)` - compiler-emitted.
pub const SYMBOL_PROGRAM_MAIN: &str = "pump_program_main";

/// The static type descriptor table.
pub const SYMBOL_TYPE_TABLE: &str = "pump_type_table";

/// The global root table.
pub const SYMBOL_GLOBAL_ROOTS: &str = "pump_global_roots";

/// The pending-error slot.
pub const SYMBOL_ERROR_SLOT: &str = "pump_error_slot";

// -- cua vao cua runtime

/// The signature of a runtime entry point.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeSignature {
    pub params: &'static [IrType],
    pub ret: Option<IrType>,
    pub diverges: bool,
}

const fn sig(params: &'static [IrType], ret: Option<IrType>) -> RuntimeSignature {
    RuntimeSignature {
        params,
        ret,
        diverges: false,
    }
}
