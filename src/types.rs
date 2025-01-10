// kieu da giai xong, kem may bang khai bao ma resolver dien vao va ai
// dung sau cung doc.
//
// kieu duoc intern. TypeId chi la mot so nho Copy duoc, hai kieu giong nhau
// ve cau truc thi luon cung mot id, nen so sanh hai kieu chi la `==`. Rieng
// bien suy dien la ngoai le, chung co y khac nhau va giai qua substitution.
//
// TypeContext giu: bo intern, bo substitution, bang khai bao kieu tra theo
// DefId, va bang ham/method tra theo FuncId.
//
// obj -> type

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::ast::VisibilityKind;
use crate::token::Span;

/// Id of one module. Mot module la mot file, 10.2.7.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ModuleId(pub u32);

impl ModuleId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Id of one struct, enum or interface.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DefId(pub u32);

impl DefId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Id of one function, method, or interface method signature.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FuncId(pub u32);

impl FuncId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An interned type. So sanh hai cai chi can `==`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeId(pub u32);

impl TypeId {
    // kieu co ban chiem cac o co dinh, intern san boi TypeContext::new, the
    // nen goi ten chung bang hang duoc
    pub const ERROR: TypeId = TypeId(0);
    pub const VOID: TypeId = TypeId(1);
    pub const NEVER: TypeId = TypeId(2);
    pub const BOOL: TypeId = TypeId(3);
    pub const INT: TypeId = TypeId(4);
    pub const UINT: TypeId = TypeId(5);
    pub const FLOAT: TypeId = TypeId(6);
    pub const CHAR: TypeId = TypeId(7);
    pub const STRING: TypeId = TypeId(8);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An inference variable. Checker de ra, unify giai.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeVar(pub u32);

impl TypeVar {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A bound generic parameter, e.g.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GenericId {
    pub owner: GenericOwner,
    pub index: u32,
}

/// Who the generic paramter belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum GenericOwner {
    Type(DefId),
    Func(FuncId),
}

/// Shape of a type.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeKind {
    Error,
    Void,
    Never,

    Bool,
    Int,
    Uint,
    Float,
    Char,
    String,

    Array(TypeId),
    Map { key: TypeId, value: TypeId },
    Set(TypeId),
    Tuple(Vec<TypeId>),

    Optional(TypeId),
    Failable(TypeId),

    Function(FnType),

    Named { def: DefId, args: Vec<TypeId> },

    Generic(GenericId),

    Var(TypeVar),

    UntypedInt,
    UntypedFloat,
}

/// Type of a function value. Khong co ten tham so, khong co default.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FnType {
    pub params: Vec<TypeId>,
    pub variadic: Option<TypeId>,
    pub ret: TypeId,
    pub failable: bool,
}

// -- khai bao

/// A struct, an enum, or an interface.
#[derive(Clone, Debug)]
pub struct TypeDef {
    pub id: DefId,
    pub name: String,
    pub module: ModuleId,
    pub visibility: VisibilityKind,
    pub generics: Vec<GenericParamDef>,
    // hoi truoc cai nay ten la Obj nen field moi la obj_kind. Doi ten het thi
    // phai sua nam sau cho, luoi.
    pub obj_kind: TypeDefKind,
    pub span: Span,
}

impl TypeDef {
    pub fn is_generic(&self) -> bool {
        !self.generics.is_empty()
    }

    pub fn as_struct(&self) -> Option<&StructDef> {
        match &self.obj_kind {
            TypeDefKind::Struct(def) => Some(def),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<&EnumDef> {
        match &self.obj_kind {
            TypeDefKind::Enum(def) => Some(def),
            _ => None,
        }
    }

    pub fn as_interface(&self) -> Option<&InterfaceDef> {
        match &self.obj_kind {
            TypeDefKind::Interface(def) => Some(def),
            _ => None,
        }
    }

    /// Method khai bao thang tren cai nay, theo thu tu source.
    pub fn methods(&self) -> &[FuncId] {
        match &self.obj_kind {
            TypeDefKind::Struct(def) => &def.methods,
            TypeDefKind::Enum(def) => &def.methods,
            TypeDefKind::Interface(def) => &def.methods,
        }
    }
}

#[derive(Clone, Debug)]
pub enum TypeDefKind {
    Struct(StructDef),
    Enum(EnumDef),
    Interface(InterfaceDef),
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub fields: Vec<FieldDef>,
    pub methods: Vec<FuncId>,
}

impl StructDef {
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|field| field.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeId,
    pub visibility: VisibilityKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub variants: Vec<VariantDef>,
    pub methods: Vec<FuncId>,
}

impl EnumDef {
    pub fn variant_index(&self, name: &str) -> Option<usize> {
        self.variants
            .iter()
            .position(|variant| variant.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub payload: Vec<TypeId>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct InterfaceDef {
    pub methods: Vec<FuncId>,
}

/// One generic paramter as declared, with its bounds.
#[derive(Clone, Debug)]
pub struct GenericParamDef {
    pub name: String,
    pub bounds: Vec<DefId>,
    pub span: Span,
}

/// A function, a method, or the signature of an interface method.
#[derive(Clone, Debug)]
pub struct FuncDef {
    pub id: FuncId,
    pub name: String,
    pub module: ModuleId,
    pub owner: Option<DefId>,
    pub visibility: VisibilityKind,
    pub generics: Vec<GenericParamDef>,
    pub params: Vec<ParamDef>,
    pub ret: TypeId,
    pub failable: bool,
    pub has_receiver: bool,
    pub has_body: bool,
    pub span: Span,
}
