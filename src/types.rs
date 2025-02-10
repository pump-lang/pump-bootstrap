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

impl FuncDef {
    pub fn is_generic(&self) -> bool {
        !self.generics.is_empty()
    }

    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|param| param.name == name)
    }

    pub fn variadic_index(&self) -> Option<usize> {
        self.params.iter().position(|param| param.variadic)
    }

    /// So tham so dau tien bat buoc phai truyen, theo vi tri hoac theo ten.
    pub fn required_param_count(&self) -> usize {
        self.params
            .iter()
            .filter(|param| param.default.is_none() && !param.variadic)
            .count()
    }
}

#[derive(Clone, Debug)]
pub struct ParamDef {
    pub name: String,
    pub ty: TypeId,
    pub default: Option<ConstValue>,
    pub variadic: bool,
    pub span: Span,
}

/// A value worked out at compile time: paramter default, module constant,
/// or a constant expression already folded. 12.6.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Char(char),
    Str(String),
    Null,
    Array(Vec<ConstValue>),
    Tuple(Vec<ConstValue>),
    Map(Vec<(ConstValue, ConstValue)>),
    Set(Vec<ConstValue>),
    EnumVariant { def: DefId, variant: u32 },
}

// -- cai context

/// Bo intern, bo substitution, va tat ca cac bang khai bao.
#[derive(Clone, Debug)]
pub struct TypeContext {
    kinds: Vec<TypeKind>,
    interned: HashMap<TypeKind, TypeId>,
    // dinh de thang cai Vec o day, nhung bind_var thi can &mut ma cho goi no
    // gan nhu bao gio cung dang cam san mot cai &self.kind(...), borrow
    // checker keu suot ca buoi. Boc Rc<RefCell<>> vao la het keu.
    //
    // CHU Y: TypeContext van derive Clone, ma Rc thi clone ra la DUNG CHUNG
    // chu khong phai copy ra cai moi. Hien khong cho nao clone context ca nen
    // khong sao, nhung ai dinh clone thi doc dong nay truoc.
    substitution: Rc<RefCell<Vec<Option<TypeId>>>>,
    defs: Vec<TypeDef>,
    funcs: Vec<FuncDef>,
    modules: Vec<Vec<String>>,
}

impl Default for TypeContext {
    fn default() -> TypeContext {
        TypeContext::new()
    }
}

impl TypeContext {
    /// Make a context with the primitive types already interned at the
    /// fixed slots that the TypeId constants name.
    pub fn new() -> TypeContext {
        let mut context = TypeContext {
            kinds: Vec::new(),
            interned: HashMap::new(),
            substitution: Rc::new(RefCell::new(Vec::new())),
            defs: Vec::new(),
            funcs: Vec::new(),
            modules: Vec::new(),
        };

        let primitives = [
            (TypeKind::Error, TypeId::ERROR),
            (TypeKind::Void, TypeId::VOID),
            (TypeKind::Never, TypeId::NEVER),
            (TypeKind::Bool, TypeId::BOOL),
            (TypeKind::Int, TypeId::INT),
            (TypeKind::Uint, TypeId::UINT),
            (TypeKind::Float, TypeId::FLOAT),
            (TypeKind::Char, TypeId::CHAR),
            (TypeKind::String, TypeId::STRING),
        ];
        for (kind, expected) in primitives {
            let id = context.intern(kind);
            debug_assert_eq!(id, expected, "primitive type slots are fixed");
        }
        context
    }

    // ---- interning ----

