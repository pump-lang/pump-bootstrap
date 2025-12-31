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
    pub const SLOT_FLAGS_OFFSET: u32 = HEADER_SIZE + 60;

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
pub fn layout_record(context: &TypeContext, fields: &[TypeId], start_offset: u32) -> RecordLayout {
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
    layout_record(context, fields, structure::FIELDS_OFFSET)
}

/// Lays out a tuple, which is a struct whose fields are its elements.
pub fn layout_tuple(context: &TypeContext, elements: &[TypeId]) -> RecordLayout {
    layout_record(context, elements, structure::FIELDS_OFFSET)
}

/// Lays out one enum variant's payload, from `enumeration::PAYLOAD_OFFSET`.
pub fn layout_variant(context: &TypeContext, payload: &[TypeId]) -> RecordLayout {
    let layout = layout_record(context, payload, enumeration::PAYLOAD_OFFSET);
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

const fn noreturn(params: &'static [IrType]) -> RuntimeSignature {
    RuntimeSignature {
        params,
        ret: None,
        diverges: true,
    }
}

/// Every function the runtime exports to compiled code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RuntimeFn {
    // ---- lifecycle ----
    RtInit,
    RtShutdown,

    // ---- allocation and collection ----
    Alloc,
    AllocBuffer,
    GcCollect,
    GcDisable,
    GcEnable,
    GcAddRoot,
    GcRemoveRoot,

    // ---- panics, all diverging ----
    Panic,
    PanicCstr,
    PanicIndex,
    PanicDivideByZero,
    PanicNull,
    PanicNegativeShift,
    PanicMissingKey,
    PanicConcurrentModification,
    Exit,

    // ---- the pending-error slot ----
    ErrorSet,
    ErrorTake,
    ErrorPending,

    // ---- strings ----
    StringNew,
    StringConcat,
    StringEq,
    StringCmp,
    StringHash,
    StringLen,
    StringCharCount,
    StringByteAt,
    StringSlice,
    StringChars,
    StringFromChar,
    StringFromBool,
    StringFromInt,
    StringFromUint,
    StringFromFloat,
    CharFromUint,

    // ---- output ----
    Print,
    Println,
    PrintError,

    // ---- arrays ----
    ArrayNew,
    ArrayWithLength,
    ArrayLen,
    ArrayGet,
    ArraySet,
    ArrayPush,
    ArrayPop,
    ArrayReserve,
    ArrayConcat,
    ArraySlice,

    // ---- maps ----
    MapNew,
    MapLen,
    MapLookup,
    MapGet,
    MapSet,
    MapRemove,
    MapHas,
    MapKeys,
    MapValues,
    MapIterNext,

    // ---- sets ----
    SetNew,
    SetLen,
    SetAdd,
    SetHas,
    SetRemove,
    SetIterNext,

    // ---- iteration guard ----
    CollectionModcount,

    // ---- composite constructors ----
    ClosureNew,
    BoxNew,
    IfaceNew,

    // ---- file va tien trinh ----
    ReadFileText,
    ReadFileBytes,
    WriteFileText,
    WriteFileBytes,
    OsArgs,
    OsRun,
    OsError,
}

