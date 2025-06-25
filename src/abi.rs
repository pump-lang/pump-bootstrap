// ABI. Hinh dang ...
//
// file nay va docs/abi.md la cung mot tai lieu viet ra hai lan. clif.rs lam
// mot nua, runtime/src/gc.rs lam nua kia. Hai nua ma noi khac nhau la
// segfault, ma segfault kieu do khong lan ra duoc la loi cua ai. Sua thi sua
// ca hai, khong thi dung dung vao.
//
// RIENG file nay t co y ghi chu thich bang tieng Anh, de doi chieu tung dong
// voi docs/abi.md cho de. May file khac thi khong can.
//
// x86_64-pc-windows-msvc, little endian, ...
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