    pub fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(&id) = self.interned.get(&kind) {
            return id;
        }
        let id = TypeId(self.kinds.len() as u32);
        self.kinds.push(kind.clone());
        self.interned.insert(kind, id);
        id
    }

    pub fn kind(&self, ty: TypeId) -> &TypeKind {
        &self.kinds[ty.index()]
    }

    pub fn array_of(&mut self, element: TypeId) -> TypeId {
        self.intern(TypeKind::Array(element))
    }

    pub fn map_of(&mut self, key: TypeId, value: TypeId) -> TypeId {
        self.intern(TypeKind::Map { key, value })
    }

    pub fn set_of(&mut self, element: TypeId) -> TypeId {
        self.intern(TypeKind::Set(element))
    }

    pub fn tuple_of(&mut self, elements: Vec<TypeId>) -> TypeId {
        self.intern(TypeKind::Tuple(elements))
    }

    pub fn optional_of(&mut self, inner: TypeId) -> TypeId {
        self.intern(TypeKind::Optional(inner))
    }

    pub fn failable_of(&mut self, inner: TypeId) -> TypeId {
        self.intern(TypeKind::Failable(inner))
    }

    pub fn named(&mut self, def: DefId, args: Vec<TypeId>) -> TypeId {
        self.intern(TypeKind::Named { def, args })
    }

    pub fn function(&mut self, signature: FnType) -> TypeId {
        self.intern(TypeKind::Function(signature))
    }

    // ---- inference variables ----

    pub fn fresh_var(&mut self) -> TypeId {
        let var = TypeVar(self.substitution.borrow().len() as u32);
        self.substitution.borrow_mut().push(None);
        self.intern(TypeKind::Var(var))
    }

    /// Bind an inference variable to a type.
    // &self chu khong phai &mut self, chinh la ly do co cai RefCell. Dung giu
    // mot cai borrow() nao dang song ma goi vao day, khong thi panic.
    pub fn bind_var(&self, var: TypeVar, ty: TypeId) {
        debug_assert!(
            self.substitution.borrow()[var.index()].is_none(),
            "an inference variable is bound at most once"
        );
        self.substitution.borrow_mut()[var.index()] = Some(ty);
    }

    pub fn var_binding(&self, var: TypeVar) -> Option<TypeId> {
        self.substitution.borrow()[var.index()]
    }

    /// Follow the substitution, chi mot lop ngoai cung thoi.
    pub fn shallow_resolve(&self, mut ty: TypeId) -> TypeId {
        while let TypeKind::Var(var) = self.kind(ty) {
            // lay ra bien rieng roi tha borrow ngay. Match thang tren
            // borrow() la giu no song qua ca than match.
            let bound = self.substitution.borrow()[var.index()];
            match bound {
                Some(bound) => ty = bound,
                None => break,
            }
        }
        ty
    }

    /// Follow the substitution everywhere and build the type again.
    pub fn resolve(&mut self, ty: TypeId) -> TypeId {
        let ty = self.shallow_resolve(ty);
        match self.kind(ty).clone() {
            TypeKind::Array(element) => {
                let element = self.resolve(element);
                self.array_of(element)
            }
            TypeKind::Map { key, value } => {
                let key = self.resolve(key);
                let value = self.resolve(value);
                self.map_of(key, value)
            }
            TypeKind::Set(element) => {
                let element = self.resolve(element);
                self.set_of(element)
            }
            TypeKind::Tuple(elements) => {
                let elements = elements.into_iter().map(|e| self.resolve(e)).collect();
                self.tuple_of(elements)
            }
            TypeKind::Optional(inner) => {
                let inner = self.resolve(inner);
                self.optional_of(inner)
            }
            TypeKind::Failable(inner) => {
                let inner = self.resolve(inner);
                self.failable_of(inner)
            }
            TypeKind::Function(signature) => {
                let params = signature.params.iter().map(|&p| self.resolve(p)).collect();
                let variadic = signature.variadic.map(|v| self.resolve(v));
                let ret = self.resolve(signature.ret);
                self.function(FnType {
                    params,
                    variadic,
                    ret,
                    failable: signature.failable,
                })
            }
            TypeKind::Named { def, args } => {
                let args = args.into_iter().map(|a| self.resolve(a)).collect();
                self.named(def, args)
            }
            _ => ty,
        }
    }

    /// True if var shows up anywhere inside ty. Buoc vao thi thanh kieu vo
    /// han, phai chan.
    pub fn occurs_in(&self, var: TypeVar, ty: TypeId) -> bool {
        let ty = self.shallow_resolve(ty);
        match self.kind(ty) {
            TypeKind::Var(other) => *other == var,
            TypeKind::Array(element) | TypeKind::Set(element) => self.occurs_in(var, *element),
            TypeKind::Map { key, value } => {
                self.occurs_in(var, *key) || self.occurs_in(var, *value)
            }
            TypeKind::Tuple(elements) => elements.iter().any(|&e| self.occurs_in(var, e)),
            TypeKind::Optional(inner) | TypeKind::Failable(inner) => self.occurs_in(var, *inner),
            TypeKind::Function(signature) => {
                signature.params.iter().any(|&p| self.occurs_in(var, p))
                    || signature.variadic.is_some_and(|v| self.occurs_in(var, v))
                    || self.occurs_in(var, signature.ret)
            }
            TypeKind::Named { args, .. } => args.iter().any(|&a| self.occurs_in(var, a)),
            _ => false,
        }
    }

    /// True while a generic paramter is still somewhere inside ty, tuc la
    /// no van la khuon cho mono chu chua phai ban that.
    pub fn mentions_generic(&self, ty: TypeId) -> bool {
        let ty = self.shallow_resolve(ty);
        match self.kind(ty) {
            TypeKind::Generic(_) => true,
            TypeKind::Array(element) | TypeKind::Set(element) => self.mentions_generic(*element),
            TypeKind::Map { key, value } => {
                self.mentions_generic(*key) || self.mentions_generic(*value)
            }
            TypeKind::Tuple(elements) => elements.iter().any(|&e| self.mentions_generic(e)),
            TypeKind::Optional(inner) | TypeKind::Failable(inner) => self.mentions_generic(*inner),
            TypeKind::Function(signature) => {
                signature.params.iter().any(|&p| self.mentions_generic(p))
                    || signature.variadic.is_some_and(|v| self.mentions_generic(v))
                    || self.mentions_generic(signature.ret)
            }
            TypeKind::Named { args, .. } => args.iter().any(|&a| self.mentions_generic(a)),
            _ => false,
        }
    }

    /// Swap every Generic belonging to owner for the matching args entry.
    pub fn substitute(&mut self, ty: TypeId, owner: GenericOwner, args: &[TypeId]) -> TypeId {
        match self.kind(ty).clone() {
            TypeKind::Generic(generic) if generic.owner == owner => args
                .get(generic.index as usize)
                .copied()
                .unwrap_or(TypeId::ERROR),
            TypeKind::Array(element) => {
                let element = self.substitute(element, owner, args);
                self.array_of(element)
            }
            TypeKind::Map { key, value } => {
                let key = self.substitute(key, owner, args);
                let value = self.substitute(value, owner, args);
                self.map_of(key, value)
            }
            TypeKind::Set(element) => {
                let element = self.substitute(element, owner, args);
                self.set_of(element)
            }
            TypeKind::Tuple(elements) => {
                let elements = elements
                    .into_iter()
                    .map(|e| self.substitute(e, owner, args))
                    .collect();
                self.tuple_of(elements)
            }
            TypeKind::Optional(inner) => {
                let inner = self.substitute(inner, owner, args);
                self.optional_of(inner)
            }
            TypeKind::Failable(inner) => {
                let inner = self.substitute(inner, owner, args);
                self.failable_of(inner)
            }
            TypeKind::Function(signature) => {
                let params = signature
                    .params
                    .iter()
                    .map(|&p| self.substitute(p, owner, args))
                    .collect();
                let variadic = signature.variadic.map(|v| self.substitute(v, owner, args));
                let ret = self.substitute(signature.ret, owner, args);
                self.function(FnType {
                    params,
                    variadic,
                    ret,
                    failable: signature.failable,
                })
            }
            TypeKind::Named { def, args: params } => {
                let params = params
                    .into_iter()
                    .map(|p| self.substitute(p, owner, args))
                    .collect();
                self.named(def, params)
            }
            _ => ty,
        }
    }

    // ---- bang khai bao ----

    pub fn add_module(&mut self, path: Vec<String>) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        self.modules.push(path);
        id
    }

    pub fn module_path(&self, module: ModuleId) -> &[String] {
        &self.modules[module.index()]
    }

    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Xi truoc mot DefId khi chua biet than cua no, de hai kieu goi nhau
    /// vong tron van tro duoc den nhau.
    pub fn add_def(&mut self, def: TypeDef) -> DefId {
        let id = DefId(self.defs.len() as u32);
        debug_assert_eq!(def.id, id, "a TypeDef must be built with its own DefId");
        self.defs.push(def);
        id
    }

    pub fn next_def_id(&self) -> DefId {
        DefId(self.defs.len() as u32)
    }

    pub fn def(&self, id: DefId) -> &TypeDef {
        let obj = &self.defs[id.index()];
        obj
    }

    pub fn def_mut(&mut self, id: DefId) -> &mut TypeDef {
        &mut self.defs[id.index()]
    }

    pub fn defs(&self) -> &[TypeDef] {
        &self.defs
    }

    pub fn add_func(&mut self, func: FuncDef) -> FuncId {
        let id = FuncId(self.funcs.len() as u32);
        debug_assert_eq!(func.id, id, "a FuncDef must be built with its own FuncId");
        self.funcs.push(func);
        id
    }

    pub fn next_func_id(&self) -> FuncId {
        FuncId(self.funcs.len() as u32)
    }

    pub fn func(&self, id: FuncId) -> &FuncDef {
        &self.funcs[id.index()]
    }

    pub fn func_mut(&mut self, id: FuncId) -> &mut FuncDef {
        &mut self.funcs[id.index()]
    }

    pub fn funcs(&self) -> &[FuncDef] {
        &self.funcs
    }

    /// Find a method by name on a definition, theo thu tu khai bao.
    pub fn find_method(&self, def: DefId, name: &str) -> Option<FuncId> {
        self.def(def)
            .methods()
            .iter()
            .copied()
            .find(|&id| self.func(id).name == name)
    }

    // ---- may cau hoi dung/sai ----

    /// True for types that are a pointer to a GC object at runtime.
    pub fn is_reference(&self, ty: TypeId) -> bool {
        match self.kind(self.shallow_resolve(ty)) {
            TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Uint
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::UntypedInt
            | TypeKind::UntypedFloat
            | TypeKind::Void
            | TypeKind::Never
            | TypeKind::Error => false,
            TypeKind::String
            | TypeKind::Array(_)
            | TypeKind::Map { .. }
            | TypeKind::Set(_)
            | TypeKind::Tuple(_)
            | TypeKind::Optional(_)
            | TypeKind::Function(_)
            | TypeKind::Named { .. } => true,
            // kieu co the loi khong bao gio la gia tri, no chi song tren chu ky
            TypeKind::Failable(_) => false,
            // tham so generic chi con thay duoc sau mono, ma toi luc do thi
            // chung the het roi
            TypeKind::Generic(_) | TypeKind::Var(_) => false,
        }
    }

    pub fn is_numeric(&self, ty: TypeId) -> bool {
        matches!(
            self.kind(self.shallow_resolve(ty)),
            TypeKind::Int
                | TypeKind::Uint
                | TypeKind::Float
                | TypeKind::UntypedInt
                | TypeKind::UntypedFloat
        )
    }

    pub fn is_integer(&self, ty: TypeId) -> bool {
        matches!(
            self.kind(self.shallow_resolve(ty)),
            TypeKind::Int | TypeKind::Uint | TypeKind::UntypedInt
        )
    }

    /// True for types allowed as a map key or a set element.
    pub fn is_hashable(&self, ty: TypeId) -> bool {
        match self.kind(self.shallow_resolve(ty)) {
            TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Uint
            | TypeKind::Char
            | TypeKind::String
            | TypeKind::UntypedInt => true,
            TypeKind::Tuple(elements) => elements.iter().all(|&e| self.is_hashable(e)),
            TypeKind::Named { def, .. } => self
                .def(*def)
                .as_enum()
                .is_some_and(|e| e.variants.iter().all(|v| v.payload.is_empty())),
            _ => false,
        }
    }

    pub fn is_interface(&self, ty: TypeId) -> bool {
        match self.kind(self.shallow_resolve(ty)) {
            TypeKind::Named { def, .. } => self.def(*def).as_interface().is_some(),
            _ => false,
        }
    }

    /// Kieu mac dinh ma mot literal chua co kieu se nhan khi khong co gi de
    /// dua vao. 3.5.1.
    pub fn default_of(&self, ty: TypeId) -> TypeId {
        match self.kind(self.shallow_resolve(ty)) {
            TypeKind::UntypedInt => TypeId::INT,
            TypeKind::UntypedFloat => TypeId::FLOAT,
            _ => self.shallow_resolve(ty),
        }
    }

    // ---- in ra ----

    /// Print a type back the way the user wrote it, for error messages.
    pub fn display(&self, ty: TypeId) -> String {
        let mut out = String::new();
        self.write_type(&mut out, ty);
        out
    }

    fn write_type(&self, out: &mut String, ty: TypeId) {
        let ty = self.shallow_resolve(ty);
        match self.kind(ty) {
            TypeKind::Error => out.push_str("<error>"),
            TypeKind::Void => out.push_str("void"),
            TypeKind::Never => out.push_str("<never>"),
            TypeKind::Bool => out.push_str("bool"),
            TypeKind::Int | TypeKind::UntypedInt => out.push_str("int"),
            TypeKind::Uint => out.push_str("uint"),
            TypeKind::Float | TypeKind::UntypedFloat => out.push_str("float"),
            TypeKind::Char => out.push_str("char"),
            TypeKind::String => out.push_str("string"),
            TypeKind::Array(element) => {
                out.push('[');
                self.write_type(out, *element);
                out.push(']');
            }
            TypeKind::Map { key, value } => {
                out.push('[');
                self.write_type(out, *key);
                out.push_str(": ");
                self.write_type(out, *value);
                out.push(']');
            }
            TypeKind::Set(element) => {
                out.push_str("set<");
                self.write_type(out, *element);
                out.push('>');
            }
            TypeKind::Tuple(elements) => {
                out.push('(');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    self.write_type(out, *element);
                }
                out.push(')');
            }
            TypeKind::Optional(inner) => {
                self.write_type(out, *inner);
                out.push('?');
            }
            TypeKind::Failable(inner) => {
                self.write_type(out, *inner);
                out.push('!');
            }
            TypeKind::Function(signature) => {
                out.push_str("fn(");
                for (index, param) in signature.params.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    self.write_type(out, *param);
                }
                if let Some(variadic) = signature.variadic {
                    if !signature.params.is_empty() {
                        out.push_str(", ");
                    }
                    out.push_str("...");
                    self.write_type(out, variadic);
                }
                out.push(')');
                if signature.ret != TypeId::VOID || signature.failable {
                    out.push_str(": ");
                    self.write_type(out, signature.ret);
                    if signature.failable {
                        out.push('!');
                    }
                }
            }
            TypeKind::Named { def, args } => {
                out.push_str(&self.def(*def).name);
                if !args.is_empty() {
                    out.push('<');
                    for (index, arg) in args.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        self.write_type(out, *arg);
                    }
                    out.push('>');
                }
            }
            TypeKind::Generic(generic) => {
                let name = match generic.owner {
                    GenericOwner::Type(def) => self
                        .def(def)
                        .generics
                        .get(generic.index as usize)
                        .map(|g| g.name.as_str()),
                    GenericOwner::Func(func) => self
                        .func(func)
                        .generics
                        .get(generic.index as usize)
                        .map(|g| g.name.as_str()),
                };
                out.push_str(name.unwrap_or("?"));
            }
            TypeKind::Var(var) => {
                let _ = write!(out, "?{}", var.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_land_on_their_fixed_slots() {
        let context = TypeContext::new();
        assert!(matches!(context.kind(TypeId::INT), TypeKind::Int));
        assert!(matches!(context.kind(TypeId::STRING), TypeKind::String));
        assert!(matches!(context.kind(TypeId::VOID), TypeKind::Void));
    }

    #[test]
    fn interning_makes_equal_types_identical() {
        let mut context = TypeContext::new();
        let a = context.array_of(TypeId::INT);
        let b = context.array_of(TypeId::INT);
        assert_eq!(a, b);
        assert_eq!(context.display(a), "[int]");
    }

    #[test]
    fn resolve_follows_the_substitution() {
        let mut context = TypeContext::new();
        let var = context.fresh_var();
        let TypeKind::Var(handle) = *context.kind(var) else {
            panic!("fresh_var must produce a variable");
        };
        let array = context.array_of(var);
        context.bind_var(handle, TypeId::STRING);
        let resolved = context.resolve(array);
        assert_eq!(context.display(resolved), "[string]");
    }

    #[test]
    fn a_generic_is_visible_through_a_composite() {
        // mono dung cai nay de phan biet khuon voi ban that: doi so kieu ma
        // van con la ten mot tham so thi khong phai ban that
        let mut context = TypeContext::new();
        let generic = context.intern(TypeKind::Generic(GenericId {
            owner: GenericOwner::Func(FuncId(0)),
            index: 0,
        }));
        let optional = context.optional_of(generic);
        let nested = context.array_of(optional);
        let concrete = context.array_of(TypeId::INT);
        assert!(context.mentions_generic(nested));
        assert!(!context.mentions_generic(concrete));
    }

    #[test]
    fn float_is_not_hashable() {
        let context = TypeContext::new();
        assert!(context.is_hashable(TypeId::STRING));
        assert!(!context.is_hashable(TypeId::FLOAT));
    }
}