impl RuntimeFn {
    /// Every entry point, in declaration order.
    pub const ALL: &'static [RuntimeFn] = &[
        RuntimeFn::RtInit,
        RuntimeFn::RtShutdown,
        RuntimeFn::Alloc,
        RuntimeFn::AllocBuffer,
        RuntimeFn::GcCollect,
        RuntimeFn::GcDisable,
        RuntimeFn::GcEnable,
        RuntimeFn::GcAddRoot,
        RuntimeFn::GcRemoveRoot,
        RuntimeFn::Panic,
        RuntimeFn::PanicCstr,
        RuntimeFn::PanicIndex,
        RuntimeFn::PanicDivideByZero,
        RuntimeFn::PanicNull,
        RuntimeFn::PanicNegativeShift,
        RuntimeFn::PanicMissingKey,
        RuntimeFn::PanicConcurrentModification,
        RuntimeFn::Exit,
        RuntimeFn::ErrorSet,
        RuntimeFn::ErrorTake,
        RuntimeFn::ErrorPending,
        RuntimeFn::StringNew,
        RuntimeFn::StringConcat,
        RuntimeFn::StringEq,
        RuntimeFn::StringCmp,
        RuntimeFn::StringHash,
        RuntimeFn::StringLen,
        RuntimeFn::StringCharCount,
        RuntimeFn::StringByteAt,
        RuntimeFn::StringSlice,
        RuntimeFn::StringChars,
        RuntimeFn::StringFromChar,
        RuntimeFn::StringFromBool,
        RuntimeFn::StringFromInt,
        RuntimeFn::StringFromUint,
        RuntimeFn::StringFromFloat,
        RuntimeFn::CharFromUint,
        RuntimeFn::Print,
        RuntimeFn::Println,
        RuntimeFn::PrintError,
        RuntimeFn::ArrayNew,
        RuntimeFn::ArrayWithLength,
        RuntimeFn::ArrayLen,
        RuntimeFn::ArrayGet,
        RuntimeFn::ArraySet,
        RuntimeFn::ArrayPush,
        RuntimeFn::ArrayPop,
        RuntimeFn::ArrayReserve,
        RuntimeFn::ArrayConcat,
        RuntimeFn::ArraySlice,
        RuntimeFn::MapNew,
        RuntimeFn::MapLen,
        RuntimeFn::MapLookup,
        RuntimeFn::MapGet,
        RuntimeFn::MapSet,
        RuntimeFn::MapRemove,
        RuntimeFn::MapHas,
        RuntimeFn::MapKeys,
        RuntimeFn::MapValues,
        RuntimeFn::MapIterNext,
        RuntimeFn::SetNew,
        RuntimeFn::SetLen,
        RuntimeFn::SetAdd,
        RuntimeFn::SetHas,
        RuntimeFn::SetRemove,
        RuntimeFn::SetIterNext,
        RuntimeFn::CollectionModcount,
        RuntimeFn::ClosureNew,
        RuntimeFn::BoxNew,
        RuntimeFn::IfaceNew,
        RuntimeFn::ReadFileText,
        RuntimeFn::ReadFileBytes,
        RuntimeFn::WriteFileText,
        RuntimeFn::WriteFileBytes,
        RuntimeFn::OsArgs,
        RuntimeFn::OsRun,
        RuntimeFn::OsError,
    ];

    /// The unmangled C symbol name.
    pub fn symbol(self) -> &'static str {
        use RuntimeFn::*;
        match self {
            RtInit => "pump_rt_init",
            RtShutdown => "pump_rt_shutdown",
            Alloc => "pump_alloc",
            AllocBuffer => "pump_alloc_buffer",
            GcCollect => "pump_gc_collect",
            GcDisable => "pump_gc_disable",
            GcEnable => "pump_gc_enable",
            GcAddRoot => "pump_gc_add_root",
            GcRemoveRoot => "pump_gc_remove_root",
            Panic => "pump_panic",
            PanicCstr => "pump_panic_cstr",
            PanicIndex => "pump_panic_index",
            PanicDivideByZero => "pump_panic_divide_by_zero",
            PanicNull => "pump_panic_null",
            PanicNegativeShift => "pump_panic_negative_shift",
            PanicMissingKey => "pump_panic_missing_key",
            PanicConcurrentModification => "pump_panic_concurrent_modification",
            Exit => "pump_exit",
            ErrorSet => "pump_error_set",
            ErrorTake => "pump_error_take",
            ErrorPending => "pump_error_pending",
            StringNew => "pump_string_new",
            StringConcat => "pump_string_concat",
            StringEq => "pump_string_eq",
            StringCmp => "pump_string_cmp",
            StringHash => "pump_string_hash",
            StringLen => "pump_string_len",
            StringCharCount => "pump_string_char_count",
            StringByteAt => "pump_string_byte_at",
            StringSlice => "pump_string_slice",
            StringChars => "pump_string_chars",
            StringFromChar => "pump_string_from_char",
            StringFromBool => "pump_string_from_bool",
            StringFromInt => "pump_string_from_int",
            StringFromUint => "pump_string_from_uint",
            StringFromFloat => "pump_string_from_float",
            CharFromUint => "pump_char_from_uint",
            Print => "pump_print",
            Println => "pump_println",
            PrintError => "pump_print_error",
            ArrayNew => "pump_array_new",
            ArrayWithLength => "pump_array_with_length",
            ArrayLen => "pump_array_len",
            ArrayGet => "pump_array_get",
            ArraySet => "pump_array_set",
            ArrayPush => "pump_array_push",
            ArrayPop => "pump_array_pop",
            ArrayReserve => "pump_array_reserve",
            ArrayConcat => "pump_array_concat",
            ArraySlice => "pump_array_slice",
            MapNew => "pump_map_new",
            MapLen => "pump_map_len",
            MapLookup => "pump_map_lookup",
            MapGet => "pump_map_get",
            MapSet => "pump_map_set",
            MapRemove => "pump_map_remove",
            MapHas => "pump_map_has",
            MapKeys => "pump_map_keys",
            MapValues => "pump_map_values",
            MapIterNext => "pump_map_iter_next",
            SetNew => "pump_set_new",
            SetLen => "pump_set_len",
            SetAdd => "pump_set_add",
            SetHas => "pump_set_has",
            SetRemove => "pump_set_remove",
            SetIterNext => "pump_set_iter_next",
            CollectionModcount => "pump_collection_modcount",
            ClosureNew => "pump_closure_new",
            BoxNew => "pump_box_new",
            IfaceNew => "pump_iface_new",
            ReadFileText => "pump_read_file_text",
            ReadFileBytes => "pump_read_file_bytes",
            WriteFileText => "pump_write_file_text",
            WriteFileBytes => "pump_write_file_bytes",
            OsArgs => "pump_os_args",
            OsRun => "pump_os_run",
            OsError => "pump_os_error",
        }
    }

    /// The exact machine signature.
    pub fn signature(self) -> RuntimeSignature {
        use IrType::*;
        use RuntimeFn::*;
        match self {
            RtInit => sig(&[Ptr, Ptr, I64, Ptr, I64, I32, Ptr], None),
            RtShutdown => sig(&[I32], None),

            Alloc => sig(&[I32, I64], Some(Ptr)),
            AllocBuffer => sig(&[I64], Some(Ptr)),
            GcCollect | GcDisable | GcEnable => sig(&[], None),
            GcAddRoot | GcRemoveRoot => sig(&[Ptr], None),

            Panic => noreturn(&[Ptr]),
            PanicCstr => noreturn(&[Ptr, I64]),
            PanicIndex => noreturn(&[I64, I64]),
            PanicDivideByZero | PanicNull | PanicMissingKey | PanicConcurrentModification => {
                noreturn(&[])
            }
            PanicNegativeShift => noreturn(&[I64]),
            Exit => noreturn(&[I32]),

            ErrorSet => sig(&[Ptr], None),
            ErrorTake => sig(&[], Some(Ptr)),
            ErrorPending => sig(&[], Some(I8)),

            StringNew => sig(&[Ptr, I64], Some(Ptr)),
            StringConcat => sig(&[Ptr, Ptr], Some(Ptr)),
            StringEq => sig(&[Ptr, Ptr], Some(I8)),
            StringCmp => sig(&[Ptr, Ptr], Some(I64)),
            StringHash | StringLen | StringCharCount => sig(&[Ptr], Some(I64)),
            StringByteAt => sig(&[Ptr, I64], Some(I64)),
            StringSlice => sig(&[Ptr, I64, I64], Some(Ptr)),
            StringChars => sig(&[Ptr], Some(Ptr)),
            StringFromChar => sig(&[I32], Some(Ptr)),
            StringFromBool => sig(&[I8], Some(Ptr)),
            StringFromInt | StringFromUint => sig(&[I64], Some(Ptr)),
            StringFromFloat => sig(&[F64], Some(Ptr)),
            CharFromUint => sig(&[I64], Some(I32)),

            Print | Println | PrintError => sig(&[Ptr], None),

            ArrayNew | ArrayWithLength => sig(&[I32, I64], Some(Ptr)),
            ArrayLen => sig(&[Ptr], Some(I64)),
            ArrayGet => sig(&[Ptr, I64], Some(I64)),
            ArraySet => sig(&[Ptr, I64, I64], None),
            ArrayPush => sig(&[Ptr, I64], None),
            ArrayPop => sig(&[Ptr], Some(I64)),
            ArrayReserve => sig(&[Ptr, I64], None),
            ArrayConcat => sig(&[Ptr, Ptr], Some(Ptr)),
            ArraySlice => sig(&[Ptr, I64, I64], Some(Ptr)),

            MapNew => sig(&[I32], Some(Ptr)),
            MapLen => sig(&[Ptr], Some(I64)),
            MapLookup => sig(&[Ptr, I64, Ptr], Some(I8)),
            MapGet => sig(&[Ptr, I64], Some(I64)),
            MapSet => sig(&[Ptr, I64, I64], None),
            MapRemove | MapHas => sig(&[Ptr, I64], Some(I8)),
            MapKeys | MapValues => sig(&[Ptr], Some(Ptr)),
            MapIterNext => sig(&[Ptr, Ptr, Ptr, Ptr], Some(I8)),

            SetNew => sig(&[I32], Some(Ptr)),
            SetLen => sig(&[Ptr], Some(I64)),
            SetAdd | SetHas | SetRemove => sig(&[Ptr, I64], Some(I8)),
            SetIterNext => sig(&[Ptr, Ptr, Ptr], Some(I8)),

            CollectionModcount => sig(&[Ptr], Some(I64)),

            ClosureNew => sig(&[I32, Ptr, I64], Some(Ptr)),
            BoxNew => sig(&[I32, I64], Some(Ptr)),
            IfaceNew => sig(&[Ptr, Ptr], Some(Ptr)),

            // Doc thi tra ve object hoac null, ghi thi tra ve co/khong. Cai
            // nao hong thi de loi lai o pump_os_error chu khong panic, vi
            // ben Pump con phai `fail` duoc.
            ReadFileText | ReadFileBytes => sig(&[Ptr], Some(Ptr)),
            WriteFileText | WriteFileBytes => sig(&[Ptr, Ptr], Some(I8)),
            OsArgs | OsError => sig(&[], Some(Ptr)),
            // tra ve mot `int?`, tuc la cai hop chua ma thoat, hoac null khi
            // chuong trinh khong chay noi.
            OsRun => sig(&[Ptr, Ptr], Some(Ptr)),
        }
    }
}

// -- bam ten symbol

/// The separator between the fields of a mangled Pump symbol.
pub const MANGLE_SEPARATOR: char = '$';

/// Builds the symbol name for a compiled Pump function.
pub fn mangle_function(
    module_path: &[String],
    owner: Option<&str>,
    name: &str,
    type_arguments: &str,
) -> String {
    format!(
        "pump{sep}{module}{sep}{owner}{sep}{name}{sep}{args}",
        sep = MANGLE_SEPARATOR,
        module = module_path.join("."),
        owner = owner.unwrap_or(""),
        name = name,
        args = type_arguments,
    )
}

/// Builds the symbol name for the itable that lets `concrete` satisfy
/// `interface`.
pub fn mangle_itable(
    interface_module: &[String],
    interface_name: &str,
    concrete_module: &[String],
    concrete_name: &str,
) -> String {
    format!(
        "pumpvt{sep}{im}.{iname}{sep}{cm}.{cname}",
        sep = MANGLE_SEPARATOR,
        im = interface_module.join("."),
        iname = interface_name,
        cm = concrete_module.join("."),
        cname = concrete_name,
    )
}

/// Encodes a resolved type into a symbol-safe string, for monomorphised
/// symbol names.
pub fn mangle_type(context: &TypeContext, ty: TypeId, out: &mut String) {
    use std::fmt::Write as _;

    let ty = context.shallow_resolve(ty);
    match context.kind(ty) {
        TypeKind::Bool => out.push('b'),
        TypeKind::Int | TypeKind::UntypedInt => out.push('i'),
        TypeKind::Uint => out.push('u'),
        TypeKind::Float | TypeKind::UntypedFloat => out.push('f'),
        TypeKind::Char => out.push('c'),
        TypeKind::String => out.push('s'),
        TypeKind::Void | TypeKind::Never | TypeKind::Error => out.push('v'),
        TypeKind::Array(element) => {
            out.push('A');
            mangle_type(context, *element, out);
        }
        TypeKind::Map { key, value } => {
            out.push('M');
            mangle_type(context, *key, out);
            mangle_type(context, *value, out);
        }
        TypeKind::Set(element) => {
            out.push('S');
            mangle_type(context, *element, out);
        }
        TypeKind::Tuple(elements) => {
            let _ = write!(out, "T{}", elements.len());
            for &element in elements {
                mangle_type(context, element, out);
            }
        }
        TypeKind::Optional(inner) => {
            out.push('O');
            mangle_type(context, *inner, out);
        }
        TypeKind::Failable(inner) => {
            out.push('E');
            mangle_type(context, *inner, out);
        }
        TypeKind::Function(signature) => {
            let _ = write!(out, "F{}", signature.params.len());
            for &param in &signature.params {
                mangle_type(context, param, out);
            }
            mangle_type(context, signature.ret, out);
        }
        TypeKind::Named { def, args } => {
            let name = &context.def(*def).name;
            let _ = write!(out, "N{}{}{}", name.len(), name, args.len());
            for &arg in args {
                mangle_type(context, arg, out);
            }
        }
        TypeKind::Generic(_) | TypeKind::Var(_) => {
            // mono the het may cai nay di truoc khi dat ten symbol. Toi
            // duoc day tuc la lower bi bug.
            out.push('?');
        }
    }
}

/// Encodes a list of type arguments for `mangle_function`.
pub fn mangle_type_arguments(context: &TypeContext, arguments: &[TypeId]) -> String {
    let mut out = String::new();
    for &argument in arguments {
        mangle_type(context, argument, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeContext;

    #[test]
    fn the_header_is_sixteen_bytes_and_the_payload_is_aligned() {
        assert_eq!(HEADER_SIZE, 16);
        assert_eq!(HEADER_SIZE % OBJECT_ALIGN, 0);
        assert_eq!(HEADER_TYPE_ID_OFFSET, 0);
        assert_eq!(HEADER_FLAGS_OFFSET, 4);
        assert_eq!(HEADER_SIZE_OFFSET, 8);
    }

    #[test]
    fn builtin_layouts_match_the_document() {
        assert_eq!(string::LENGTH_OFFSET, 16);
        assert_eq!(string::HASH_OFFSET, 24);
        assert_eq!(string::BYTES_OFFSET, 32);

        assert_eq!(array::LENGTH_OFFSET, 16);
        assert_eq!(array::CAPACITY_OFFSET, 24);
        assert_eq!(array::DATA_OFFSET, 32);
        assert_eq!(array::MODCOUNT_OFFSET, 40);
        assert_eq!(array::SIZE, 48);

        assert_eq!(map::SIZE, 80);
        assert_eq!(map::ENTRY_SIZE, 24);

        assert_eq!(interface::UNALIGNED_SIZE, 32);
        assert_eq!(enumeration::TAG_OFFSET, 16);
        assert_eq!(enumeration::PAYLOAD_OFFSET, 24);
        assert_eq!(itable_method_offset(0), 24);
        assert_eq!(itable_method_offset(3), 48);
    }

    #[test]
    fn record_layout_respects_declaration_order_and_alignment() {
        let context = TypeContext::new();
        // struct { flag: bool, ch: char, count: int, name: string }
        let layout = layout_struct(
            &context,
            &[TypeId::BOOL, TypeId::CHAR, TypeId::INT, TypeId::STRING],
        );
        assert_eq!(layout.fields[0].offset, 16);
        assert_eq!(layout.fields[1].offset, 20);
        assert_eq!(layout.fields[2].offset, 24);
        assert_eq!(layout.fields[3].offset, 32);
        assert_eq!(layout.ref_offsets, vec![32]);
        assert_eq!(layout.size, 48);
    }

    #[test]
    fn an_empty_struct_is_just_a_header() {
        let context = TypeContext::new();
        let layout = layout_struct(&context, &[]);
        assert_eq!(layout.size, 16);
        assert!(layout.ref_offsets.is_empty());
    }

    #[test]
    fn every_runtime_symbol_is_distinct() {
        let mut names: Vec<&str> = RuntimeFn::ALL.iter().map(|f| f.symbol()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate runtime symbol name");
    }

    #[test]
    fn mangling_is_five_fields() {
        let name = mangle_function(&["app".to_string()], Some("User"), "greet", "");
        assert_eq!(name, "pump$app$User$greet$");
        assert_eq!(name.matches(MANGLE_SEPARATOR).count(), 4);
    }

    #[test]
    fn type_mangling_is_prefix_free() {
        let mut context = TypeContext::new();
        let array_of_string = context.array_of(TypeId::STRING);
        let map = context.map_of(TypeId::INT, array_of_string);
        assert_eq!(
            mangle_type_arguments(&context, &[map, TypeId::BOOL]),
            "MiAsb"
        );
    }
}
