// lower. AST da check xong bien thanh mot ir::Program.
//
// day la cho ngon ngu dung lai va may bat dau, nen moi thu frontend da biet
// ma backend khong phai nghi lai deu chot o day:
//
//  * mono - moi ban duoc dung toi la mot ham IR, worklist moi tu main, tu
//    danh sach ban da ghi lai, va tu tung itable,
//  * layout - offset cua field, payload, phan tu bien thanh so tran,
//  * type descriptor - moi hinh dang runtime khac nhau mot cai. Sai
//    ref_offsets la GC no do, nen phai dung crate::abi ma dung, tuyet doi
//    khong go tay,
//  * itable - moi conformance mot cai, method xep theo dung thu tu cua
//    interface,
//  * duong hoa - vong for, noi suy chuoi thanh chuoi concat, catch thanh
//    nhanh re theo o loi, `?` va `!` thanh return som, gan gop thanh
//    load-tinh-store, closure thanh capture cong mot con tro code,
//  * dong hop - bien bi capture thanh mot cai hop dung chung cho tat ca ai
//    capture no, va primitive optional cung thanh mot cai hop.
//
// Hai dieu tuyet doi khong duoc truot, ca hai lay tu docs/abi.md muc 22:
// khong con tro noi bo nao duoc song vat qua mot cho co the cap phat, va moi
// bien deu phai nam trong o stack de cai quet bao thu con thay. Nho de het
// bien trong o ma vong lap khong can tham so block nao ca; tham so block chi
// dung o cho nao SSA no tu nhien, kieu ket qua cua `&&` ngat som.

use std::collections::{HashMap, HashSet};

use crate::abi::{self, DescriptorKind, IrType, RuntimeFn, TypeDescriptor, VariantDescriptor};
use crate::ast::{
    Argument, AssignStmt, Block, CatchHandler, ClosureExpr, ConstDecl, Expr, ExprKind,
    FieldPattern, ForStmt, Ident, IfStmt, IrrefutablePattern, IrrefutablePatternKind, MapEntry,
    MatchArmBody, MatchStmt, NodeId, Param, ParamKind, Pattern, PatternKind, RangeEndpoint, Stmt,
    StmtKind, StringLit, StringPart, StructLit, WhileStmt,
};
use crate::ast::{BinaryOp as SourceBinaryOp, UnaryOp as SourceUnaryOp};
use crate::check::{
    BoundArgument, BuiltinMethod, Callee, Checked, ConformanceMethod, FieldAccess, ResolvedCall,
};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::{
    BinaryOp, BlockRef, CompareOp, ConvertOp, EnumSingleton, FuncRef, Function, Global, GlobalRef,
    InstKind, Itable, ItableRef, Program, Signature, SlotRef, Terminator, TypeIdx, UnaryOp, Value,
};
use crate::resolve::{GlobalConstId, LocalId, Predeclared, ValueBinding};
use crate::token::Span;
use crate::types::{
    ConstValue, DefId, FuncId, GenericOwner, ModuleId, TypeContext, TypeId, TypeKind,
};

const INSTANCE_LIMIT: usize = 20_000;

/// Lower a checked program down to IR.
pub fn lower(checked: &Checked) -> Result<Program, CompileError> {
    Lowerer::new(checked).run()
}

type InstanceKey = (FuncId, Vec<TypeId>);

#[derive(Clone, Debug, Default)]
struct Substitution {
    owner: Option<DefId>,
    owner_args: Vec<TypeId>,
    func: Option<FuncId>,
    func_args: Vec<TypeId>,
}

impl Substitution {
    fn is_empty(&self) -> bool {
        self.owner_args.is_empty() && self.func_args.is_empty()
    }
}

enum Job<'c> {
    Function {
        handle: FuncRef,
        func: FuncId,
        subst: Substitution,
    },
    Closure(Box<ClosureJob<'c>>),
    Thunk {
        handle: FuncRef,
        target: FuncRef,
    },
    BuiltinShim {
        handle: FuncRef,
        method: BuiltinMethod,
        concrete: TypeId,
    },
    ModuleInit {
        handle: FuncRef,
    },
}

struct ClosureJob<'c> {
    handle: FuncRef,
    expr: &'c ClosureExpr,
    ty: TypeId,
    captures: Vec<LocalId>,
    captured_this: Option<u32>,
    subst: Substitution,
    module: ModuleId,
}

struct Lowerer<'c> {
    checked: &'c Checked,
    types: TypeContext,
    program: Program,
    descriptors: HashMap<TypeId, TypeIdx>,
    instances: HashMap<InstanceKey, FuncRef>,
    thunks: HashMap<FuncRef, FuncRef>,
    shims: HashMap<(BuiltinMethod, TypeId), FuncRef>,
    itables: HashMap<(DefId, TypeId), ItableRef>,
    singletons: HashSet<(u32, u32)>,
    jobs: Vec<Job<'c>>,
    closure_counter: u32,
    errors: Vec<CompileError>,
}

impl<'c> Lowerer<'c> {
    fn new(checked: &'c Checked) -> Lowerer<'c> {
        Lowerer {
            checked,
            types: checked.context().clone(),
            program: Program::new(),
            descriptors: HashMap::new(),
            instances: HashMap::new(),
            thunks: HashMap::new(),
            shims: HashMap::new(),
            itables: HashMap::new(),
            singletons: HashSet::new(),
            jobs: Vec::new(),
            closure_counter: 0,
            errors: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Program, CompileError> {
        self.reserve_builtin_descriptors();
        self.declare_globals();

        let module_init = self.declare(
            abi::SYMBOL_MODULE_INIT.to_string(),
            Signature::new(Vec::new(), None),
            Span::synthetic(),
            false,
            true,
            TypeId::VOID,
        );
        self.program.module_init = Some(module_init);
        self.jobs.push(Job::ModuleInit {
            handle: module_init,
        });

        if let Some(main) = self.checked.resolution.entry {
            let handle = self.instance(main, Vec::new(), Span::synthetic());
            self.program.entry = Some(handle);
        }

        let checked = self.checked;
        for instantiation in &checked.instantiations {
            // loi goi nam trong than mot ham generic thi duoc ghi lai kem
            // tham so kieu cua chinh than do, ma may cai do khong chi vao ban
            // cu the nao ca. Lower se toi duoc ban that luc no di qua cac ban
            // cua ham bao ngoai, nen o day bo qua cai khuon.
            if instantiation
                .type_arguments
                .iter()
                .any(|&argument| self.types.mentions_generic(argument))
            {
                continue;
            }
            let span = checked.context().func(instantiation.func).span;
            self.instance(
                instantiation.func,
                instantiation.type_arguments.clone(),
                span,
            );
        }
        for conformance in &checked.conformances {
            if self.types.mentions_generic(conformance.concrete) {
                continue;
            }
            self.itable(
                conformance.interface,
                conformance.concrete,
                Span::synthetic(),
            );
        }

        while let Some(job) = self.jobs.pop() {
            self.lower_job(job);
        }

        match self.errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(self.program),
        }
    }

    fn report(&mut self, error: CompileError) {
        self.errors.push(error);
    }

    fn unsupported(&mut self, span: Span, what: impl Into<String>) {
        self.report(CompileError::at(
            ErrorCode::UnsupportedConstruct,
            span,
            what,
        ));
    }

    // ---- kieu ----

    fn ir_type(&self, ty: TypeId) -> IrType {
        match self.types.kind(self.types.shallow_resolve(ty)) {
            TypeKind::Bool => IrType::I8,
            TypeKind::Char => IrType::I8,
            TypeKind::Int | TypeKind::Uint | TypeKind::UntypedInt => IrType::I64,
            TypeKind::Float | TypeKind::UntypedFloat => IrType::F64,
            _ => IrType::Ptr,
        }
    }

    fn ir_return_type(&self, ty: TypeId) -> Option<IrType> {
        match self.types.kind(self.types.shallow_resolve(ty)) {
            TypeKind::Void => None,
            TypeKind::Failable(inner) => self.ir_return_type(*inner),
            _ => Some(self.ir_type(ty)),
        }
    }

    fn kind_of(&self, ty: TypeId) -> TypeKind {
        self.types.kind(self.types.shallow_resolve(ty)).clone()
    }

    fn apply(&mut self, ty: TypeId, subst: &Substitution) -> TypeId {
        let mut ty = self.types.resolve(ty);
        if subst.is_empty() {
            return ty;
        }
        if let Some(func) = subst.func {
            if !subst.func_args.is_empty() {
                ty = self
                    .types
                    .substitute(ty, GenericOwner::Func(func), &subst.func_args);
            }
        }
        if let Some(owner) = subst.owner {
            if !subst.owner_args.is_empty() {
                ty = self
                    .types
                    .substitute(ty, GenericOwner::Type(owner), &subst.owner_args);
            }
        }
        ty
    }

    fn substitute_owner(&mut self, ty: TypeId, def: DefId, args: &[TypeId]) -> TypeId {
        let ty = self.types.resolve(ty);
        if args.is_empty() {
            return ty;
        }
        self.types.substitute(ty, GenericOwner::Type(def), args)
    }

    fn named_parts(&self, ty: TypeId) -> Option<(DefId, Vec<TypeId>)> {
        match self.types.kind(self.types.shallow_resolve(ty)) {
            TypeKind::Named { def, args } => Some((*def, args.clone())),
            _ => None,
        }
    }

    fn struct_fields(&mut self, def: DefId, args: &[TypeId]) -> Vec<TypeId> {
        let fields: Vec<TypeId> = match self.checked.context().def(def).as_struct() {
            Some(structure) => structure.fields.iter().map(|field| field.ty).collect(),
            None => Vec::new(),
        };
        fields
            .into_iter()
            .map(|field| self.substitute_owner(field, def, args))
            .collect()
    }

    fn variant_payload(&mut self, def: DefId, args: &[TypeId], variant: usize) -> Vec<TypeId> {
        let payload: Vec<TypeId> = self
            .checked
            .context()
            .def(def)
            .as_enum()
            .and_then(|enumeration| enumeration.variants.get(variant))
            .map(|variant| variant.payload.clone())
            .unwrap_or_default();
        payload
            .into_iter()
            .map(|field| self.substitute_owner(field, def, args))
            .collect()
    }

    // ---- type descriptor ----

    fn reserve_builtin_descriptors(&mut self) {
        fn descriptor(
            type_id: u32,
            kind: DescriptorKind,
            flags: u32,
            size: u64,
            name: &str,
        ) -> TypeDescriptor {
            TypeDescriptor {
                type_id,
                kind,
                flags,
                size,
                ref_offsets: Vec::new(),
                variants: Vec::new(),
                name: name.to_string(),
            }
        }

        let boxed = abi::align_object_size(abi::boxed::UNALIGNED_SIZE);
        let mut table = vec![
            descriptor(abi::TYPE_ID_INVALID, DescriptorKind::Buffer, 0, 0, "<free>"),
            descriptor(abi::TYPE_ID_BUFFER, DescriptorKind::Buffer, 0, 0, "buffer"),
            descriptor(abi::TYPE_ID_STRING, DescriptorKind::String, 0, 0, "string"),
            descriptor(
                abi::TYPE_ID_BOX_SCALAR,
                DescriptorKind::Box,
                0,
                boxed,
                "box",
            ),
            descriptor(
                abi::TYPE_ID_BOX_REF,
                DescriptorKind::Box,
                abi::DESC_FLAG_ELEM_IS_REF,
                boxed,
                "box",
            ),
            descriptor(
                abi::TYPE_ID_INTERFACE,
                DescriptorKind::Interface,
                0,
                abi::align_object_size(abi::interface::UNALIGNED_SIZE),
                "interface",
            ),
        ];
        for type_id in table.len() as u32..abi::FIRST_USER_TYPE_ID {
            table.push(descriptor(
                type_id,
                DescriptorKind::Buffer,
                0,
                0,
                "<reserved>",
            ));
        }
        self.program.type_descriptors = table;
    }

    fn add_descriptor(&mut self, mut descriptor: TypeDescriptor) -> TypeIdx {
        let type_id = self.program.type_descriptors.len() as u32;
        descriptor.type_id = type_id;
        self.program.type_descriptors.push(descriptor);
        TypeIdx(type_id)
    }

    fn descriptor_for(&mut self, ty: TypeId) -> TypeIdx {
        let ty = self.types.resolve(ty);
        if let Some(&existing) = self.descriptors.get(&ty) {
            return existing;
        }
        // nhet mot cho trong vao truoc: struct nao co field tro ve chinh
        // kieu no qua mot collection thi khong the lai de quy mai mai.
        self.descriptors.insert(ty, TypeIdx(abi::TYPE_ID_INVALID));

        let handle = match self.kind_of(ty) {
            TypeKind::String => TypeIdx(abi::TYPE_ID_STRING),
            TypeKind::Optional(inner) => {
                if self.ir_type(inner).is_pointer() {
                    self.descriptor_for(inner)
                } else {
                    TypeIdx(abi::TYPE_ID_BOX_SCALAR)
                }
            }
            TypeKind::Array(element) => {
                let flags = self.reference_flag(element, abi::DESC_FLAG_ELEM_IS_REF);
                self.shape_descriptor(ty, DescriptorKind::Array, flags, abi::array::SIZE)
            }
            TypeKind::Map { key, value } => {
                let flags = self.reference_flag(key, abi::DESC_FLAG_KEY_IS_REF)
                    | self.reference_flag(value, abi::DESC_FLAG_VALUE_IS_REF);
                self.shape_descriptor(ty, DescriptorKind::Map, flags, abi::map::SIZE)
            }
            TypeKind::Set(element) => {
                // set giu phan tu o cot key cua mot object hinh map, nen
                // KEY_IS_REF moi la co chinh. ELEM_IS_REF di kem vi trong
                // source doc thi set la mot cai chua phan tu. GC nhan ca hai
                // (docs/abi.md 8).
                let flags = self.reference_flag(
                    element,
                    abi::DESC_FLAG_ELEM_IS_REF | abi::DESC_FLAG_KEY_IS_REF,
                );
                self.shape_descriptor(ty, DescriptorKind::Set, flags, abi::set::SIZE)
            }
            TypeKind::Tuple(elements) => {
                let layout = abi::layout_tuple(&self.types, &elements);
                let name = self.types.display(ty);
                self.add_descriptor(TypeDescriptor {
                    type_id: 0,
                    kind: DescriptorKind::Tuple,
                    flags: 0,
                    size: layout.size,
                    ref_offsets: layout.ref_offsets,
                    variants: Vec::new(),
                    name,
                })
            }
            // closure to nho bao nhieu la tuy so capture, nen GC doc kich
            // thuoc tu header cua object.
            TypeKind::Function(_) => self.shape_descriptor(ty, DescriptorKind::Closure, 0, 0),
            TypeKind::Named { def, args } => self.named_descriptor(ty, def, &args),
            _ => TypeIdx(abi::TYPE_ID_INVALID),
        };

        self.descriptors.insert(ty, handle);
        handle
    }

    fn reference_flag(&self, ty: TypeId, flag: u32) -> u32 {
        if self.ir_type(ty).is_pointer() {
            flag
        } else {
            0
        }
    }

    fn shape_descriptor(
        &mut self,
        ty: TypeId,
        kind: DescriptorKind,
        flags: u32,
        size: u64,
    ) -> TypeIdx {
        let name = self.types.display(ty);
        self.add_descriptor(TypeDescriptor {
            type_id: 0,
            kind,
            flags,
            size,
            ref_offsets: Vec::new(),
            variants: Vec::new(),
            name,
        })
    }

    fn named_descriptor(&mut self, ty: TypeId, def: DefId, args: &[TypeId]) -> TypeIdx {
        let name = self.types.display(ty);
        let definition = self.checked.context().def(def);

        if definition.as_interface().is_some() {
            return TypeIdx(abi::TYPE_ID_INTERFACE);
        }

        if definition.as_struct().is_some() {
            let fields = self.struct_fields(def, args);
            let layout = abi::layout_struct(&self.types, &fields);
            return self.add_descriptor(TypeDescriptor {
                type_id: 0,
                kind: DescriptorKind::Struct,
                flags: 0,
                size: layout.size,
                ref_offsets: layout.ref_offsets,
                variants: Vec::new(),
                name,
            });
        }

        let variant_names: Vec<String> = definition
            .as_enum()
            .map(|enumeration| {
                enumeration
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut variants = Vec::with_capacity(variant_names.len());
        for (index, variant_name) in variant_names.into_iter().enumerate() {
            let payload = self.variant_payload(def, args, index);
            let layout = abi::layout_variant(&self.types, &payload);
            variants.push(VariantDescriptor {
                ref_offsets: layout.ref_offsets,
                name: variant_name,
            });
        }
        self.add_descriptor(TypeDescriptor {
            type_id: 0,
            kind: DescriptorKind::Enum,
            flags: 0,
            // mot gia tri enum duoc cap dung bang variant cua no, nen GC
            // doc kich thuoc tu header chu khong lay o day.
            size: 0,
            ref_offsets: Vec::new(),
            variants,
            name,
        })
    }

    fn enum_singleton(&mut self, ty: TypeId, type_id: TypeIdx, tag: u32) {
        if !self.singletons.insert((type_id.0, tag)) {
            return;
        }
        let variant = self
            .named_parts(ty)
            .and_then(|(def, _)| {
                self.checked
                    .context()
                    .def(def)
                    .as_enum()
                    .and_then(|enumeration| enumeration.variants.get(tag as usize))
                    .map(|variant| variant.name.clone())
            })
            .unwrap_or_else(|| tag.to_string());
        let name = format!("pumpenum${}${}", self.types.display(ty), variant);
        self.program
            .enum_singletons
            .push(EnumSingleton { name, type_id, tag });
    }

    // ---- ham ----

    fn declare(
        &mut self,
        name: String,
        signature: Signature,
        span: Span,
        failable: bool,
        exported: bool,
        return_type: TypeId,
    ) -> FuncRef {
        let mut function = Function::new(name, signature, span);
        function.failable = failable;
        function.exported = exported;
        function.source_return_type = return_type;
        self.program.add_function(function)
    }

    fn physical_signature(&mut self, func: FuncId, subst: &Substitution) -> Signature {
        let definition = self.checked.context().func(func);
        let has_receiver = definition.has_receiver;
        let declared: Vec<(TypeId, bool)> = definition
            .params
            .iter()
            .map(|param| (param.ty, param.variadic))
            .collect();
        let ret = definition.ret;

        let mut params = Vec::with_capacity(declared.len() + 1);
        if has_receiver {
            params.push(IrType::Ptr);
        }
        for (ty, variadic) in declared {
            if variadic {
                // tham so `...T` toi noi la mot `[T]` vua duoc dung ra.
                params.push(IrType::Ptr);
            } else {
                let ty = self.apply(ty, subst);
                params.push(self.ir_type(ty));
            }
        }
        let ret = self.apply(ret, subst);
        Signature::new(params, self.ir_return_type(ret))
    }

    fn substitution_for(&self, func: FuncId, arguments: &[TypeId]) -> Substitution {
        let definition = self.checked.context().func(func);
        let owner_arity = definition
            .owner
            .map(|owner| self.checked.context().def(owner).generics.len())
            .unwrap_or(0)
            .min(arguments.len());

        Substitution {
            owner: definition.owner,
            owner_args: arguments[..owner_arity].to_vec(),
            func: Some(func),
            func_args: arguments[owner_arity..].to_vec(),
        }
    }

    fn instance(&mut self, func: FuncId, arguments: Vec<TypeId>, span: Span) -> FuncRef {
        let arguments: Vec<TypeId> = arguments
            .into_iter()
            .map(|argument| self.types.resolve(argument))
            .collect();
        let key = (func, arguments.clone());
        if let Some(&existing) = self.instances.get(&key) {
            return existing;
        }

        if self.instances.len() >= INSTANCE_LIMIT {
            if !self
                .errors
                .iter()
                .any(|error| error.code == ErrorCode::MonomorphisationDepthExceeded)
            {
                self.report(CompileError::at(
                    ErrorCode::MonomorphisationDepthExceeded,
                    span,
                    format!("more than {INSTANCE_LIMIT} generic instantiations were required"),
                ));
            }
            return self
                .instances
                .values()
                .copied()
                .next()
                .unwrap_or(FuncRef(0));
        }

        let subst = self.substitution_for(func, &arguments);
        let signature = self.physical_signature(func, &subst);
        let definition = self.checked.context().func(func);
        let owner_name = definition
            .owner
            .map(|owner| self.checked.context().def(owner).name.clone());
        let name = abi::mangle_function(
            self.checked.context().module_path(definition.module),
            owner_name.as_deref(),
            &definition.name,
            &abi::mangle_type_arguments(&self.types, &arguments),
        );
        let failable = definition.failable;
        let declaration_span = definition.span;
        let ret = definition.ret;
        let return_type = self.apply(ret, &subst);

        let handle = self.declare(
            name,
            signature,
            declaration_span,
            failable,
            false,
            return_type,
        );
        self.instances.insert(key, handle);
        self.jobs.push(Job::Function {
            handle,
            func,
            subst,
        });
        handle
    }

    fn thunk(&mut self, target: FuncRef) -> FuncRef {
        if let Some(&existing) = self.thunks.get(&target) {
            return existing;
        }
        let declared = self.program.function(target);
        let mut params = vec![IrType::Ptr];
        params.extend_from_slice(&declared.signature.params);
        let signature = Signature::new(params, declared.signature.ret);
        let name = format!("{}thunk", declared.name);
        let span = declared.span;
        let return_type = declared.source_return_type;
        let failable = declared.failable;

        let handle = self.declare(name, signature, span, failable, false, return_type);
        self.thunks.insert(target, handle);
        self.jobs.push(Job::Thunk { handle, target });
        handle
    }

    fn builtin_shim(&mut self, method: BuiltinMethod, concrete: TypeId) -> FuncRef {
        let concrete = self.types.resolve(concrete);
        if let Some(&existing) = self.shims.get(&(method, concrete)) {
            return existing;
        }
        let name = format!(
            "pumpbuiltin${}${}",
            self.types.display(concrete),
            method.spelling()
        );
        let handle = self.declare(
            name,
            Signature::new(vec![IrType::Ptr], Some(IrType::Ptr)),
            Span::synthetic(),
            false,
            false,
            TypeId::STRING,
        );
        self.shims.insert((method, concrete), handle);
        self.jobs.push(Job::BuiltinShim {
            handle,
            method,
            concrete,
        });
        handle
    }

    // ---- itable ----

    fn conformance_method(
        &mut self,
        interface: DefId,
        concrete: TypeId,
        slot: u32,
    ) -> Option<ConformanceMethod> {
        let concrete = self.types.resolve(concrete);
        let checked = self.checked;
        checked
            .conformances
            .iter()
            .find(|conformance| {
                conformance.interface == interface
                    && self.types.resolve(conformance.concrete) == concrete
            })
            .and_then(|conformance| conformance.methods.get(slot as usize).copied())
    }

    fn itable(&mut self, interface: DefId, concrete: TypeId, span: Span) -> Option<ItableRef> {
        let concrete = self.types.resolve(concrete);
        if let Some(&existing) = self.itables.get(&(interface, concrete)) {
            return Some(existing);
        }

        let checked = self.checked;
        let slots: Vec<ConformanceMethod> = checked
            .conformances
            .iter()
            .find(|conformance| {
                conformance.interface == interface
                    && self.types.resolve(conformance.concrete) == concrete
            })
            .map(|conformance| conformance.methods.clone())?;

        let arguments = self
            .named_parts(concrete)
            .map(|(_, args)| args)
            .unwrap_or_default();
        let methods: Vec<FuncRef> = slots
            .into_iter()
            .map(|slot| match slot {
                ConformanceMethod::User(func) => self.instance(func, arguments.clone(), span),
                ConformanceMethod::Builtin(method) => self.builtin_shim(method, concrete),
            })
            .collect();

        let interface_definition = checked.context().def(interface);
        let interface_module = checked
            .context()
            .module_path(interface_definition.module)
            .to_vec();
        let interface_name = interface_definition.name.clone();

        let (concrete_module, concrete_name) = match self.named_parts(concrete) {
            Some((def, _)) => {
                let definition = checked.context().def(def);
                (
                    checked.context().module_path(definition.module).to_vec(),
                    definition.name.clone(),
                )
            }
            None => (Vec::new(), self.types.display(concrete)),
        };

        let concrete_type = self.descriptor_for(concrete);
        let handle = self.program.add_itable(Itable {
            name: abi::mangle_itable(
                &interface_module,
                &interface_name,
                &concrete_module,
                &concrete_name,
            ),
            interface_id: interface.0 as u64,
            concrete_type,
            methods,
        });
        self.itables.insert((interface, concrete), handle);
        Some(handle)
    }

    // ---- hang muc module ----

    fn declare_globals(&mut self) {
        let checked = self.checked;
        for constant in &checked.resolution.globals {
            let ty = checked
                .global_types
                .get(constant.id.index())
                .copied()
                .unwrap_or(TypeId::ERROR);
            self.program.add_global(Global {
                name: abi::mangle_function(
                    checked.context().module_path(constant.module),
                    None,
                    &constant.name,
                    "",
                ),
                ty: self.ir_type(ty),
                span: constant.span,
            });
        }
    }

    fn initialisation_plan(&self) -> Vec<(&'c ConstDecl, Vec<GlobalConstId>)> {
        let resolution = &self.checked.resolution;
        let mut order: Vec<(ModuleId, usize)> = Vec::new();
        let mut seen: HashSet<(ModuleId, usize)> = HashSet::new();

        let sequence = resolution
            .const_init_order
            .iter()
            .copied()
            .chain((0..resolution.globals.len() as u32).map(GlobalConstId));
        for constant in sequence {
            let Some(entry) = resolution.globals.get(constant.index()) else {
                continue;
            };
            let key = (entry.module, entry.declaration);
            if seen.insert(key) {
                order.push(key);
            }
        }

        order
            .into_iter()
            .filter_map(|(module, declaration)| {
                let members: Vec<GlobalConstId> = resolution
                    .globals
                    .iter()
                    .filter(|entry| entry.module == module && entry.declaration == declaration)
                    .map(|entry| entry.id)
                    .collect();
                let declaration = resolution.global_decl(*members.first()?)?;
                Some((declaration, members))
            })
            .collect()
    }

    // ---- day cai worklist ----

    fn lower_job(&mut self, job: Job<'c>) {
        match job {
            Job::Function {
                handle,
                func,
                subst,
            } => self.lower_function(handle, func, subst),
            Job::Closure(job) => self.lower_closure_body(*job),
            Job::Thunk { handle, target } => self.lower_thunk(handle, target),
            Job::BuiltinShim {
                handle,
                method,
                concrete,
            } => self.lower_builtin_shim(handle, method, concrete),
            Job::ModuleInit { handle } => self.lower_module_init(handle),
        }
    }

    fn lower_function(&mut self, handle: FuncRef, func: FuncId, subst: Substitution) {
        let Some(declaration) = self.checked.resolution.function_decl(func) else {
            let definition = self.checked.context().func(func);
            if definition.has_body {
                let span = definition.span;
                let name = definition.name.clone();
                self.unsupported(span, format!("no body was found for `{name}`"));
            }
            return;
        };

        let definition = self.checked.context().func(func);
        let module = definition.module;
        let has_receiver = definition.has_receiver;
        let declared: Vec<(TypeId, bool)> = definition
            .params
            .iter()
            .map(|param| (param.ty, param.variadic))
            .collect();
        let return_type = self.apply(definition.ret, &subst);

        let parameters: Vec<(&'c Param, TypeId, bool)> = declaration
            .params
            .iter()
            .zip(declared)
            .map(|(written, (ty, variadic))| (written, ty, variadic))
            .collect();

        let mut body = Body::new(self, handle, subst, module, return_type);
        body.bind_parameters(has_receiver, &parameters);
        body.lower_block(&declaration.body);
        body.finish();
    }

    fn lower_closure_body(&mut self, job: ClosureJob<'c>) {
        let TypeKind::Function(signature) = self.kind_of(job.ty) else {
            self.unsupported(job.expr.span, "a closure whose type is not a function type");
            return;
        };

        let parameters: Vec<(&'c Param, TypeId, bool)> = job
            .expr
            .params
            .iter()
            .enumerate()
            .map(|(index, written)| {
                let variadic = matches!(written.kind, ParamKind::Variadic);
                let declared = if variadic {
                    signature.variadic.unwrap_or(TypeId::ERROR)
                } else {
                    signature
                        .params
                        .get(index)
                        .copied()
                        .unwrap_or(TypeId::ERROR)
                };
                (written, declared, variadic)
            })
            .collect();

        let mut body = Body::new(self, job.handle, job.subst, job.module, signature.ret);
        body.bind_environment(&job.captures, job.captured_this);
        body.bind_parameters(false, &parameters);
        body.lower_block(&job.expr.body);
        body.finish();
    }

    fn lower_thunk(&mut self, handle: FuncRef, target: FuncRef) {
        let result = self.program.function(target).signature.ret;
        let mut function = self.program.function(handle).clone();
        let span = function.span;
        let entry = function.entry;
        // tham so vat ly so 0 la object closure, thunk bo qua no.
        let arguments: Vec<Value> = function.params()[1..].to_vec();
        let value = function.push_call(
            entry,
            InstKind::Call {
                func: target,
                args: arguments,
            },
            result,
            span,
        );
        function.set_terminator(entry, Terminator::Return { value });
        *self.program.function_mut(handle) = function;
    }

    fn lower_builtin_shim(&mut self, handle: FuncRef, method: BuiltinMethod, concrete: TypeId) {
        let repr = self.ir_type(concrete);
        let mut body = Body::new(
            self,
            handle,
            Substitution::default(),
            ModuleId(0),
            TypeId::STRING,
        );
        let span = Span::synthetic();
        let receiver = body.function.params()[0];

        let result = match method {
            BuiltinMethod::StringMessage => receiver,
            BuiltinMethod::ToString if repr.is_pointer() => {
                body.stringify(receiver, concrete, span)
            }
            BuiltinMethod::ToString => {
                let raw = body.value(
                    InstKind::Load {
                        ptr: receiver,
                        offset: abi::boxed::VALUE_OFFSET as i32,
                        ty: IrType::I64,
                    },
                    span,
                );
                let unboxed = body.narrow(raw, repr, span);
                body.stringify(unboxed, concrete, span)
            }
            other => {
                let spelling = other.spelling();
                body.lo.unsupported(
                    span,
                    format!("`{spelling}` cannot be reached through an interface"),
                );
                receiver
            }
        };
        body.seal(Terminator::Return {
            value: Some(result),
        });
        body.finish();
    }

    fn lower_module_init(&mut self, handle: FuncRef) {
        let plan = self.initialisation_plan();
        let mut body = Body::new(
            self,
            handle,
            Substitution::default(),
            ModuleId(0),
            TypeId::VOID,
        );
        for (declaration, members) in plan {
            body.lower_constant_declaration(declaration, &members);
        }
        body.finish();
    }
}

#[derive(Clone, Copy, Debug)]
enum Storage {
    Direct { slot: SlotRef, ty: IrType },
    Boxed { slot: SlotRef, ty: IrType },
    Captured { index: u32, ty: IrType },
}

#[derive(Clone, Copy, Debug)]
struct LoopFrame {
    continue_target: BlockRef,
    break_target: BlockRef,
}

struct Body<'a, 'c> {
    lo: &'a mut Lowerer<'c>,
    handle: FuncRef,
    function: Function,
    block: BlockRef,
    terminated: bool,
    subst: Substitution,
    module: ModuleId,
    return_type: TypeId,
    return_repr: Option<IrType>,
    storage: HashMap<LocalId, Storage>,
    this: Option<Value>,
    environment: Option<Value>,
    loops: Vec<LoopFrame>,
}

impl<'a, 'c> Body<'a, 'c> {
    fn new(
        lo: &'a mut Lowerer<'c>,
        handle: FuncRef,
        subst: Substitution,
        module: ModuleId,
        return_type: TypeId,
    ) -> Body<'a, 'c> {
        let declared = lo.program.function(handle);
        let mut function = Function::new(
            declared.name.clone(),
            declared.signature.clone(),
            declared.span,
        );
        function.failable = declared.failable;
        function.exported = declared.exported;
        function.source_return_type = declared.source_return_type;
        let block = function.entry;
        let return_repr = function.signature.ret;

        Body {
            lo,
            handle,
            function,
            block,
            terminated: false,
            subst,
            module,
            return_type,
            return_repr,
            storage: HashMap::new(),
            this: None,
            environment: None,
            loops: Vec::new(),
        }
    }

    fn checked(&self) -> &'c Checked {
        self.lo.checked
    }

    fn finish(mut self) {
        if !self.terminated {
            let value = self.return_repr.map(|ty| self.zero(ty, Span::synthetic()));
            self.seal(Terminator::Return { value });
        }
        *self.lo.program.function_mut(self.handle) = self.function;
    }

    // ---- sinh ma ----

    fn push(&mut self, kind: InstKind, span: Span) -> Option<Value> {
        self.function.push(self.block, kind, span)
    }

    fn value(&mut self, kind: InstKind, span: Span) -> Value {
        self.function.push_value(self.block, kind, span)
    }

    fn call(&mut self, kind: InstKind, result: Option<IrType>, span: Span) -> Option<Value> {
        self.function.push_call(self.block, kind, result, span)
    }

    fn runtime(&mut self, entry: RuntimeFn, args: Vec<Value>, span: Span) -> Option<Value> {
        let signature = entry.signature();
        debug_assert_eq!(signature.params.len(), args.len(), "{entry:?} arity");
        let result = self.call(InstKind::CallRuntime { entry, args }, signature.ret, span);
        if signature.diverges {
            self.seal(Terminator::Unreachable);
        }
        result
    }

    fn runtime_value(&mut self, entry: RuntimeFn, args: Vec<Value>, span: Span) -> Value {
        match self.runtime(entry, args, span) {
            Some(value) => value,
            None => self.const_int(0, span),
        }
    }

    fn new_block(&mut self, span: Span) -> BlockRef {
        self.function.new_block(span)
    }

    fn switch_to(&mut self, block: BlockRef) {
        self.block = block;
        self.terminated = false;
    }

    fn seal(&mut self, terminator: Terminator) {
        if self.terminated {
            return;
        }
        self.function.set_terminator(self.block, terminator);
        self.terminated = true;
    }

    fn jump(&mut self, target: BlockRef, args: Vec<Value>) {
        self.seal(Terminator::Jump { target, args });
    }

    fn branch(&mut self, cond: Value, then_block: BlockRef, else_block: BlockRef) {
        self.seal(Terminator::Branch {
            cond,
            then_block,
            then_args: Vec::new(),
            else_block,
            else_args: Vec::new(),
        });
    }

    fn load(&mut self, ptr: Value, offset: u32, ty: IrType, span: Span) -> Value {
        self.value( InstKind::Load { ptr, offset: offset as i32, ty, }, span, )
    }

    fn store(&mut self, ptr: Value, offset: u32, value: Value, span: Span) {
        self.push(
            InstKind::Store {
                ptr,
                offset: offset as i32,
                value,
            },
            span,
        );
    }

    fn compare(&mut self, op: CompareOp, lhs: Value, rhs: Value, span: Span) -> Value {
        self.value(InstKind::Compare { op, lhs, rhs }, span)
    }

    // ---- hang so ----

    fn const_int(&mut self, literal: i64, span: Span) -> Value {
        self.value(InstKind::ConstInt(literal), span)
    }

    fn const_bool(&mut self, literal: bool, span: Span) -> Value {
        self.value(InstKind::ConstBool(literal), span)
    }

    fn const_null(&mut self, span: Span) -> Value {
        self.value(InstKind::ConstNull, span)
    }

    fn const_string(&mut self, text: &str, span: Span) -> Value {
        let handle = self.lo.program.add_string(text);
        self.value(InstKind::ConstString(handle), span)
    }

    fn zero(&mut self, ty: IrType, span: Span) -> Value {
        match ty {
            IrType::I8 => self.const_bool(false, span),
            IrType::I32 => self.value(InstKind::ConstChar(0), span),
            IrType::I64 => self.const_int(0, span),
            IrType::F64 => self.value(InstKind::ConstFloat(0.0), span),
            IrType::Ptr => self.const_null(span),
        }
    }

    // ---- doi cach bieu dien ----

    fn widen(&mut self, value: Value, ty: IrType, span: Span) -> Value {
        let op = match ty {
            IrType::I8 => ConvertOp::BoolToInt,
            IrType::I32 => ConvertOp::CharToInt,
            IrType::I64 => return value,
            IrType::F64 => ConvertOp::BitcastFloatToInt,
            IrType::Ptr => ConvertOp::PtrToInt,
        };
        self.value(InstKind::Convert { op, value }, span)
    }

    fn narrow(&mut self, value: Value, ty: IrType, span: Span) -> Value {
        let op = match ty {
            IrType::I8 => ConvertOp::IntToBool,
            IrType::I32 => ConvertOp::IntToChar,
            IrType::I64 => return value,
            IrType::F64 => ConvertOp::BitcastIntToFloat,
            IrType::Ptr => ConvertOp::IntToPtr,
        };
        self.value(InstKind::Convert { op, value }, span)
    }

    fn box_value(&mut self, value: Value, ty: IrType, span: Span) -> Value {
        let type_id = if ty.is_pointer() {
            abi::TYPE_ID_BOX_REF
        } else {
            abi::TYPE_ID_BOX_SCALAR
        };
        let type_id = self.const_u32(type_id, span);
        let widened = self.widen(value, ty, span);
        self.runtime_value(RuntimeFn::BoxNew, vec![type_id, widened], span)
    }

    fn unbox_value(&mut self, boxed: Value, ty: IrType, span: Span) -> Value {
        let raw = self.load(boxed, abi::boxed::VALUE_OFFSET, IrType::I64, span);
        self.narrow(raw, ty, span)
    }

    // ---- kieu, lan 2 ----

    fn ty_of(&mut self, node: NodeId) -> TypeId {
        let recorded = self.checked().type_of(node);
        self.apply(recorded)
    }

    fn apply(&mut self, ty: TypeId) -> TypeId {
        let subst = self.subst.clone();
        self.lo.apply(ty, &subst)
    }

    fn ir_type(&self, ty: TypeId) -> IrType {
        self.lo.ir_type(ty)
    }

    fn kind_of(&self, ty: TypeId) -> TypeKind {
        self.lo.kind_of(ty)
    }

    fn element_of(&self, ty: TypeId) -> TypeId {
        match self.kind_of(ty) {
            TypeKind::Array(element) | TypeKind::Set(element) => element,
            _ => TypeId::ERROR,
        }
    }

    fn interface_of(&self, ty: TypeId) -> Option<DefId> {
        let (def, _) = self.lo.named_parts(ty)?;
        self.checked()
            .context()
            .def(def)
            .as_interface()
            .map(|_| def)
    }

    // ---- ep kieu ----

    fn coerce(&mut self, value: Value, from: TypeId, to: TypeId, span: Span) -> Value {
        let from = self.apply(from);
        let to = self.apply(to);
        if from == to {
            return value;
        }

        match (self.kind_of(from), self.kind_of(to)) {
            (_, TypeKind::Optional(inner)) => {
                if matches!(self.kind_of(from), TypeKind::Optional(_)) {
                    return value;
                }
                if matches!(self.kind_of(from), TypeKind::Never | TypeKind::Error) {
                    return value;
                }
                let inner_repr = self.ir_type(inner);
                let value = self.coerce(value, from, inner, span);
                if inner_repr.is_pointer() {
                    value
                } else {
                    self.box_value(value, inner_repr, span)
                }
            }
            (TypeKind::Optional(inner), _) if inner == to => {
                // thu hep null da chung minh gia tri khac null roi; kieu
                // cai nay tham chieu thi
                // khoi hop.
                let repr = self.ir_type(inner);
                if repr.is_pointer() {
                    value
                } else {
                    self.unbox_value(value, repr, span)
                }
            }
            _ => match self.interface_of(to) {
                Some(interface) if self.interface_of(from) != Some(interface) => {
                    self.make_interface(value, from, interface, span)
                }
                _ => value,
            },
        }
    }

    fn unwrap_narrowed(
        &mut self,
        value: Value,
        declared: TypeId,
        narrowed: TypeId,
        span: Span,
    ) -> Value {
        let declared = self.apply(declared);
        let narrowed = self.apply(narrowed);
        if declared == narrowed {
            return value;
        }
        match self.kind_of(declared) {
            TypeKind::Optional(inner) if self.apply(inner) == narrowed => {
                let repr = self.ir_type(narrowed);
                if repr.is_pointer() {
                    value
                } else {
                    self.unbox_value(value, repr, span)
                }
            }
            _ => value,
        }
    }

    fn make_interface(
        &mut self,
        value: Value,
        concrete: TypeId,
        interface: DefId,
        span: Span,
    ) -> Value {
        let repr = self.ir_type(concrete);
        let data = if repr.is_pointer() {
            value
        } else {
            self.box_value(value, repr, span)
        };
        let Some(itable) = self.lo.itable(interface, concrete, span) else {
            let name = self.checked().context().def(interface).name.clone();
            let shown = self.lo.types.display(concrete);
            self.lo.unsupported(
                span,
                format!("no itable was recorded for `{shown}` as `{name}`"),
            );
            return data;
        };
        let itable = self.value(InstKind::ConstItable(itable), span);
        self.runtime_value(RuntimeFn::IfaceNew, vec![itable, data], span)
    }

    // ---- buoc ten ----

    fn declared_local(&self, span: Span) -> Option<LocalId> {
        self.checked()
            .resolution
            .declared_locals
            .get(&span)
            .copied()
    }

    fn local_type(&mut self, local: LocalId) -> TypeId {
        let recorded = self
            .checked()
            .local_types
            .get(local.index())
            .copied()
            .unwrap_or(TypeId::ERROR);
        self.apply(recorded)
    }

    fn is_captured(&self, local: LocalId) -> bool {
        self.checked()
            .resolution
            .locals
            .get(local.index())
            .is_some_and(|binding| binding.captured)
    }

    fn declare_binding(&mut self, local: LocalId, ty: TypeId, initial: Value, span: Span) {
        let repr = self.ir_type(ty);
        let storage = if self.is_captured(local) {
            let slot = self.function.add_slot_for(IrType::Ptr, span);
            Storage::Boxed { slot, ty: repr }
        } else {
            let slot = self.function.add_slot_for(repr, span);
            Storage::Direct { slot, ty: repr }
        };
        self.storage.insert(local, storage);

        match storage {
            Storage::Direct { slot, .. } => {
                let address = self.value(InstKind::SlotAddr(slot), span);
                self.store(address, 0, initial, span);
            }
            Storage::Boxed { slot, .. } => {
                let boxed = self.box_value(initial, repr, span);
                let address = self.value(InstKind::SlotAddr(slot), span);
                self.store(address, 0, boxed, span);
            }
            Storage::Captured { .. } => unreachable!("a declaration is never a capture"),
        }
    }

    fn box_pointer(&mut self, local: LocalId, span: Span) -> Option<Value> {
        match self.storage.get(&local).copied() {
            Some(Storage::Boxed { slot, .. }) => {
                let address = self.value(InstKind::SlotAddr(slot), span);
                Some(self.load(address, 0, IrType::Ptr, span))
            }
            Some(Storage::Captured { index, .. }) => {
                let environment = self.environment?;
                let offset = abi::closure::capture_offset(index as u64) as u32;
                Some(self.load(environment, offset, IrType::Ptr, span))
            }
            _ => None,
        }
    }

    fn read_binding(&mut self, local: LocalId, span: Span) -> Option<Value> {
        match self.storage.get(&local).copied() {
            Some(Storage::Direct { slot, ty }) => {
                let address = self.value(InstKind::SlotAddr(slot), span);
                Some(self.load(address, 0, ty, span))
            }
            Some(Storage::Boxed { ty, .. }) | Some(Storage::Captured { ty, .. }) => {
                let boxed = self.box_pointer(local, span)?;
                Some(self.unbox_value(boxed, ty, span))
            }
            None => None,
        }
    }

    fn write_binding(&mut self, local: LocalId, value: Value, span: Span) {
        match self.storage.get(&local).copied() {
            Some(Storage::Direct { slot, .. }) => {
                let address = self.value(InstKind::SlotAddr(slot), span);
                self.store(address, 0, value, span);
            }
            Some(Storage::Boxed { ty, .. }) | Some(Storage::Captured { ty, .. }) => {
                let Some(boxed) = self.box_pointer(local, span) else {
                    return;
                };
                let widened = self.widen(value, ty, span);
                self.store(boxed, abi::boxed::VALUE_OFFSET, widened, span);
            }
            None => self
                .lo
                .unsupported(span, "an assignment to a binding with no storage"),
        }
    }

    fn bind_parameters(&mut self, has_receiver: bool, parameters: &[(&'c Param, TypeId, bool)]) {
        let incoming: Vec<Value> = self.function.params().to_vec();
        let mut next = usize::from(self.environment.is_some());

        if has_receiver {
            self.this = incoming.get(next).copied();
            next += 1;
        }

        for (written, declared, variadic) in parameters {
            let Some(value) = incoming.get(next).copied() else {
                break;
            };
            next += 1;

            let ty = if *variadic {
                let element = self.apply(*declared);
                self.lo.types.array_of(element)
            } else {
                self.apply(*declared)
            };
            if let Some(local) = self.declared_local(written.name.span) {
                self.declare_binding(local, ty, value, written.span);
            }
        }
    }

    fn bind_environment(&mut self, captures: &[LocalId], captured_this: Option<u32>) {
        let environment = self.function.params().first().copied();
        self.environment = environment;

        for (index, local) in captures.iter().enumerate() {
            let ty = self.local_type(*local);
            let repr = self.ir_type(ty);
            self.storage.insert(
                *local,
                Storage::Captured {
                    index: index as u32,
                    ty: repr,
                },
            );
        }

        if let (Some(index), Some(environment)) = (captured_this, environment) {
            let span = Span::synthetic();
            let offset = abi::closure::capture_offset(index as u64) as u32;
            let boxed = self.load(environment, offset, IrType::Ptr, span);
            self.this = Some(self.unbox_value(boxed, IrType::Ptr, span));
        }
    }

    // ---- hang muc module ----

    fn lower_constant_declaration(
        &mut self,
        declaration: &'c ConstDecl,
        members: &[GlobalConstId],
    ) {
        let span = declaration.value.span;
        let whole = self.ty_of(declaration.value.id);
        let value = self.lower_coerced(&declaration.value, whole);

        for member in members {
            let Some(entry) = self.checked().resolution.globals.get(member.index()) else {
                continue;
            };
            let mut projected = value;
            let mut ty = whole;
            for &step in &entry.path {
                let TypeKind::Tuple(elements) = self.kind_of(ty) else {
                    break;
                };
                let layout = abi::layout_tuple(&self.lo.types, &elements);
                let Some(field) = layout.fields.get(step as usize) else {
                    break;
                };
                projected = self.load(projected, field.offset, field.ty, span);
                ty = elements[step as usize];
            }
            let address = self.value(InstKind::GlobalAddr(GlobalRef(member.0)), span);
            self.store(address, 0, projected, span);
        }
    }

    // ---- statement ----

    fn lower_block(&mut self, block: &'c Block) {
        for statement in &block.statements {
            if self.terminated {
                // phan con lai cua block khong the toi duoc.
                break;
            }
            self.lower_stmt(statement);
        }
    }

    fn lower_stmt(&mut self, statement: &'c Stmt) {
        let span = statement.span;
        match &statement.kind {
            StmtKind::Let(declaration) => {
                let ty = self.ty_of(declaration.value.id);
                let value = self.lower_coerced(&declaration.value, ty);
                self.bind_irrefutable(&declaration.pattern, value, ty, span);
            }
            StmtKind::Const(declaration) => {
                let ty = self.ty_of(declaration.value.id);
                let value = self.lower_coerced(&declaration.value, ty);
                self.bind_irrefutable(&declaration.pattern, value, ty, span);
            }
            StmtKind::Assign(assignment) => self.lower_assignment(assignment),
            StmtKind::Expr(expression) => {
                self.lower_expr(expression);
            }
            StmtKind::If(statement) => self.lower_if(statement),
            StmtKind::While(statement) => self.lower_while(statement),
            StmtKind::For(statement) => self.lower_for(statement),
            StmtKind::Match(statement) => self.lower_match(statement),
            StmtKind::Return(None) => {
                let value = self.return_repr.map(|ty| self.zero(ty, span));
                self.seal(Terminator::Return { value });
            }
            StmtKind::Return(Some(expression)) => {
                let target = self.return_type;
                let value = self.lower_coerced(expression, target);
                let value = self.return_repr.map(|_| value);
                self.seal(Terminator::Return { value });
            }
            StmtKind::Fail(expression) => self.lower_fail(expression),
            StmtKind::Break => match self.loops.last().copied() {
                Some(frame) => self.jump(frame.break_target, Vec::new()),
                None => self.lo.unsupported(span, "`break` outside a loop"),
            },
            StmtKind::Continue => match self.loops.last().copied() {
                Some(frame) => self.jump(frame.continue_target, Vec::new()),
                None => self.lo.unsupported(span, "`continue` outside a loop"),
            },
            StmtKind::Block(block) => self.lower_block(block),
        }
    }

    fn lower_fail(&mut self, expression: &'c Expr) {
        let error_type = self.checked().resolution.prelude.error_type;
        let error = self.lower_coerced(expression, error_type);
        self.seal(Terminator::ReturnError { error });
    }

    fn lower_if(&mut self, statement: &'c IfStmt) {
        let span = statement.span;
        let condition = self.lower_operand(&statement.condition);
        let then_block = self.new_block(statement.then_block.span);
        let else_block = self.new_block(span);
        let join = self.new_block(span);
        self.branch(condition, then_block, else_block);

        self.switch_to(then_block);
        self.lower_block(&statement.then_block);
        self.jump(join, Vec::new());

        self.switch_to(else_block);
        match &statement.else_branch {
            Some(crate::ast::ElseBranch::Block(block)) => self.lower_block(block),
            Some(crate::ast::ElseBranch::If(nested)) => self.lower_if(nested),
            None => {}
        }
        self.jump(join, Vec::new());

        self.switch_to(join);
    }

    fn lower_while(&mut self, statement: &'c WhileStmt) {
        let span = statement.span;
        let header = self.new_block(span);
        let body = self.new_block(statement.body.span);
        let exit = self.new_block(span);
        self.jump(header, Vec::new());

        self.switch_to(header);
        let condition = self.lower_operand(&statement.condition);
        self.branch(condition, body, exit);

        self.switch_to(body);
        self.loops.push(LoopFrame {
            continue_target: header,
            break_target: exit,
        });
        self.lower_block(&statement.body);
        self.loops.pop();
        self.jump(header, Vec::new());

        self.switch_to(exit);
    }

    // ---- vong for ----

    fn lower_for(&mut self, statement: &'c ForStmt) {
        let iterable = strip_groups(&statement.iterable);
        if let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &iterable.kind
        {
            let start = self.lower_operand(start);
            let end = self.lower_operand(end);
            let inclusive = self.const_bool(*inclusive, iterable.span);
            self.lower_range_loop(statement, start, end, inclusive);
            return;
        }

        let ty = self.ty_of(iterable.id);
        let range_type = self.checked().resolution.prelude.range_type;
        if self.apply(range_type) == ty {
            let span = iterable.span;
            let object = self.lower_operand(iterable);
            let layout = self.range_layout();
            let start = self.load(object, layout[0], IrType::I64, span);
            let end = self.load(object, layout[1], IrType::I64, span);
            let inclusive = self.load(object, layout[2], IrType::I8, span);
            self.lower_range_loop(statement, start, end, inclusive);
            return;
        }

        match self.kind_of(ty) {
            TypeKind::Array(element) => {
                let array = self.lower_operand(iterable);
                self.lower_array_loop(statement, array, element, true);
            }
            TypeKind::String => {
                let string = self.lower_operand(iterable);
                let span = iterable.span;
                let chars = self.runtime_value(RuntimeFn::StringChars, vec![string], span);
                self.lower_array_loop(statement, chars, TypeId::CHAR, false);
            }
            TypeKind::Map { key, value } => {
                let map = self.lower_operand(iterable);
                self.lower_table_loop(statement, map, Some((key, value)));
            }
            TypeKind::Set(element) => {
                let set = self.lower_operand(iterable);
                self.lower_table_loop(statement, set, None);
                let _ = element;
            }
            _ => {
                let shown = self.lo.types.display(ty);
                self.lo
                    .unsupported(iterable.span, format!("iteration over `{shown}`"));
            }
        }
    }

    fn range_layout(&self) -> [u32; 3] {
        let layout = abi::layout_struct(&self.lo.types, &[TypeId::INT, TypeId::INT, TypeId::BOOL]);
        [
            layout.fields[0].offset,
            layout.fields[1].offset,
            layout.fields[2].offset,
        ]
    }

    fn lower_range_loop(
        &mut self,
        statement: &'c ForStmt,
        start: Value,
        end: Value,
        inclusive: Value,
    ) {
        let span = statement.span;
        let cursor = self.function.add_slot_for(IrType::I64, span);
        let cursor_address = self.value(InstKind::SlotAddr(cursor), span);
        self.store(cursor_address, 0, start, span);

        let header = self.new_block(span);
        let body = self.new_block(statement.body.span);
        let latch = self.new_block(span);
        let exit = self.new_block(span);
        self.jump(header, Vec::new());

        self.switch_to(header);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let current = self.load(address, 0, IrType::I64, span);
        let below = self.compare(CompareOp::SLt, current, end, span);
        let at_or_below = self.compare(CompareOp::SLe, current, end, span);
        let condition = self.value(
            InstKind::Select {
                cond: inclusive,
                then_value: at_or_below,
                else_value: below,
            },
            span,
        );
        self.branch(condition, body, exit);

        self.switch_to(body);
        self.bind_irrefutable(&statement.pattern, current, TypeId::INT, span);
        self.loops.push(LoopFrame {
            continue_target: latch,
            break_target: exit,
        });
        self.lower_block(&statement.body);
        self.loops.pop();
        self.jump(latch, Vec::new());

        self.switch_to(latch);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let current = self.load(address, 0, IrType::I64, span);
        let finished = self.compare(CompareOp::IEq, current, end, span);
        let step = self.new_block(span);
        self.branch(finished, exit, step);

        self.switch_to(step);
        let one = self.const_int(1, span);
        let next = self.value(
            InstKind::Binary {
                op: BinaryOp::IAdd,
                lhs: current,
                rhs: one,
            },
            span,
        );
        let address = self.value(InstKind::SlotAddr(cursor), span);
        self.store(address, 0, next, span);
        self.jump(header, Vec::new());

        self.switch_to(exit);
    }

    fn lower_array_loop(
        &mut self,
        statement: &'c ForStmt,
        array: Value,
        element: TypeId,
        guard: bool,
    ) {
        let span = statement.span;
        let element_repr = self.ir_type(element);
        let snapshot = if guard {
            Some(self.runtime_value(RuntimeFn::CollectionModcount, vec![array], span))
        } else {
            None
        };

        let cursor = self.function.add_slot_for(IrType::I64, span);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let start = self.const_int(0, span);
        self.store(address, 0, start, span);

        let header = self.new_block(span);
        let body = self.new_block(statement.body.span);
        let latch = self.new_block(span);
        let exit = self.new_block(span);
        self.jump(header, Vec::new());

        self.switch_to(header);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let index = self.load(address, 0, IrType::I64, span);
        let length = self.runtime_value(RuntimeFn::ArrayLen, vec![array], span);
        let condition = self.compare(CompareOp::SLt, index, length, span);
        self.branch(condition, body, exit);

        self.switch_to(body);
        if let Some(snapshot) = snapshot {
            self.check_modcount(array, snapshot, span);
        }
        let raw = self.runtime_value(RuntimeFn::ArrayGet, vec![array, index], span);
        let value = self.narrow(raw, element_repr, span);
        self.bind_irrefutable(&statement.pattern, value, element, span);
        self.loops.push(LoopFrame {
            continue_target: latch,
            break_target: exit,
        });
        self.lower_block(&statement.body);
        self.loops.pop();
        self.jump(latch, Vec::new());

        self.switch_to(latch);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let index = self.load(address, 0, IrType::I64, span);
        let one = self.const_int(1, span);
        let next = self.value(
            InstKind::Binary {
                op: BinaryOp::IAdd,
                lhs: index,
                rhs: one,
            },
            span,
        );
        self.store(address, 0, next, span);
        self.jump(header, Vec::new());

        self.switch_to(exit);
    }

    fn lower_table_loop(
        &mut self,
        statement: &'c ForStmt,
        table: Value,
        entries: Option<(TypeId, TypeId)>,
    ) {
        let span = statement.span;
        let snapshot = self.runtime_value(RuntimeFn::CollectionModcount, vec![table], span);

        let cursor = self.function.add_slot_for(IrType::I64, span);
        let first = self.function.add_slot_for(IrType::I64, span);
        let second = self.function.add_slot_for(IrType::I64, span);
        let address = self.value(InstKind::SlotAddr(cursor), span);
        let zero = self.const_int(0, span);
        self.store(address, 0, zero, span);

        let header = self.new_block(span);
        let body = self.new_block(statement.body.span);
        let exit = self.new_block(span);
        self.jump(header, Vec::new());

        self.switch_to(header);
        let cursor_address = self.value(InstKind::SlotAddr(cursor), span);
        let first_address = self.value(InstKind::SlotAddr(first), span);
        let more = match entries {
            Some(_) => {
                let second_address = self.value(InstKind::SlotAddr(second), span);
                self.runtime_value(
                    RuntimeFn::MapIterNext,
                    vec![table, cursor_address, first_address, second_address],
                    span,
                )
            }
            None => self.runtime_value(
                RuntimeFn::SetIterNext,
                vec![table, cursor_address, first_address],
                span,
            ),
        };
        self.branch(more, body, exit);

        self.switch_to(body);
        self.check_modcount(table, snapshot, span);
        match entries {
            Some((key_ty, value_ty)) => {
                let key_address = self.value(InstKind::SlotAddr(first), span);
                let key_raw = self.load(key_address, 0, IrType::I64, span);
                let key_repr = self.ir_type(key_ty);
                let key = self.narrow(key_raw, key_repr, span);

                let value_address = self.value(InstKind::SlotAddr(second), span);
                let value_raw = self.load(value_address, 0, IrType::I64, span);
                let value_repr = self.ir_type(value_ty);
                let value = self.narrow(value_raw, value_repr, span);

                self.bind_entry(&statement.pattern, key, key_ty, value, value_ty, span);
            }
            None => {
                let element_ty = {
                    let ty = self.ty_of(statement.iterable.id);
                    self.element_of(ty)
                };
                let element_address = self.value(InstKind::SlotAddr(first), span);
                let raw = self.load(element_address, 0, IrType::I64, span);
                let repr = self.ir_type(element_ty);
                let element = self.narrow(raw, repr, span);
                self.bind_irrefutable(&statement.pattern, element, element_ty, span);
            }
        }
        self.loops.push(LoopFrame {
            continue_target: header,
            break_target: exit,
        });
        self.lower_block(&statement.body);
        self.loops.pop();
        self.jump(header, Vec::new());

        self.switch_to(exit);
    }

    fn bind_entry(
        &mut self,
        pattern: &'c IrrefutablePattern,
        key: Value,
        key_ty: TypeId,
        value: Value,
        value_ty: TypeId,
        span: Span,
    ) {
        if let IrrefutablePatternKind::Tuple(elements) = &pattern.kind {
            if elements.len() == 2 {
                self.bind_irrefutable(&elements[0], key, key_ty, span);
                self.bind_irrefutable(&elements[1], value, value_ty, span);
                return;
            }
        }
        let ty = self.lo.types.tuple_of(vec![key_ty, value_ty]);
        let tuple = self.build_record(ty, &[key_ty, value_ty], &[key, value], span);
        self.bind_irrefutable(pattern, tuple, ty, span);
    }

    fn check_modcount(&mut self, collection: Value, snapshot: Value, span: Span) {
        let now = self.runtime_value(RuntimeFn::CollectionModcount, vec![collection], span);
        let unchanged = self.compare(CompareOp::IEq, now, snapshot, span);
        let ok = self.new_block(span);
        let bad = self.new_block(span);
        self.branch(unchanged, ok, bad);

        self.switch_to(bad);
        self.runtime(RuntimeFn::PanicConcurrentModification, Vec::new(), span);

        self.switch_to(ok);
    }

    // ---- pattern khong the truot ----

    fn bind_irrefutable(
        &mut self,
        pattern: &'c IrrefutablePattern,
        value: Value,
        ty: TypeId,
        span: Span,
    ) {
        match &pattern.kind {
            IrrefutablePatternKind::Wildcard => {}
            IrrefutablePatternKind::Binding(name) => {
                let Some(local) = self.declared_local(name.span) else {
                    return;
                };
                let declared = self.local_type(local);
                let value = self.coerce(value, ty, declared, span);
                self.declare_binding(local, declared, value, name.span);
            }
            IrrefutablePatternKind::Tuple(elements) => {
                let TypeKind::Tuple(element_types) = self.kind_of(ty) else {
                    self.lo
                        .unsupported(pattern.span, "a tuple pattern on a non-tuple");
                    return;
                };
                let layout = abi::layout_tuple(&self.lo.types, &element_types);
                for (index, element) in elements.iter().enumerate() {
                    let Some(field) = layout.fields.get(index) else {
                        break;
                    };
                    let extracted = self.load(value, field.offset, field.ty, span);
                    self.bind_irrefutable(element, extracted, element_types[index], span);
                }
            }
        }
    }

    // ---- phep gan ----

    fn lower_assignment(&mut self, assignment: &'c AssignStmt) {
        let span = assignment.span;
        let target = strip_groups(&assignment.target);
        let target_ty = self.ty_of(target.id);

        match &target.kind {
            ExprKind::Ident(_) | ExprKind::This => {
                let Some(binding) = self.checked().resolution.values.get(&target.id).copied()
                else {
                    self.lo.unsupported(span, "an unresolved assignment target");
                    return;
                };
                match binding {
                    ValueBinding::Local(local) | ValueBinding::Captured(local) => {
                        let value = self.combine_assignment(assignment, target_ty, |body| {
                            body.read_binding(local, span)
                        });
                        if let Some(value) = value {
                            self.write_binding(local, value, span);
                        }
                    }
                    ValueBinding::Field { owner, index } => {
                        let Some(this) = self.this else {
                            self.lo
                                .unsupported(span, "a field assignment outside a method");
                            return;
                        };
                        let this_ty = self.this_type(owner);
                        self.assign_field(assignment, this, this_ty, owner, index, target_ty, span);
                    }
                    ValueBinding::GlobalConst(global) => {
                        self.assign_global(assignment, global, target_ty, span);
                    }
                    _ => self
                        .lo
                        .unsupported(span, "an unsupported assignment target"),
                }
            }
            ExprKind::Field { base, name } => {
                match self.checked().field_accesses.get(&target.id).copied() {
                    Some(FieldAccess::Field { owner, index }) => {
                        let object_ty = self.ty_of(base.id);
                        let object = self.lower_operand(base);
                        self.assign_field(
                            assignment, object, object_ty, owner, index, target_ty, span,
                        );
                    }
                    _ => {
                        let field = name.name.clone();
                        self.lo
                            .unsupported(span, format!("an assignment to `{field}`"));
                    }
                }
            }
            ExprKind::Index { base, index } => {
                self.assign_index(assignment, base, index, target_ty, span)
            }
            _ => self
                .lo
                .unsupported(span, "an unsupported assignment target"),
        }
    }

    fn combine_assignment(
        &mut self,
        assignment: &'c AssignStmt,
        target_ty: TypeId,
        read: impl FnOnce(&mut Self) -> Option<Value>,
    ) -> Option<Value> {
        let span = assignment.span;
        match assignment.op.binary_op() {
            None => Some(self.lower_coerced(&assignment.value, target_ty)),
            Some(op) => {
                let current = read(self)?;
                let operand = self.lower_coerced(&assignment.value, target_ty);
                Some(self.arithmetic(op, current, operand, target_ty, span))
            }
        }
    }

    fn assign_field(
        &mut self,
        assignment: &'c AssignStmt,
        object: Value,
        object_ty: TypeId,
        owner: DefId,
        index: u32,
        target_ty: TypeId,
        span: Span,
    ) {
        let Some(field) = self.field_layout_of(object_ty, owner, index, span) else {
            return;
        };
        let value = self.combine_assignment(assignment, target_ty, |body| {
            Some(body.load(object, field.0, field.1, span))
        });
        if let Some(value) = value {
            self.store(object, field.0, value, span);
        }
    }

    fn assign_global(
        &mut self,
        assignment: &'c AssignStmt,
        global: GlobalConstId,
        target_ty: TypeId,
        span: Span,
    ) {
        let handle = GlobalRef(global.0);
        let repr = self.ir_type(target_ty);
        let value = self.combine_assignment(assignment, target_ty, |body| {
            let address = body.value(InstKind::GlobalAddr(handle), span);
            Some(body.load(address, 0, repr, span))
        });
        if let Some(value) = value {
            let address = self.value(InstKind::GlobalAddr(handle), span);
            self.store(address, 0, value, span);
        }
    }

    fn assign_index(
        &mut self,
        assignment: &'c AssignStmt,
        base: &'c Expr,
        index: &'c Expr,
        target_ty: TypeId,
        span: Span,
    ) {
        let container_ty = self.ty_of(base.id);
        let container = self.lower_operand(base);
        let key = self.lower_operand(index);
        let repr = self.ir_type(target_ty);

        match self.kind_of(container_ty) {
            TypeKind::Array(_) => {
                let value = self.combine_assignment(assignment, target_ty, |body| {
                    let raw = body.runtime_value(RuntimeFn::ArrayGet, vec![container, key], span);
                    Some(body.narrow(raw, repr, span))
                });
                if let Some(value) = value {
                    let widened = self.widen(value, repr, span);
                    self.runtime(RuntimeFn::ArraySet, vec![container, key, widened], span);
                }
            }
            TypeKind::Map { key: key_ty, .. } => {
                let key_repr = self.ir_type(key_ty);
                let widened_key = self.widen(key, key_repr, span);
                let value = self.combine_assignment(assignment, target_ty, |body| {
                    let raw =
                        body.runtime_value(RuntimeFn::MapGet, vec![container, widened_key], span);
                    Some(body.narrow(raw, repr, span))
                });
                if let Some(value) = value {
                    let widened = self.widen(value, repr, span);
                    self.runtime(
                        RuntimeFn::MapSet,
                        vec![container, widened_key, widened],
                        span,
                    );
                }
            }
            _ => {
                let shown = self.lo.types.display(container_ty);
                self.lo
                    .unsupported(span, format!("an indexed assignment to `{shown}`"));
            }
        }
    }

    fn field_layout_of(
        &mut self,
        object_ty: TypeId,
        owner: DefId,
        index: u32,
        span: Span,
    ) -> Option<(u32, IrType)> {
        let object_ty = self.apply(object_ty);
        let arguments = match self.lo.named_parts(object_ty) {
            Some((def, args)) if def == owner => args
                .into_iter()
                .map(|argument| self.apply(argument))
                .collect(),
            _ => self.owner_arguments(owner),
        };
        let fields = self.lo.struct_fields(owner, &arguments);
        let layout = abi::layout_struct(&self.lo.types, &fields);
        match layout.fields.get(index as usize) {
            Some(field) => Some((field.offset, field.ty)),
            None => {
                self.lo
                    .unsupported(span, "a field index outside the struct");
                None
            }
        }
    }

    fn this_type(&mut self, owner: DefId) -> TypeId {
        let args = self.owner_arguments(owner);
        self.lo.types.intern(TypeKind::Named { def: owner, args })
    }

    fn owner_arguments(&mut self, owner: DefId) -> Vec<TypeId> {
        if self.subst.owner == Some(owner) && !self.subst.owner_args.is_empty() {
            return self.subst.owner_args.clone();
        }
        let generics = self.checked().context().def(owner).generics.len();
        (0..generics)
            .map(|index| {
                self.lo
                    .types
                    .intern(TypeKind::Generic(crate::types::GenericId {
                        owner: GenericOwner::Type(owner),
                        index: index as u32,
                    }))
            })
            .collect()
    }

    // ---- match ----

    fn lower_match(&mut self, statement: &'c MatchStmt) {
        let span = statement.span;
        let scrutinee_type = self.ty_of(statement.scrutinee.id);
        let scrutinee = self.lower_operand(&statement.scrutinee);
        let join = self.new_block(span);

        for arm in &statement.arms {
            let body_block = self.new_block(arm.body.span());
            let next_arm = self.new_block(arm.span);
            self.declare_pattern_bindings(&arm.pattern);

            let matched = if arm.guard.is_some() {
                self.new_block(arm.span)
            } else {
                body_block
            };
            self.test_pattern(&arm.pattern, scrutinee, scrutinee_type, matched, next_arm);

            if let Some(guard) = &arm.guard {
                self.switch_to(matched);
                let condition = self.lower_operand(guard);
                self.branch(condition, body_block, next_arm);
            }

            self.switch_to(body_block);
            match &arm.body {
                MatchArmBody::Block(block) => self.lower_block(block),
                MatchArmBody::Stmt(statement) => self.lower_stmt(statement),
            }
            self.jump(join, Vec::new());

            self.switch_to(next_arm);
        }

        // da check vet het nhanh roi
        // cuoi khong bao gio di duoc; van nhay de block con dung hinh.
        self.jump(join, Vec::new());
        self.switch_to(join);
    }

    fn declare_pattern_bindings(&mut self, pattern: &'c Pattern) {
        match &pattern.kind {
            PatternKind::Binding(name) => self.reserve_binding(name.span),
            PatternKind::Variant { payload, .. } => {
                for sub in payload.iter().flatten() {
                    self.declare_pattern_bindings(sub);
                }
            }
            PatternKind::Struct { fields, .. } => {
                for field in fields {
                    match &field.pattern {
                        Some(sub) => self.declare_pattern_bindings(sub),
                        None => self.reserve_binding(field.name.span),
                    }
                }
            }
            PatternKind::Tuple(elements) | PatternKind::Or(elements) => {
                for element in elements {
                    self.declare_pattern_bindings(element);
                }
            }
            _ => {}
        }
    }

    fn reserve_binding(&mut self, span: Span) {
        let Some(local) = self.declared_local(span) else {
            return;
        };
        if self.storage.contains_key(&local) {
            return;
        }
        let ty = self.local_type(local);
        let repr = self.ir_type(ty);
        if self.is_captured(local) {
            let slot = self.function.add_slot_for(IrType::Ptr, span);
            self.storage
                .insert(local, Storage::Boxed { slot, ty: repr });
            let empty = self.zero(repr, span);
            let boxed = self.box_value(empty, repr, span);
            let address = self.value(InstKind::SlotAddr(slot), span);
            self.store(address, 0, boxed, span);
        } else {
            let slot = self.function.add_slot_for(repr, span);
            self.storage
                .insert(local, Storage::Direct { slot, ty: repr });
        }
    }

    fn test_pattern(
        &mut self,
        pattern: &'c Pattern,
        value: Value,
        ty: TypeId,
        success: BlockRef,
        failure: BlockRef,
    ) {
        let span = pattern.span;

        // pattern co cau truc ma nam duoi `T?` thi khong duoc chay tren con
        // tro null, nen null bi lai sang `failure` truoc, phan con lai cua
        // pattern chi nhin thay
        if let TypeKind::Optional(inner) = self.kind_of(ty) {
            let structural = !matches!(
                pattern.kind,
                PatternKind::Null
                    | PatternKind::Wildcard
                    | PatternKind::Binding(_)
                    | PatternKind::Or(_)
            );
            if structural {
                let null = self.const_null(span);
                let is_null = self.compare(CompareOp::IEq, value, null, span);
                let present = self.new_block(span);
                self.branch(is_null, failure, present);

                self.switch_to(present);
                let repr = self.ir_type(inner);
                let unwrapped = if repr.is_pointer() {
                    value
                } else {
                    self.unbox_value(value, repr, span)
                };
                self.test_pattern(pattern, unwrapped, inner, success, failure);
                return;
            }
        }

        match &pattern.kind {
            PatternKind::Wildcard => self.jump(success, Vec::new()),
            PatternKind::Binding(name) => {
                self.bind_pattern_name(name.span, value, ty);
                self.jump(success, Vec::new());
            }
            PatternKind::Null => {
                let null = self.const_null(span);
                let is_null = self.compare(CompareOp::IEq, value, null, span);
                self.branch(is_null, success, failure);
            }
            PatternKind::Bool(expected) => {
                let expected = self.const_bool(*expected, span);
                let equal = self.compare(CompareOp::IEq, value, expected, span);
                self.branch(equal, success, failure);
            }
            PatternKind::Int {
                magnitude,
                negative,
            } => {
                let expected = signed_literal(*magnitude, *negative);
                let expected = self.const_int(expected, span);
                let equal = self.compare(CompareOp::IEq, value, expected, span);
                self.branch(equal, success, failure);
            }
            PatternKind::Char(expected) => {
                let expected = self.value(InstKind::ConstChar(*expected as u32), span);
                let equal = self.compare(CompareOp::IEq, value, expected, span);
                self.branch(equal, success, failure);
            }
            PatternKind::Str(text) => {
                let expected = self.const_string(text, span);
                let equal = self.runtime_value(RuntimeFn::StringEq, vec![value, expected], span);
                self.branch(equal, success, failure);
            }
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                let unsigned = matches!(self.kind_of(ty), TypeKind::Uint | TypeKind::Char);
                let low = self.endpoint(*start, span);
                let above = if unsigned {
                    CompareOp::UGe
                } else {
                    CompareOp::SGe
                };
                let at_or_above = self.compare(above, value, low, span);
                let upper = self.new_block(span);
                self.branch(at_or_above, upper, failure);

                self.switch_to(upper);
                let high = self.endpoint(*end, span);
                let below = match (inclusive, unsigned) {
                    (true, true) => CompareOp::ULe,
                    (true, false) => CompareOp::SLe,
                    (false, true) => CompareOp::ULt,
                    (false, false) => CompareOp::SLt,
                };
                let within = self.compare(below, value, high, span);
                self.branch(within, success, failure);
            }
            PatternKind::Tuple(elements) => {
                let TypeKind::Tuple(element_types) = self.kind_of(ty) else {
                    self.lo
                        .unsupported(span, "a tuple pattern on a value that is not a tuple");
                    self.jump(failure, Vec::new());
                    return;
                };
                let layout = abi::layout_tuple(&self.lo.types, &element_types);
                let fields: Vec<PatternField<'c>> = elements
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sub)| {
                        let field = layout.fields.get(index)?;
                        Some((field.offset, field.ty, element_types[index], sub))
                    })
                    .collect();
                self.test_fields(&fields, value, success, failure);
            }
            PatternKind::Variant {
                variant, payload, ..
            } => self.test_variant(
                pattern,
                variant,
                payload.as_deref(),
                value,
                ty,
                success,
                failure,
            ),
            PatternKind::Struct { fields, .. } => {
                self.test_struct(pattern, fields, value, ty, success, failure)
            }
            PatternKind::Or(alternatives) => {
                if alternatives.is_empty() {
                    self.jump(failure, Vec::new());
                    return;
                }
                for (index, alternative) in alternatives.iter().enumerate() {
                    let last = index + 1 == alternatives.len();
                    let next = if last {
                        failure
                    } else {
                        self.new_block(alternative.span)
                    };
                    self.test_pattern(alternative, value, ty, success, next);
                    if !last {
                        self.switch_to(next);
                    }
                }
            }
        }
    }

    fn test_variant(
        &mut self,
        pattern: &'c Pattern,
        variant: &'c Ident,
        payload: Option<&'c [Pattern]>,
        value: Value,
        ty: TypeId,
        success: BlockRef,
        failure: BlockRef,
    ) {
        let span = pattern.span;
        let Some(def) = self.pattern_def(pattern, ty) else {
            self.lo
                .unsupported(span, "a variant pattern on an unresolved type");
            self.jump(failure, Vec::new());
            return;
        };
        let index = self
            .checked()
            .context()
            .def(def)
            .as_enum()
            .and_then(|enumeration| enumeration.variant_index(&variant.name));
        let Some(index) = index else {
            let name = variant.name.clone();
            self.lo.unsupported(span, format!("the variant `{name}`"));
            self.jump(failure, Vec::new());
            return;
        };

        let tag = self.load(value, abi::enumeration::TAG_OFFSET, IrType::I32, span);
        let expected = self.const_u32(index as u32, span);
        let matched = self.compare(CompareOp::IEq, tag, expected, span);

        let subpatterns = payload.unwrap_or(&[]);
        if subpatterns.is_empty() {
            self.branch(matched, success, failure);
            return;
        }

        let unpack = self.new_block(span);
        self.branch(matched, unpack, failure);
        self.switch_to(unpack);

        let arguments = self
            .lo
            .named_parts(ty)
            .map(|(_, args)| args)
            .unwrap_or_default();
        let payload_types = self.lo.variant_payload(def, &arguments, index);
        let layout = abi::layout_variant(&self.lo.types, &payload_types);
        let fields: Vec<PatternField<'c>> = subpatterns
            .iter()
            .enumerate()
            .filter_map(|(position, sub)| {
                let field = layout.fields.get(position)?;
                Some((field.offset, field.ty, payload_types[position], sub))
            })
            .collect();
        self.test_fields(&fields, value, success, failure);
    }

    fn test_struct(
        &mut self,
        pattern: &'c Pattern,
        fields: &'c [FieldPattern],
        value: Value,
        ty: TypeId,
        success: BlockRef,
        failure: BlockRef,
    ) {
        let span = pattern.span;
        let Some(def) = self.pattern_def(pattern, ty) else {
            self.lo
                .unsupported(span, "a struct pattern on an unresolved type");
            self.jump(failure, Vec::new());
            return;
        };
        let arguments = self
            .lo
            .named_parts(ty)
            .map(|(_, args)| args)
            .unwrap_or_default();
        let field_types = self.lo.struct_fields(def, &arguments);
        let layout = abi::layout_struct(&self.lo.types, &field_types);
        let names: Vec<String> = match self.checked().context().def(def).as_struct() {
            Some(structure) => structure.fields.iter().map(|f| f.name.clone()).collect(),
            None => Vec::new(),
        };

        let mut tested: Vec<PatternField<'c>> = Vec::new();
        for written in fields {
            let Some(index) = names.iter().position(|name| *name == written.name.name) else {
                continue;
            };
            let Some(field) = layout.fields.get(index) else {
                continue;
            };
            match &written.pattern {
                Some(sub) => tested.push((field.offset, field.ty, field_types[index], sub)),
                None => {
                    let extracted = self.load(value, field.offset, field.ty, written.span);
                    self.bind_pattern_name(written.name.span, extracted, field_types[index]);
                }
            }
        }
        self.test_fields(&tested, value, success, failure);
    }

    fn test_fields(
        &mut self,
        fields: &[PatternField<'c>],
        object: Value,
        success: BlockRef,
        failure: BlockRef,
    ) {
        let Some(((offset, repr, ty, pattern), rest)) = fields.split_first() else {
            self.jump(success, Vec::new());
            return;
        };
        let extracted = self.load(object, *offset, *repr, pattern.span);
        if rest.is_empty() {
            self.test_pattern(pattern, extracted, *ty, success, failure);
            return;
        }
        let next = self.new_block(pattern.span);
        self.test_pattern(pattern, extracted, *ty, next, failure);
        self.switch_to(next);
        self.test_fields(rest, object, success, failure);
    }

    fn bind_pattern_name(&mut self, span: Span, value: Value, ty: TypeId) {
        let Some(local) = self.declared_local(span) else {
            return;
        };
        let declared = self.local_type(local);
        let value = self.coerce(value, ty, declared, span);
        self.write_binding(local, value, span);
    }

    fn pattern_def(&mut self, pattern: &'c Pattern, ty: TypeId) -> Option<DefId> {
        if let Some(def) = self.checked().resolution.pattern_defs.get(&pattern.id) {
            return Some(*def);
        }
        self.lo.named_parts(ty).map(|(def, _)| def)
    }

    fn endpoint(&mut self, endpoint: RangeEndpoint, span: Span) -> Value {
        match endpoint {
            RangeEndpoint::Int {
                magnitude,
                negative,
            } => {
                let literal = signed_literal(magnitude, negative);
                self.const_int(literal, span)
            }
            RangeEndpoint::Char(scalar) => self.value(InstKind::ConstChar(scalar as u32), span),
        }
    }

    // ---- bieu thuc ...

    fn lower_expr(&mut self, expr: &'c Expr) -> Option<Value> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Int(magnitude) => {
                let ty = self.ty_of(expr.id);
                Some(match self.kind_of(ty) {
                    // so nguyen chi vao duoc cho can float qua mot chu thich
                    // cai nay kieu ma checker
                    TypeKind::Float | TypeKind::UntypedFloat => {
                        self.value(InstKind::ConstFloat(*magnitude as f64), span)
                    }
                    _ => self.const_int(*magnitude as i64, span),
                })
            }
            ExprKind::Float(literal) => Some(self.value(InstKind::ConstFloat(*literal), span)),
            ExprKind::Char(scalar) => Some(self.value(InstKind::ConstChar(*scalar as u32), span)),
            ExprKind::Bool(literal) => Some(self.const_bool(*literal, span)),
            ExprKind::Str(literal) => Some(self.lower_string_literal(literal, span)),
            ExprKind::Null => Some(self.const_null(span)),
            ExprKind::This => match self.this {
                Some(this) => Some(this),
                None => {
                    self.lo.unsupported(span, "`this` outside a method");
                    None
                }
            },
            ExprKind::Ident(_) => self.lower_ident(expr, span),
            ExprKind::Array(elements) => Some(self.lower_array_literal(expr, elements, span)),
            ExprKind::Map(entries) => Some(self.lower_map_literal(expr, entries, span)),
            ExprKind::Set(elements) => Some(self.lower_set_literal(expr, elements, span)),
            ExprKind::Tuple(elements) => self.lower_tuple_literal(expr, elements, span),
            ExprKind::Group(inner) => self.lower_expr(inner),
            ExprKind::Closure(closure) => self.lower_closure(expr, closure, span),
            ExprKind::Unary { op, operand } => Some(self.lower_unary(*op, operand, span)),
            ExprKind::Binary { op, lhs, rhs } => Some(self.lower_binary(*op, lhs, rhs, span)),
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => Some(self.lower_range(start, end, *inclusive, span)),
            ExprKind::Catch { operand, handler } => self.lower_catch(expr, operand, handler, span),
            ExprKind::Field { base, name } => self.lower_field(expr, base, name, span),
            ExprKind::TupleField { base, index, .. } => self.lower_tuple_field(base, *index, span),
            ExprKind::Call { callee, args } => self.lower_call(expr, callee, args, span),
            ExprKind::Index { base, index } => self.lower_index(expr, base, index, span),
            ExprKind::NullPropagate(operand) => self.lower_null_propagate(expr, operand, span),
            ExprKind::ErrorPropagate(operand) => self.lower_error_propagate(operand, span),
            ExprKind::TypeArgs { base, .. } => self.lower_expr(base),
            ExprKind::StructLit(literal) => self.lower_struct_literal(expr, literal, span),
        }
    }

    fn lower_operand(&mut self, expr: &'c Expr) -> Value {
        match self.lower_expr(expr) {
            Some(value) => value,
            None => {
                let ty = self.ty_of(expr.id);
                self.zero_of(ty, expr.span)
            }
        }
    }

    fn lower_coerced(&mut self, expr: &'c Expr, target: TypeId) -> Value {
        let from = self.ty_of(expr.id);
        let value = self.lower_operand(expr);
        self.coerce(value, from, target, expr.span)
    }

    fn zero_of(&mut self, ty: TypeId, span: Span) -> Value {
        let repr = self.ir_type(ty);
        self.zero(repr, span)
    }

    fn const_u32(&mut self, literal: u32, span: Span) -> Value {
        self.value(InstKind::ConstChar(literal), span)
    }

    fn type_id_of(&mut self, ty: TypeId, span: Span) -> Value {
        let type_id = self.lo.descriptor_for(ty);
        self.const_u32(type_id.0, span)
    }

    // cai nay  ---- literal

    fn lower_string_literal(&mut self, literal: &'c StringLit, span: Span) -> Value {
        if let Some(text) = literal.as_plain() {
            return self.const_string(&text, span);
        }

        let mut result: Option<Value> = None;
        for part in &literal.parts {
            let piece = match part {
                StringPart::Text { value, span } => self.const_string(value, *span),
                StringPart::Interp(expr) => {
                    let ty = self.ty_of(expr.id);
                    let value = self.lower_operand(expr);
                    self.stringify(value, ty, expr.span)
                }
            };
            result = Some(match result {
                None => piece,
                Some(prefix) => {
                    self.runtime_value(RuntimeFn::StringConcat, vec![prefix, piece], span)
                }
            });
        }
        match result {
            Some(value) => value,
            None => self.const_string("", span),
        }
    }

    fn stringify(&mut self, value: Value, ty: TypeId, span: Span) -> Value {
        let ty = self.apply(ty);
        match self.kind_of(ty) {
            TypeKind::String => value,
            TypeKind::Bool => self.runtime_value(RuntimeFn::StringFromBool, vec![value], span),
            TypeKind::Char => self.runtime_value(RuntimeFn::StringFromChar, vec![value], span),
            TypeKind::Int | TypeKind::UntypedInt => {
                self.runtime_value(RuntimeFn::StringFromInt, vec![value], span)
            }
            TypeKind::Uint => self.runtime_value(RuntimeFn::StringFromUint, vec![value], span),
            TypeKind::Float | TypeKind::UntypedFloat => {
                self.runtime_value(RuntimeFn::StringFromFloat, vec![value], span)
            }
            _ => self.to_string_call(value, ty, span),
        }
    }

    fn to_string_call(&mut self, value: Value, ty: TypeId, span: Span) -> Value {
        let declared = self.interface_of(ty);
        if declared.is_none() {
            if let Some((def, arguments)) = self.lo.named_parts(ty) {
                if let Some(func) = self.checked().context().find_method(def, "to_string") {
                    let handle = self.lo.instance(func, arguments, span);
                    return self
                        .call(
                            InstKind::Call {
                                func: handle,
                                args: vec![value],
                            },
                            Some(IrType::Ptr),
                            span,
                        )
                        .unwrap_or(value);
                }
            }
        }

        let stringable = self.checked().resolution.prelude.stringable;
        let (object, interface) = match declared {
            Some(interface) => (value, interface),
            None => (self.make_interface(value, ty, stringable, span), stringable),
        };
        let Some(slot) = self.interface_slot(interface, "to_string") else {
            let shown = self.lo.types.display(ty);
            self.lo
                .unsupported(span, format!("`{shown}` has no `to_string`"));
            return value;
        };
        let sig = self
            .lo
            .program
            .add_signature(Signature::new(vec![IrType::Ptr], Some(IrType::Ptr)));
        self.call( InstKind::CallInterface { object, slot, sig, args: Vec::new(), }, Some(IrType::Ptr), span, )
        .unwrap_or(value)
    }

    fn interface_slot(&self, interface: DefId, name: &str) -> Option<u32> {
        let context = self.checked().context();
        let methods = &context.def(interface).as_interface()?.methods;
        methods
            .iter()
            .position(|func| context.func(*func).name == name)
            .map(|slot| slot as u32)
    }

    fn lower_array_literal(&mut self, expr: &'c Expr, elements: &'c [Expr], span: Span) -> Value {
        let ty = self.ty_of(expr.id);
        let element_type = self.element_of(ty);
        let repr = self.ir_type(element_type);
        let mut values = Vec::with_capacity(elements.len());
        for element in elements {
            let value = self.lower_coerced(element, element_type);
            values.push(self.widen(value, repr, element.span));
        }
        self.build_array(ty, values, span)
    }

    fn build_array(&mut self, ty: TypeId, elements: Vec<Value>, span: Span) -> Value {
        let descriptor = self.type_id_of(ty, span);
        let capacity = self.const_int(elements.len() as i64, span);
        let array = self.runtime_value(RuntimeFn::ArrayNew, vec![descriptor, capacity], span);
        for element in elements {
            self.runtime(RuntimeFn::ArrayPush, vec![array, element], span);
        }
        array
    }

    fn lower_map_literal(&mut self, expr: &'c Expr, entries: &'c [MapEntry], span: Span) -> Value {
        let ty = self.ty_of(expr.id);
        // cai nay `{}` rong ma
        // phan biet duoc hai cai, nen kieu da ghi lai moi quyet dinh dung
        // cai nao.
        if entries.is_empty() && matches!(self.kind_of(ty), TypeKind::Set(_)) {
            return self.lower_set_literal(expr, &[], span);
        }
        let (key_type, value_type) = match self.kind_of(ty) {
            TypeKind::Map { key, value } => (key, value),
            _ => (TypeId::ERROR, TypeId::ERROR),
        };
        let key_repr = self.ir_type(key_type);
        let value_repr = self.ir_type(value_type);

        let mut pairs = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = self.lower_coerced(&entry.key, key_type);
            let key = self.widen(key, key_repr, entry.span);
            let value = self.lower_coerced(&entry.value, value_type);
            let value = self.widen(value, value_repr, entry.span);
            pairs.push((key, value));
        }

        let descriptor = self.type_id_of(ty, span);
        let map = self.runtime_value(RuntimeFn::MapNew, vec![descriptor], span);
        for (key, value) in pairs {
            self.runtime(RuntimeFn::MapSet, vec![map, key, value], span);
        }
        map
    }

    fn lower_set_literal(&mut self, expr: &'c Expr, elements: &'c [Expr], span: Span) -> Value {
        let ty = self.ty_of(expr.id);
        let element_type = self.element_of(ty);
        let repr = self.ir_type(element_type);

        let mut values = Vec::with_capacity(elements.len());
        for element in elements {
            let value = self.lower_coerced(element, element_type);
            values.push(self.widen(value, repr, element.span));
        }

        let descriptor = self.type_id_of(ty, span);
        let set = self.runtime_value(RuntimeFn::SetNew, vec![descriptor], span);
        for value in values {
            self.runtime(RuntimeFn::SetAdd, vec![set, value], span);
        }
        set
    }

    fn lower_tuple_literal(
        &mut self,
        expr: &'c Expr,
        elements: &'c [Expr],
        span: Span,
    ) -> Option<Value> {
        let ty = self.ty_of(expr.id);
        let TypeKind::Tuple(element_types) = self.kind_of(ty) else {
            self.lo.unsupported(span, "a tuple literal of unknown type");
            return None;
        };
        let mut values = Vec::with_capacity(elements.len());
        for (element, target) in elements.iter().zip(&element_types) {
            values.push(self.lower_coerced(element, *target));
        }
        Some(self.build_record(ty, &element_types, &values, span))
    }

    fn build_record(
        &mut self,
        ty: TypeId,
        field_types: &[TypeId],
        values: &[Value],
        span: Span,
    ) -> Value {
        let type_id = self.lo.descriptor_for(ty);
        let layout = abi::layout_struct(&self.lo.types, field_types);
        let object = self.value(
            InstKind::Alloc {
                type_id,
                size: layout.size,
            },
            span,
        );
        for (field, value) in layout.fields.iter().zip(values) {
            self.store(object, field.offset, *value, span);
        }
        object
    }

    fn build_variant(
        &mut self,
        ty: TypeId,
        tag: u32,
        payload_types: &[TypeId],
        values: &[Value],
        span: Span,
    ) -> Value {
        let type_id = self.lo.descriptor_for(ty);
        if payload_types.is_empty() {
            self.lo.enum_singleton(ty, type_id, tag);
            return self.value(InstKind::ConstEnumSingleton { type_id, tag }, span);
        }

        let layout = abi::layout_variant(&self.lo.types, payload_types);
        let object = self.value(
            InstKind::Alloc {
                type_id,
                size: layout.size,
            },
            span,
        );
        let tag_value = self.const_u32(tag, span);
        self.store(object, abi::enumeration::TAG_OFFSET, tag_value, span);
        for (field, value) in layout.fields.iter().zip(values) {
            self.store(object, field.offset, *value, span);
        }
        object
    }

    fn lower_struct_literal(
        &mut self,
        expr: &'c Expr,
        literal: &'c StructLit,
        span: Span,
    ) -> Option<Value> {
        let ty = self.ty_of(expr.id);
        let Some((def, arguments)) = self.lo.named_parts(ty) else {
            self.lo
                .unsupported(span, "a struct literal of unknown type");
            return None;
        };
        let field_types = self.lo.struct_fields(def, &arguments);
        let names: Vec<String> = match self.checked().context().def(def).as_struct() {
            Some(structure) => structure.fields.iter().map(|f| f.name.clone()).collect(),
            None => Vec::new(),
        };

        let mut written: Vec<Option<Value>> = vec![None; field_types.len()];
        for initialiser in &literal.fields {
            let Some(index) = names.iter().position(|name| *name == initialiser.name.name) else {
                continue;
            };
            written[index] = Some(self.lower_coerced(&initialiser.value, field_types[index]));
        }

        let mut values = Vec::with_capacity(field_types.len());
        for (index, value) in written.into_iter().enumerate() {
            values.push(match value {
                Some(value) => value,
                // 14.13 bat phai co du field,
                // bao loi tu truoc roi.
                None => self.zero_of(field_types[index], span),
            });
        }
        Some(self.build_record(ty, &field_types, &values, span))
    }

    fn lower_range(
        &mut self,
        start: &'c Expr,
        end: &'c Expr,
        inclusive: bool,
        span: Span,
    ) -> Value {
        let start = self.lower_coerced(start, TypeId::INT);
        let end = self.lower_coerced(end, TypeId::INT);
        let inclusive = self.const_bool(inclusive, span);
        let range = self.checked().resolution.prelude.range_type;
        let range = self.apply(range);
        self.build_record(
            range,
            &[TypeId::INT, TypeId::INT, TypeId::BOOL],
            &[start, end, inclusive],
            span,
        )
    }

    // ---- ten ----

    fn lower_ident(&mut self, expr: &'c Expr, span: Span) -> Option<Value> {
        let binding = self.checked().resolution.values.get(&expr.id).copied();
        match binding {
            Some(ValueBinding::Local(local)) | Some(ValueBinding::Captured(local)) => {
                match self.read_binding(local, span) {
                    Some(value) => {
                        // cho chua luon giu dung
                        // thu hep null (D-25) cho lan dung nay kieu da boc
                        // thi primitive optional phai lay ra khoi hop. Xem
                        // docs/abi.md muc 2.1. ...
                        let declared = self.local_type(local);
                        let narrowed = self.ty_of(expr.id);
                        Some(self.unwrap_narrowed(value, declared, narrowed, span))
                    }
                    None => {
                        self.lo
                            .unsupported(span, "a read of a binding with no storage");
                        None
                    }
                }
            }
            Some(ValueBinding::Field { owner, index }) => {
                let Some(this) = self.this else {
                    self.lo
                        .unsupported(span, "a field reference outside a method");
                    return None;
                };
                let this_ty = self.this_type(owner);
                let (offset, repr) = self.field_layout_of(this_ty, owner, index, span)?;
                Some(self.load(this, offset, repr, span))
            }
            Some(ValueBinding::GlobalConst(global)) => Some(self.read_global(global, expr, span)),
            Some(ValueBinding::Function(func)) => self.function_value(func, expr, span),
            other => {
                let what = match other {
                    Some(ValueBinding::Method(_)) => "a bare method reference",
                    Some(ValueBinding::Module(_)) => "a module used as a value",
                    Some(ValueBinding::Type(_)) => "a type used as a value",
                    Some(ValueBinding::Predeclared(_)) => "a builtin used as a value",
                    Some(ValueBinding::Conversion(_)) => "a conversion used as a value",
                    _ => "an unresolved name",
                };
                self.lo.unsupported(span, what);
                None
            }
        }
    }

    fn read_global(&mut self, global: GlobalConstId, expr: &'c Expr, span: Span) -> Value {
        let ty = self.ty_of(expr.id);
        let repr = self.ir_type(ty);
        let address = self.value(InstKind::GlobalAddr(GlobalRef(global.0)), span);
        self.load(address, 0, repr, span)
    }

    fn function_value(&mut self, func: FuncId, expr: &'c Expr, span: Span) -> Option<Value> {
        let ty = self.ty_of(expr.id);
        if !matches!(self.kind_of(ty), TypeKind::Function(_)) {
            self.lo
                .unsupported(span, "a function value of non-function type");
            return None;
        }
        let target = self.lo.instance(func, Vec::new(), span);
        let thunk = self.lo.thunk(target);
        Some(self.build_closure(ty, thunk, &[], span))
    }

    fn lower_field(
        &mut self,
        expr: &'c Expr,
        base: &'c Expr,
        name: &'c Ident,
        span: Span,
    ) -> Option<Value> {
        match self.checked().field_accesses.get(&expr.id).copied() {
            Some(FieldAccess::Field { owner, index }) => {
                let object_ty = self.ty_of(base.id);
                let object = self.lower_operand(base);
                let (offset, repr) = self.field_layout_of(object_ty, owner, index, span)?;
                Some(self.load(object, offset, repr, span))
            }
            Some(FieldAccess::Length) => {
                let container_type = self.ty_of(base.id);
                let container = self.lower_operand(base);
                let entry = match self.kind_of(container_type) {
                    TypeKind::String => RuntimeFn::StringLen,
                    TypeKind::Array(_) => RuntimeFn::ArrayLen,
                    TypeKind::Map { .. } => RuntimeFn::MapLen,
                    TypeKind::Set(_) => RuntimeFn::SetLen,
                    _ => {
                        let shown = self.lo.types.display(container_type);
                        self.lo.unsupported(span, format!("`length` on `{shown}`"));
                        return None;
                    }
                };
                Some(self.runtime_value(entry, vec![container], span))
            }
            None => self.lower_qualified(expr, name, span),
        }
    }

    fn lower_qualified(&mut self, expr: &'c Expr, name: &'c Ident, span: Span) -> Option<Value> {
        match self.checked().resolution.values.get(&expr.id).copied() {
            Some(ValueBinding::GlobalConst(global)) => {
                return Some(self.read_global(global, expr, span))
            }
            Some(ValueBinding::Function(func)) => return self.function_value(func, expr, span),
            _ => {}
        }

        let ty = self.ty_of(expr.id);
        if let Some((def, _)) = self.lo.named_parts(ty) {
            let variant = self
                .checked()
                .context()
                .def(def)
                .as_enum()
                .and_then(|enumeration| enumeration.variant_index(&name.name));
            if let Some(variant) = variant {
                return Some(self.build_variant(ty, variant as u32, &[], &[], span));
            }
        }

        let shown = name.name.clone();
        self.lo
            .unsupported(span, format!("a reference to `{shown}`"));
        None
    }

    fn lower_tuple_field(&mut self, base: &'c Expr, index: u32, span: Span) -> Option<Value> {
        let base_type = self.ty_of(base.id);
        let TypeKind::Tuple(elements) = self.kind_of(base_type) else {
            self.lo
                .unsupported(span, "a tuple index on a value that is not a tuple");
            return None;
        };
        let object = self.lower_operand(base);
        let layout = abi::layout_tuple(&self.lo.types, &elements);
        let Some(field) = layout.fields.get(index as usize) else {
            self.lo.unsupported(span, "a tuple index outside the tuple");
            return None;
        };
        Some(self.load(object, field.offset, field.ty, span))
    }

    // cai nay  ---- toan

    fn lower_unary(&mut self, op: SourceUnaryOp, operand: &'c Expr, span: Span) -> Value {
        let ty = self.ty_of(operand.id);
        let value = self.lower_operand(operand);
        let op = match op {
            SourceUnaryOp::Not => UnaryOp::Not,
            SourceUnaryOp::Neg => match self.kind_of(ty) {
                TypeKind::Float | TypeKind::UntypedFloat => UnaryOp::FNeg,
                _ => UnaryOp::INeg,
            },
        };
        self.value(InstKind::Unary { op, value }, span)
    }

    fn lower_binary( &mut self, op: SourceBinaryOp, lhs: &'c Expr, rhs: &'c Expr, span: Span, ) -> Value {
        if op.is_logical() {
            return self.lower_short_circuit(op, lhs, rhs, span);
        }

        let left_type = self.ty_of(lhs.id);
        let right_type = self.ty_of(rhs.id);
        // `null` va cai ...
        // duoc, nen ben kia quyet dinh phep so sanh.
        let operand_type = if matches!(self.kind_of(left_type), TypeKind::Error | TypeKind::Never) {
            right_type
        } else {
            left_type
        };

        let left = self.lower_coerced(lhs, operand_type);
        let right = self.lower_coerced(rhs, operand_type);
        if op.is_comparison() {
            // co `null` viet thang o mot ben thi day la phep hoi co hay
            // khong, so mot tu la xong; di theo luat optional chung se dung
            // ra mot cai hinh thoi thua quanh dung mot lenh do.
            if is_null_literal(lhs) || is_null_literal(rhs) {
                let inverted = matches!(op, SourceBinaryOp::Ne);
                let equal = self.compare(CompareOp::IEq, left, right, span);
                return self.negate_if(equal, inverted, span);
            }
            self.comparison(op, left, right, operand_type, span)
        } else {
            self.arithmetic(op, left, right, operand_type, span)
        }
    }

    fn lower_short_circuit(
        &mut self,
        op: SourceBinaryOp,
        lhs: &'c Expr,
        rhs: &'c Expr,
        span: Span,
    ) -> Value {
        let left = self.lower_operand(lhs);
        let rest = self.new_block(rhs.span);
        let join = self.new_block(span);
        let result = self.function.add_block_param(join, IrType::I8);

        // `||` ngat som khi true, `&&` ngat som khi false, va ca hai truong
        // hop cau tra loi chinh la gia tri ma ve trai da co.
        let disjunction = matches!(op, SourceBinaryOp::Or);
        let shortcut = self.const_bool(disjunction, span);
        let terminator = if disjunction {
            Terminator::Branch {
                cond: left,
                then_block: join,
                then_args: vec![shortcut],
                else_block: rest,
                else_args: Vec::new(),
            }
        } else {
            Terminator::Branch {
                cond: left,
                then_block: rest,
                then_args: Vec::new(),
                else_block: join,
                else_args: vec![shortcut],
            }
        };
        self.seal(terminator);

        self.switch_to(rest);
        let right = self.lower_operand(rhs);
        self.jump(join, vec![right]);

        self.switch_to(join);
        result
    }

    fn arithmetic(
        &mut self,
        op: SourceBinaryOp,
        lhs: Value,
        rhs: Value,
        ty: TypeId,
        span: Span,
    ) -> Value {
        let kind = self.kind_of(ty);
        let float = matches!(kind, TypeKind::Float | TypeKind::UntypedFloat);
        let unsigned = matches!(kind, TypeKind::Uint);

        if matches!(op, SourceBinaryOp::Add) && matches!(kind, TypeKind::String) {
            return self.runtime_value(RuntimeFn::StringConcat, vec![lhs, rhs], span);
        }

        let op = match op {
            SourceBinaryOp::Add if float => BinaryOp::FAdd,
            SourceBinaryOp::Add => BinaryOp::IAdd,
            SourceBinaryOp::Sub if float => BinaryOp::FSub,
            SourceBinaryOp::Sub => BinaryOp::ISub,
            SourceBinaryOp::Mul if float => BinaryOp::FMul,
            SourceBinaryOp::Mul => BinaryOp::IMul,
            SourceBinaryOp::Div if float => BinaryOp::FDiv,
            SourceBinaryOp::Div => {
                self.push(InstKind::DivisorCheck { divisor: rhs }, span);
                if unsigned {
                    BinaryOp::UDiv
                } else {
                    BinaryOp::SDiv
                }
            }
            SourceBinaryOp::Rem if float => BinaryOp::FRem,
            SourceBinaryOp::Rem => {
                self.push(InstKind::DivisorCheck { divisor: rhs }, span);
                if unsigned {
                    BinaryOp::URem
                } else {
                    BinaryOp::SRem
                }
            }
            SourceBinaryOp::BitAnd => BinaryOp::BitAnd,
            SourceBinaryOp::BitOr => BinaryOp::BitOr,
            SourceBinaryOp::BitXor => BinaryOp::BitXor,
            SourceBinaryOp::Shl => {
                self.shift_guard(rhs, unsigned, span);
                BinaryOp::Shl
            }
            SourceBinaryOp::Shr => {
                self.shift_guard(rhs, unsigned, span);
                if unsigned {
                    BinaryOp::LShr
                } else {
                    BinaryOp::AShr
                }
            }
            other => {
                let shown = self.lo.types.display(ty);
                let spelling = other.spelling();
                self.lo
                    .unsupported(span, format!("`{spelling}` on `{shown}`"));
                return lhs;
            }
        };
        self.value(InstKind::Binary { op, lhs, rhs }, span)
    }

    fn shift_guard(&mut self, count: Value, unsigned: bool, span: Span) {
        if !unsigned {
            self.push(InstKind::ShiftCountCheck { count }, span);
        }
    }

    fn comparison(
        &mut self,
        op: SourceBinaryOp,
        lhs: Value,
        rhs: Value,
        ty: TypeId,
        span: Span,
    ) -> Value {
        let equality = matches!(op, SourceBinaryOp::Eq | SourceBinaryOp::Ne);
        let inverted = matches!(op, SourceBinaryOp::Ne);
        match self.kind_of(ty) {
            TypeKind::String if equality => {
                let equal = self.runtime_value(RuntimeFn::StringEq, vec![lhs, rhs], span);
                self.negate_if(equal, inverted, span)
            }
            TypeKind::String => {
                let order = self.runtime_value(RuntimeFn::StringCmp, vec![lhs, rhs], span);
                let zero = self.const_int(0, span);
                self.compare(integer_compare(op, false), order, zero, span)
            }
            TypeKind::Tuple(_) if equality => {
                let equal = self.structural_eq(lhs, rhs, ty, span);
                self.negate_if(equal, inverted, span)
            }
            TypeKind::Optional(inner) if equality => {
                let equal = self.optional_eq(lhs, rhs, inner, span);
                self.negate_if(equal, inverted, span)
            }
            TypeKind::Float | TypeKind::UntypedFloat => {
                self.compare(float_compare(op), lhs, rhs, span)
            }
            TypeKind::Uint | TypeKind::Char => {
                self.compare(integer_compare(op, true), lhs, rhs, span)
            }
            // so nguyen, bool, va moi tham chieu. Voi cai gi khong phai
            // primitive, chuoi hay tuple thi `==` la so cung mot vat.
            _ => self.compare(integer_compare(op, false), lhs, rhs, span),
        }
    }

    fn negate_if(&mut self, value: Value, negate: bool, span: Span) -> Value {
        if negate {
            self.value(
                InstKind::Unary {
                    op: UnaryOp::Not,
                    value,
                },
                span,
            )
        } else {
            value
        }
    }

    fn optional_eq(&mut self, lhs: Value, rhs: Value, inner: TypeId, span: Span) -> Value {
        let null = self.const_null(span);
        let lhs_null = self.compare(CompareOp::IEq, lhs, null, span);
        let rhs_null = self.compare(CompareOp::IEq, rhs, null, span);
        let either_null = self.value(
            InstKind::Binary {
                op: BinaryOp::BitOr,
                lhs: lhs_null,
                rhs: rhs_null,
            },
            span,
        );

        let absent = self.new_block(span);
        let present = self.new_block(span);
        let join = self.new_block(span);
        let result = self.function.add_block_param(join, IrType::I8);
        self.seal(Terminator::Branch {
            cond: either_null,
            then_block: absent,
            then_args: Vec::new(),
            else_block: present,
            else_args: Vec::new(),
        });

        // co it nhat mot ben la null thi so hai tu voi nhau la ra luon:
        // dung khi va chi khi ca hai deu null.
        self.switch_to(absent);
        let identical = self.compare(CompareOp::IEq, lhs, rhs, span);
        self.jump(join, vec![identical]);

        self.switch_to(present);
        let repr = self.ir_type(inner);
        let (left, right) = if repr.is_pointer() {
            (lhs, rhs)
        } else {
            let left = self.unbox_value(lhs, repr, span);
            let right = self.unbox_value(rhs, repr, span);
            (left, right)
        };
        let equal = self.structural_eq(left, right, inner, span);
        self.jump(join, vec![equal]);

        self.switch_to(join);
        result
    }

    fn structural_eq(&mut self, lhs: Value, rhs: Value, ty: TypeId, span: Span) -> Value {
        match self.kind_of(ty) {
            TypeKind::String => self.runtime_value(RuntimeFn::StringEq, vec![lhs, rhs], span),
            TypeKind::Float | TypeKind::UntypedFloat => {
                self.compare(CompareOp::FEq, lhs, rhs, span)
            }
            TypeKind::Tuple(elements) => {
                let layout = abi::layout_tuple(&self.lo.types, &elements);
                let mut result = self.const_bool(true, span);
                for (field, element) in layout.fields.iter().zip(&elements) {
                    let left = self.load(lhs, field.offset, field.ty, span);
                    let right = self.load(rhs, field.offset, field.ty, span);
                    let equal = self.structural_eq(left, right, *element, span);
                    result = self.value(
                        InstKind::Binary {
                            op: BinaryOp::BitAnd,
                            lhs: result,
                            rhs: equal,
                        },
                        span,
                    );
                }
                result
            }
            _ => self.compare(CompareOp::IEq, lhs, rhs, span),
        }
    }

    // ---- lay theo chi so ----

    fn lower_index(
        &mut self,
        expr: &'c Expr,
        base: &'c Expr,
        index: &'c Expr,
        span: Span,
    ) -> Option<Value> {
        let container_type = self.ty_of(base.id);
        let result_type = self.ty_of(expr.id);
        let repr = self.ir_type(result_type);
        let container = self.lower_operand(base);

        match self.kind_of(container_type) {
            TypeKind::Array(_) => {
                let position = self.lower_coerced(index, TypeId::INT);
                let raw = self.runtime_value(RuntimeFn::ArrayGet, vec![container, position], span);
                Some(self.narrow(raw, repr, span))
            }
            TypeKind::Map { key, .. } => {
                let subscript = self.lower_coerced(index, key);
                let subscript = self.widen_as(subscript, key, span);
                let raw = self.runtime_value(RuntimeFn::MapGet, vec![container, subscript], span);
                Some(self.narrow(raw, repr, span))
            }
            _ => {
                let shown = self.lo.types.display(container_type);
                self.lo.unsupported(span, format!("indexing `{shown}`"));
                None
            }
        }
    }

    // ---- lan loi va catch ----

    fn lower_null_propagate(
        &mut self,
        expr: &'c Expr,
        operand: &'c Expr,
        span: Span,
    ) -> Option<Value> {
        let operand_type = self.ty_of(operand.id);
        let value = self.lower_operand(operand);
        let null = self.const_null(span);
        let is_null = self.compare(CompareOp::IEq, value, null, span);

        let absent = self.new_block(span);
        let present = self.new_block(span);
        self.branch(is_null, absent, present);

        self.switch_to(absent);
        let empty = self.return_repr.map(|repr| self.zero(repr, span));
        self.seal(Terminator::Return { value: empty });

        self.switch_to(present);
        let result_type = self.ty_of(expr.id);
        Some(self.coerce(value, operand_type, result_type, span))
    }

    fn lower_error_propagate(&mut self, operand: &'c Expr, span: Span) -> Option<Value> {
        let value = self.lower_expr(operand);
        let pending = self.value(InstKind::ErrorPending, span);

        let failed = self.new_block(span);
        let ok = self.new_block(span);
        self.branch(pending, failed, ok);

        self.switch_to(failed);
        let empty = self.return_repr.map(|repr| self.zero(repr, span));
        self.seal(Terminator::Return { value: empty });

        self.switch_to(ok);
        value
    }

    fn lower_catch(
        &mut self,
        expr: &'c Expr,
        operand: &'c Expr,
        handler: &'c CatchHandler,
        span: Span,
    ) -> Option<Value> {
        let result_type = self.ty_of(expr.id);
        let repr = self.lo.ir_return_type(result_type);
        let value = self.lower_expr(operand);
        let pending = self.value(InstKind::ErrorPending, span);

        let failed = self.new_block(handler.span());
        let join = self.new_block(span);
        let result = repr.map(|ty| self.function.add_block_param(join, ty));

        let success_args = match (repr, value) {
            (Some(_), Some(value)) => vec![value],
            (Some(ty), None) => vec![self.zero(ty, span)],
            (None, _) => Vec::new(),
        };
        self.seal(Terminator::Branch {
            cond: pending,
            then_block: failed,
            then_args: Vec::new(),
            else_block: join,
            else_args: success_args,
        });

        self.switch_to(failed);
        // lay loi ra la xoa o do luon, tra lai dieu bat bien: khong co loi
        // nao dang bay thi o do la null.
        let error = self.value(InstKind::ErrorTake, span);
        match handler {
            CatchHandler::Discard(block) => {
                self.lower_block(block);
                self.leave_handler(join, repr, span);
            }
            CatchHandler::Bind { name, block } => {
                if let Some(local) = self.declared_local(name.span) {
                    let ty = self.local_type(local);
                    self.declare_binding(local, ty, error, name.span);
                }
                self.lower_block(block);
                self.leave_handler(join, repr, span);
            }
            CatchHandler::Value(fallback) => {
                let fallback = self.lower_coerced(fallback, result_type);
                let args = match repr {
                    Some(_) => vec![fallback],
                    None => Vec::new(),
                };
                self.jump(join, args);
            }
        }

        self.switch_to(join);
        result
    }

    fn leave_handler(&mut self, join: BlockRef, repr: Option<IrType>, span: Span) {
        if self.terminated {
            return;
        }
        let args = match repr {
            Some(ty) => vec![self.zero(ty, span)],
            None => Vec::new(),
        };
        self.jump(join, args);
    }

    // ---- closure ----

    fn lower_closure(
        &mut self,
        expr: &'c Expr,
        closure: &'c ClosureExpr,
        span: Span,
    ) -> Option<Value> {
        let ty = self.ty_of(expr.id);
        let TypeKind::Function(signature) = self.kind_of(ty) else {
            self.lo
                .unsupported(span, "a closure whose type is not a function type");
            return None;
        };
        let info = self
            .checked()
            .resolution
            .closures
            .get(&closure.id)
            .cloned()
            .unwrap_or_default();

        // moi truong giu mot cai hop cho moi bien bi capture, theo dung thu
        // tu ma bind_environment doc lai, `this` xep cuoi neu than ham co
        // dung toi.
        let mut environment = Vec::with_capacity(info.captures.len() + 1);
        for local in &info.captures {
            let Some(pointer) = self.box_pointer(*local, span) else {
                self.lo
                    .unsupported(span, "a capture of a binding with no box");
                return None;
            };
            environment.push(pointer);
        }
        let captured_this = match (info.captures_this, self.this) {
            (true, Some(this)) => {
                let boxed = self.box_value(this, IrType::Ptr, span);
                environment.push(boxed);
                Some(info.captures.len() as u32)
            }
            _ => None,
        };

        let mut params = vec![IrType::Ptr];
        for param in &signature.params {
            let param = self.apply(*param);
            params.push(self.ir_type(param));
        }
        if signature.variadic.is_some() {
            params.push(IrType::Ptr);
        }
        let return_type = self.apply(signature.ret);
        let physical = Signature::new(params, self.lo.ir_return_type(return_type));

        let ordinal = self.lo.closure_counter;
        self.lo.closure_counter += 1;
        let name = abi::mangle_function(
            self.checked().context().module_path(self.module),
            None,
            &format!("closure{ordinal}"),
            "",
        );
        let handle = self.lo.declare(
            name,
            physical,
            closure.span,
            signature.failable,
            false,
            return_type,
        );
        self.lo.jobs.push(Job::Closure(Box::new(ClosureJob {
            handle,
            expr: closure,
            ty,
            captures: info.captures.clone(),
            captured_this,
            subst: self.subst.clone(),
            module: self.module,
        })));

        Some(self.build_closure(ty, handle, &environment, span))
    }

    fn build_closure(
        &mut self,
        ty: TypeId,
        code: FuncRef,
        captures: &[Value],
        span: Span,
    ) -> Value {
        let descriptor = self.type_id_of(ty, span);
        let address = self.value(InstKind::ConstFuncAddr(code), span);
        let count = self.const_int(captures.len() as i64, span);
        let closure = self.runtime_value(
            RuntimeFn::ClosureNew,
            vec![descriptor, address, count],
            span,
        );
        for (index, capture) in captures.iter().enumerate() {
            let offset = abi::closure::capture_offset(index as u64) as u32;
            self.store(closure, offset, *capture, span);
        }
        closure
    }

    // ---- goi ham ----

    fn lower_call(
        &mut self,
        expr: &'c Expr,
        callee: &'c Expr,
        args: &'c [Argument],
        span: Span,
    ) -> Option<Value> {
        let Some(resolved) = self.checked().calls.get(&expr.id).cloned() else {
            self.lo
                .unsupported(span, "a call the checker did not resolve");
            return None;
        };
        let operands = call_operands(callee, args);
        let result_type = self.ty_of(expr.id);

        match resolved.callee {
            Callee::Function(func) | Callee::Method { func, .. } => {
                self.lower_direct_call(&resolved, func, &operands, span)
            }
            Callee::Interface { interface, slot } => {
                self.lower_interface_call(&resolved, interface, slot, &operands, result_type, span)
            }
            Callee::Closure => self.lower_closure_call(&resolved, callee, &operands, span),
            Callee::Conversion { target } => {
                self.lower_conversion(&resolved, target, &operands, span)
            }
            Callee::Predeclared(value) => self.lower_predeclared(value, &resolved, &operands, span),
            Callee::Builtin(method) => {
                self.lower_builtin(method, &resolved, &operands, result_type, span)
            }
            Callee::Variant { def, variant } => {
                self.lower_variant_call(&resolved, def, variant, &operands, result_type, span)
            }
        }
    }

    fn lower_direct_call(
        &mut self,
        resolved: &ResolvedCall,
        func: FuncId,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let arguments: Vec<TypeId> = resolved
            .type_arguments
            .iter()
            .map(|argument| self.apply(*argument))
            .collect();
        let parameters = self.physical_parameter_types(func, &arguments);
        let args = self.lower_arguments(&resolved.arguments, &parameters, operands, span);
        let handle = self.lo.instance(func, arguments, span);
        let result = self.lo.program.function(handle).signature.ret;
        self.call(InstKind::Call { func: handle, args }, result, span)
    }

    fn physical_parameter_types(&mut self, func: FuncId, arguments: &[TypeId]) -> Vec<TypeId> {
        let subst = self.lo.substitution_for(func, arguments);
        let definition = self.checked().context().func(func);
        let owner = definition.owner;
        let has_receiver = definition.has_receiver;
        let declared: Vec<(TypeId, bool)> = definition
            .params
            .iter()
            .map(|param| (param.ty, param.variadic))
            .collect();

        let mut types = Vec::with_capacity(declared.len() + 1);
        if has_receiver {
            let receiver = match owner {
                Some(owner) => {
                    let owner_args = subst.owner_args.clone();
                    self.lo.types.named(owner, owner_args)
                }
                None => TypeId::ERROR,
            };
            types.push(receiver);
        }
        for (ty, variadic) in declared {
            let ty = self.lo.apply(ty, &subst);
            types.push(if variadic {
                self.lo.types.array_of(ty)
            } else {
                ty
            });
        }
        types
    }

    fn lower_interface_call(
        &mut self,
        resolved: &ResolvedCall,
        interface: DefId,
        slot: u32,
        operands: &Operands<'c>,
        result_type: TypeId,
        span: Span,
    ) -> Option<Value> {
        let Some(receiver) = resolved
            .arguments
            .first()
            .and_then(|argument| self.bound_expression(argument, operands))
        else {
            self.lo
                .unsupported(span, "an interface call with no receiver");
            return None;
        };
        let receiver_type = self.ty_of(receiver.id);

        // trong mot ban da mono thi receiver cua generic co rang buoc la
        // kieu cu the chu khong phai con tro beo: `fn show<T: Stringable>(x:
        // T)` voi `T = Money` giu mot `Money`, lam gi co tu itable nao ma
        // nap. Bang conformance da chi ro slot do la method nao, nen loi goi
        // giai duoc ngay luc dich.
        if self.interface_of(receiver_type).is_none() {
            return self.lower_devirtualised_call(
                resolved,
                interface,
                slot,
                receiver_type,
                operands,
                result_type,
                span,
            );
        }

        let arguments = self
            .lo
            .named_parts(receiver_type)
            .map(|(_, args)| args)
            .unwrap_or_default();

        let method = self
            .checked()
            .context()
            .def(interface)
            .as_interface()
            .and_then(|definition| definition.methods.get(slot as usize).copied());
        let Some(method) = method else {
            self.lo
                .unsupported(span, "an interface slot with no declared method");
            return None;
        };

        let definition = self.checked().context().func(method);
        let declared: Vec<TypeId> = definition.params.iter().map(|param| param.ty).collect();
        let return_type = definition.ret;

        let mut parameters = vec![receiver_type];
        for ty in declared {
            let ty = self.lo.substitute_owner(ty, interface, &arguments);
            parameters.push(ty);
        }
        let return_type = self.lo.substitute_owner(return_type, interface, &arguments);
        let result = self.lo.ir_return_type(return_type);

        let values = self.lower_arguments(&resolved.arguments, &parameters, operands, span);
        let Some((object, rest)) = values.split_first() else {
            return None;
        };
        let (object, rest) = (*object, rest.to_vec());

        let mut physical = vec![IrType::Ptr];
        for ty in &parameters[1..] {
            physical.push(self.ir_type(*ty));
        }
        let sig = self
            .lo
            .program
            .add_signature(Signature::new(physical, result));
        self.call(
            InstKind::CallInterface {
                object,
                slot,
                sig,
                args: rest,
            },
            result,
            span,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_devirtualised_call(
        &mut self,
        resolved: &ResolvedCall,
        interface: DefId,
        slot: u32,
        receiver_type: TypeId,
        operands: &Operands<'c>,
        result_type: TypeId,
        span: Span,
    ) -> Option<Value> {
        let Some(method) = self.lo.conformance_method(interface, receiver_type, slot) else {
            let shown = self.lo.types.display(receiver_type);
            self.lo.unsupported(
                span,
                format!("`{shown}` has no implementation for this interface method"),
            );
            return None;
        };

        match method {
            ConformanceMethod::User(func) => {
                let arguments = self
                    .lo
                    .named_parts(receiver_type)
                    .map(|(_, args)| args)
                    .unwrap_or_default();
                let parameters = self.physical_parameter_types(func, &arguments);
                let args = self.lower_arguments(&resolved.arguments, &parameters, operands, span);
                let handle = self.lo.instance(func, arguments, span);
                let result = self.lo.program.function(handle).signature.ret;
                self.call(InstKind::Call { func: handle, args }, result, span)
            }
            ConformanceMethod::Builtin(builtin) => {
                self.lower_builtin(builtin, resolved, operands, result_type, span)
            }
        }
    }

    fn lower_closure_call(
        &mut self,
        resolved: &ResolvedCall,
        callee: &'c Expr,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let callee_type = self.ty_of(callee.id);
        let TypeKind::Function(signature) = self.kind_of(callee_type) else {
            self.lo
                .unsupported(span, "a call through a value that is not a closure");
            return None;
        };

        let mut parameters: Vec<TypeId> = Vec::with_capacity(signature.params.len() + 1);
        for param in &signature.params {
            parameters.push(self.apply(*param));
        }
        if let Some(element) = signature.variadic {
            let element = self.apply(element);
            parameters.push(self.lo.types.array_of(element));
        }
        let return_type = self.apply(signature.ret);
        let result = self.lo.ir_return_type(return_type);

        let closure = self.lower_operand(callee);
        let args = self.lower_arguments(&resolved.arguments, &parameters, operands, span);

        let mut physical = vec![IrType::Ptr];
        for ty in &parameters {
            physical.push(self.ir_type(*ty));
        }
        let sig = self
            .lo
            .program
            .add_signature(Signature::new(physical, result));
        self.call(InstKind::CallClosure { closure, sig, args }, result, span)
    }

    fn lower_conversion(
        &mut self,
        resolved: &ResolvedCall,
        target: TypeId,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let target = self.apply(target);
        let Some(source) = resolved
            .arguments
            .first()
            .and_then(|argument| self.bound_expression(argument, operands))
        else {
            self.lo.unsupported(span, "a conversion with no operand");
            return None;
        };
        let from = self.ty_of(source.id);
        let value = self.lower_operand(source);
        Some(self.convert(value, from, target, span))
    }

    fn convert(&mut self, value: Value, from: TypeId, to: TypeId, span: Span) -> Value {
        use TypeKind::{Bool, Char, Float, Int, String, Uint, UntypedFloat, UntypedInt};

        let from_kind = self.kind_of(from);
        let to_kind = self.kind_of(to);
        let op = match (&from_kind, &to_kind) {
            (Float | UntypedFloat, Int | UntypedInt) => Some(ConvertOp::FloatToInt),
            (Float | UntypedFloat, Uint) => Some(ConvertOp::FloatToUint),
            (Int | UntypedInt, Float | UntypedFloat) => Some(ConvertOp::IntToFloat),
            (Uint, Float | UntypedFloat) => Some(ConvertOp::UintToFloat),
            (Char, Int | UntypedInt | Uint) => Some(ConvertOp::CharToInt),
            (Bool, Int | UntypedInt | Uint) => Some(ConvertOp::BoolToInt),
            _ => None,
        };
        if let Some(op) = op {
            return self.value(InstKind::Convert { op, value }, span);
        }

        match (&from_kind, &to_kind) {
            (Char, Char) => value,
            // `char(v)` co kiem: gia tri ngoai khoang scalar thi panic.
            (_, Char) => self.runtime_value(RuntimeFn::CharFromUint, vec![value], span),
            (_, String) => self.stringify(value, from, span),
            // `int(u)`, `uint(i)` va may phep doi ve chinh no dung chung
            // mot cach bieu dien, nen khong phai sinh gi ca.
            _ => value,
        }
    }

    fn lower_predeclared(
        &mut self,
        value: Predeclared,
        resolved: &ResolvedCall,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        match value {
            Predeclared::Len => return self.lower_len(resolved, operands, span),
            Predeclared::Print | Predeclared::Println => {
                return self.lower_print(value, resolved, operands, span)
            }
            Predeclared::ReadFileText => return self.lower_os(RuntimeFn::ReadFileText, resolved, operands, span),
            Predeclared::ReadFileBytes => return self.lower_os(RuntimeFn::ReadFileBytes, resolved, operands, span),
            Predeclared::WriteFileText => return self.lower_os(RuntimeFn::WriteFileText, resolved, operands, span),
            Predeclared::WriteFileBytes => {
                return self.lower_os(RuntimeFn::WriteFileBytes, resolved, operands, span)
            }
            Predeclared::OsArgs => return self.lower_os(RuntimeFn::OsArgs, resolved, operands, span),
            Predeclared::OsRun => return self.lower_os(RuntimeFn::OsRun, resolved, operands, span),
            Predeclared::OsError => return self.lower_os(RuntimeFn::OsError, resolved, operands, span),
            _ => {}
        }

        let parameters: Vec<TypeId> = match value {
            Predeclared::Assert => vec![TypeId::BOOL, TypeId::STRING],
            _ => vec![TypeId::STRING],
        };
        let values = self.lower_arguments(&resolved.arguments, &parameters, operands, span);
        let first = values.first().copied()?;

        match value {
            Predeclared::Panic => {
                self.runtime(RuntimeFn::Panic, vec![first], span);
                None
            }
            Predeclared::Assert => {
                let message = values.get(1).copied();
                let ok = self.new_block(span);
                let failed = self.new_block(span);
                self.branch(first, ok, failed);

                self.switch_to(failed);
                let message = match message {
                    Some(message) => message,
                    None => self.const_string("assertion failed", span),
                };
                self.runtime(RuntimeFn::Panic, vec![message], span);

                self.switch_to(ok);
                None
            }
            _ => None,
        }
    }

    // May cua vao he dieu hanh deu cung mot hinh: doi so nao vao thang do,
    // khong ep kieu gi, va gia tri tra ve da dung cach bieu dien ma checker
    // hua roi - con tro cho `T?` va cho mang, i8 cho bool.
    //
    // Khong cai nao trong so nay tu cam loi vao pump_error_slot ca. Runtime
    // khong dung duoc mot gia tri `Error` vi itable la do compiler sinh; nen
    // no de lai loi o mot cho rieng va `os_error()` lay ra. Cho `fail` that
    // su nam ben Pump, trong `std/io.pump` va `std/os.pump`.
    fn lower_os(
        &mut self,
        entry: RuntimeFn,
        resolved: &ResolvedCall,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let mut args = Vec::with_capacity(resolved.arguments.len());
        for argument in &resolved.arguments {
            let Some(expr) = self.bound_expression(argument, operands) else {
                self.lo
                    .unsupported(span, "a call argument the checker did not record");
                return None;
            };
            let value = self.lower_operand(expr);
            args.push(value);
        }
        self.runtime(entry, args, span)
    }

    fn lower_print(
        &mut self,
        value: Predeclared,
        resolved: &ResolvedCall,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let node = match resolved.arguments.first()? {
            BoundArgument::Receiver(node) | BoundArgument::Expression(node) => *node,
            BoundArgument::Default(_) | BoundArgument::Variadic(_) => return None,
        };
        let Some(expr) = operands.get(&node).copied() else {
            self.lo
                .unsupported(span, "a call argument the checker did not record");
            return None;
        };
        let ty = self.ty_of(expr.id);
        let raw = self.lower_operand(expr);
        let text = self.stringify(raw, ty, span);
        let function = match value {
            Predeclared::Print => RuntimeFn::Print,
            _ => RuntimeFn::Println,
        };
        self.runtime(function, vec![text], span);
        None
    }

    fn lower_len(
        &mut self,
        resolved: &ResolvedCall,
        operands: &Operands<'c>,
        span: Span,
    ) -> Option<Value> {
        let Some(subject) = resolved
            .arguments
            .first()
            .and_then(|argument| self.bound_expression(argument, operands))
        else {
            self.lo.unsupported(span, "`len` with no operand");
            return None;
        };
        let ty = self.ty_of(subject.id);
        let container = self.lower_operand(subject);
        let entry = match self.kind_of(ty) {
            TypeKind::String => RuntimeFn::StringLen,
            TypeKind::Array(_) => RuntimeFn::ArrayLen,
            TypeKind::Map { .. } => RuntimeFn::MapLen,
            TypeKind::Set(_) => RuntimeFn::SetLen,
            _ => {
                let shown = self.lo.types.display(ty);
                self.lo.unsupported(span, format!("`len` of `{shown}`"));
                return None;
            }
        };
        Some(self.runtime_value(entry, vec![container], span))
    }

    fn lower_variant_call(
        &mut self,
        resolved: &ResolvedCall,
        def: DefId,
        variant: u32,
        operands: &Operands<'c>,
        result_type: TypeId,
        span: Span,
    ) -> Option<Value> {
        let arguments = self
            .lo
            .named_parts(result_type)
            .map(|(_, args)| args)
            .unwrap_or_default();
        let payload = self.lo.variant_payload(def, &arguments, variant as usize);
        let values = self.lower_arguments(&resolved.arguments, &payload, operands, span);
        Some(self.build_variant(result_type, variant, &payload, &values, span))
    }

    fn lower_arguments(
        &mut self,
        bound: &[BoundArgument],
        parameters: &[TypeId],
        operands: &Operands<'c>,
        span: Span,
    ) -> Vec<Value> {
        let mut values = Vec::with_capacity(bound.len());
        for (index, argument) in bound.iter().enumerate() {
            let target = parameters.get(index).copied().unwrap_or(TypeId::ERROR);
            let value = match argument {
                // D-23: `area()` o trong than mot method la `this.area()`.
                // check.rs khong che ra node `this` gia nao ca, no ghi
                // NodeId::NONE roi hen t dat receiver vao o day.
                BoundArgument::Receiver(node) if *node == NodeId::NONE => match self.this {
                    Some(this) => this,
                    None => {
                        self.lo.unsupported(span, "`this` outside a method");
                        self.zero_of(target, span)
                    }
                },
                BoundArgument::Receiver(node) | BoundArgument::Expression(node) => {
                    match operands.get(node).copied() {
                        Some(expr) => self.lower_coerced(expr, target),
                        None => {
                            self.lo
                                .unsupported(span, "a call argument the checker did not record");
                            self.zero_of(target, span)
                        }
                    }
                }
                BoundArgument::Default(constant) => {
                    self.materialise_constant(constant, target, span)
                }
                BoundArgument::Variadic(nodes) => {
                    let element = self.element_of(target);
                    let repr = self.ir_type(element);
                    let mut collected = Vec::with_capacity(nodes.len());
                    for node in nodes {
                        let value = match operands.get(node).copied() {
                            Some(expr) => self.lower_coerced(expr, element),
                            None => self.zero(repr, span),
                        };
                        collected.push(self.widen(value, repr, span));
                    }
                    self.build_array(target, collected, span)
                }
            };
            values.push(value);
        }
        values
    }

    fn bound_expression(
        &self,
        argument: &BoundArgument,
        operands: &Operands<'c>,
    ) -> Option<&'c Expr> {
        match argument {
            BoundArgument::Receiver(node) | BoundArgument::Expression(node) => {
                operands.get(node).copied()
            }
            _ => None,
        }
    }

    fn materialise_constant(&mut self, constant: &ConstValue, target: TypeId, span: Span) -> Value {
        let target = self.apply(target);
        if let ConstValue::Null = constant {
            return self.const_null(span);
        }
        // gia tri mac dinh cua tham so optional duoc viet tran, nen hang so
        // dung o kieu ben trong roi ep sang sau.
        let inner = match self.kind_of(target) {
            TypeKind::Optional(inner) => inner,
            _ => target,
        };

        let value = match constant {
            ConstValue::Bool(literal) => self.const_bool(*literal, span),
            ConstValue::Int(literal) => self.const_int(*literal, span),
            ConstValue::Uint(literal) => self.const_int(*literal as i64, span),
            ConstValue::Float(literal) => self.value(InstKind::ConstFloat(*literal), span),
            ConstValue::Char(literal) => self.value(InstKind::ConstChar(*literal as u32), span),
            ConstValue::Str(text) => self.const_string(text, span),
            ConstValue::Null => self.const_null(span),
            ConstValue::Array(items) => {
                let values = self.constant_elements(items, inner, span);
                self.build_array(inner, values, span)
            }
            ConstValue::Set(items) => {
                let values = self.constant_elements(items, inner, span);
                let descriptor = self.type_id_of(inner, span);
                let set = self.runtime_value(RuntimeFn::SetNew, vec![descriptor], span);
                for value in values {
                    self.runtime(RuntimeFn::SetAdd, vec![set, value], span);
                }
                set
            }
            ConstValue::Map(pairs) => {
                let (key_type, value_type) = match self.kind_of(inner) {
                    TypeKind::Map { key, value } => (key, value),
                    _ => (TypeId::ERROR, TypeId::ERROR),
                };
                let mut entries = Vec::with_capacity(pairs.len());
                for (key, value) in pairs {
                    let key = self.materialise_constant(key, key_type, span);
                    let key = self.widen_as(key, key_type, span);
                    let value = self.materialise_constant(value, value_type, span);
                    let value = self.widen_as(value, value_type, span);
                    entries.push((key, value));
                }
                let descriptor = self.type_id_of(inner, span);
                let map = self.runtime_value(RuntimeFn::MapNew, vec![descriptor], span);
                for (key, value) in entries {
                    self.runtime(RuntimeFn::MapSet, vec![map, key, value], span);
                }
                map
            }
            ConstValue::Tuple(items) => {
                let TypeKind::Tuple(element_types) = self.kind_of(inner) else {
                    self.lo.unsupported(span, "a tuple default of unknown type");
                    return self.zero_of(target, span);
                };
                let mut values = Vec::with_capacity(items.len());
                for (item, element) in items.iter().zip(&element_types) {
                    values.push(self.materialise_constant(item, *element, span));
                }
                self.build_record(inner, &element_types, &values, span)
            }
            ConstValue::EnumVariant { def, variant } => {
                let ty = match self.lo.named_parts(inner) {
                    Some((named, _)) if named == *def => inner,
                    _ => self.lo.types.named(*def, Vec::new()),
                };
                return self.build_variant(ty, *variant, &[], &[], span);
            }
        };

        let natural = match constant {
            ConstValue::Bool(_) => TypeId::BOOL,
            ConstValue::Int(_) => TypeId::INT,
            ConstValue::Uint(_) => TypeId::UINT,
            ConstValue::Float(_) => TypeId::FLOAT,
            ConstValue::Char(_) => TypeId::CHAR,
            ConstValue::Str(_) => TypeId::STRING,
            _ => inner,
        };
        self.coerce(value, natural, target, span)
    }

    fn constant_elements(
        &mut self,
        items: &[ConstValue],
        container: TypeId,
        span: Span,
    ) -> Vec<Value> {
        let element = self.element_of(container);
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            let value = self.materialise_constant(item, element, span);
            values.push(self.widen_as(value, element, span));
        }
        values
    }

    // ---- method co san ----

    fn lower_builtin(
        &mut self,
        method: BuiltinMethod,
        resolved: &ResolvedCall,
        operands: &Operands<'c>,
        result_type: TypeId,
        span: Span,
    ) -> Option<Value> {
        use BuiltinMethod as Builtin;

        let Some(receiver) = resolved
            .arguments
            .first()
            .and_then(|argument| self.bound_expression(argument, operands))
        else {
            self.lo
                .unsupported(span, "a builtin method with no receiver");
            return None;
        };
        let receiver_type = self.ty_of(receiver.id);
        let element = self.element_of(receiver_type);
        let (key_type, value_type) = match self.kind_of(receiver_type) {
            TypeKind::Map { key, value } => (key, value),
            _ => (TypeId::ERROR, TypeId::ERROR),
        };
        let inner = match self.kind_of(receiver_type) {
            TypeKind::Optional(inner) => inner,
            _ => receiver_type,
        };

        let mut parameters = vec![receiver_type];
        parameters.extend(match method {
            Builtin::StringByteAt | Builtin::ArrayReserve => vec![TypeId::INT],
            Builtin::StringSlice | Builtin::ArraySlice => vec![TypeId::INT, TypeId::INT],
            Builtin::ArrayPush => vec![element],
            Builtin::ArrayConcat => vec![receiver_type],
            Builtin::MapHas | Builtin::MapGet | Builtin::MapRemove => vec![key_type],
            Builtin::MapInsert => vec![key_type, value_type],
            Builtin::SetAdd | Builtin::SetHas | Builtin::SetRemove => vec![element],
            Builtin::OptionalExpect => vec![TypeId::STRING],
            Builtin::OptionalOr => vec![inner],
            _ => Vec::new(),
        });

        let values = self.lower_arguments(&resolved.arguments, &parameters, operands, span);
        let object = *values.first()?;
        let rest = &values[1..];

        let (result, natural) = match method {
            Builtin::ToString => (self.stringify(object, receiver_type, span), TypeId::STRING),
            Builtin::StringMessage => (object, TypeId::STRING),
            Builtin::StringChars => (
                self.runtime_value(RuntimeFn::StringChars, vec![object], span),
                result_type,
            ),
            Builtin::StringCharCount => (
                self.runtime_value(RuntimeFn::StringCharCount, vec![object], span),
                TypeId::UINT,
            ),
            Builtin::StringByteAt => {
                let index = *rest.first()?;
                (
                    self.runtime_value(RuntimeFn::StringByteAt, vec![object, index], span),
                    TypeId::INT,
                )
            }
            Builtin::StringSlice => {
                let start = *rest.first()?;
                let end = *rest.get(1)?;
                (
                    self.runtime_value(RuntimeFn::StringSlice, vec![object, start, end], span),
                    TypeId::STRING,
                )
            }
            Builtin::ArrayPush => {
                let value = self.widen_as(*rest.first()?, element, span);
                self.runtime(RuntimeFn::ArrayPush, vec![object, value], span);
                return None;
            }
            Builtin::ArrayPop => {
                let repr = self.ir_type(element);
                let raw = self.runtime_value(RuntimeFn::ArrayPop, vec![object], span);
                (self.narrow(raw, repr, span), element)
            }
            Builtin::ArraySlice => {
                let start = *rest.first()?;
                let end = *rest.get(1)?;
                (
                    self.runtime_value(RuntimeFn::ArraySlice, vec![object, start, end], span),
                    receiver_type,
                )
            }
            Builtin::ArrayConcat => {
                let other = *rest.first()?;
                (
                    self.runtime_value(RuntimeFn::ArrayConcat, vec![object, other], span),
                    receiver_type,
                )
            }
            Builtin::ArrayReserve => {
                let capacity = *rest.first()?;
                self.runtime(RuntimeFn::ArrayReserve, vec![object, capacity], span);
                return None;
            }
            Builtin::MapHas => {
                let key = self.widen_as(*rest.first()?, key_type, span);
                ( self.runtime_value(RuntimeFn::MapHas, vec![object, key], span), TypeId::BOOL, )
            }
            Builtin::MapGet => {
                let key = self.widen_as(*rest.first()?, key_type, span);
                return Some(self.lower_map_get(object, key, value_type, result_type, span));
            }
            Builtin::MapInsert => {
                let key = self.widen_as(*rest.first()?, key_type, span);
                let value = self.widen_as(*rest.get(1)?, value_type, span);
                self.runtime(RuntimeFn::MapSet, vec![object, key, value], span);
                return None;
            }
            Builtin::MapRemove => {
                let key = self.widen_as(*rest.first()?, key_type, span);
                (
                    self.runtime_value(RuntimeFn::MapRemove, vec![object, key], span),
                    TypeId::BOOL,
                )
            }
            Builtin::MapKeys => (
                self.runtime_value(RuntimeFn::MapKeys, vec![object], span),
                result_type,
            ),
            Builtin::MapValues => (
                self.runtime_value(RuntimeFn::MapValues, vec![object], span),
                result_type,
            ),
            Builtin::SetAdd => {
                let value = self.widen_as(*rest.first()?, element, span);
                (
                    self.runtime_value(RuntimeFn::SetAdd, vec![object, value], span),
                    TypeId::BOOL,
                )
            }
            Builtin::SetHas => {
                let value = self.widen_as(*rest.first()?, element, span);
                (
                    self.runtime_value(RuntimeFn::SetHas, vec![object, value], span),
                    TypeId::BOOL,
                )
            }
            Builtin::SetRemove => {
                let value = self.widen_as(*rest.first()?, element, span);
                (
                    self.runtime_value(RuntimeFn::SetRemove, vec![object, value], span),
                    TypeId::BOOL,
                )
            }
            Builtin::OptionalExpect => {
                let message = *rest.first()?;
                let present = self.expect_present(object, message, receiver_type, inner, span);
                (present, inner)
            }
            Builtin::OptionalOr => {
                let fallback = *rest.first()?;
                let value = self.optional_or(object, fallback, receiver_type, inner, span);
                (value, inner)
            }
        };
        Some(self.coerce(result, natural, result_type, span))
    }

    fn widen_as(&mut self, value: Value, ty: TypeId, span: Span) -> Value {
        let repr = self.ir_type(ty);
        self.widen(value, repr, span)
    }

    fn lower_map_get(
        &mut self,
        map: Value,
        key: Value,
        value_type: TypeId,
        result_type: TypeId,
        span: Span,
    ) -> Value {
        let repr = self.ir_type(value_type);
        if !matches!(self.kind_of(result_type), TypeKind::Optional(_)) {
            let raw = self.runtime_value(RuntimeFn::MapGet, vec![map, key], span);
            return self.narrow(raw, repr, span);
        }

        let slot = self.function.add_slot_for(IrType::I64, span);
        let out = self.value(InstKind::SlotAddr(slot), span);
        let found = self.runtime_value(RuntimeFn::MapLookup, vec![map, key, out], span);

        let present = self.new_block(span);
        let join = self.new_block(span);
        let result = self.function.add_block_param(join, IrType::Ptr);
        let missing = self.const_null(span);
        self.seal(Terminator::Branch {
            cond: found,
            then_block: present,
            then_args: Vec::new(),
            else_block: join,
            else_args: vec![missing],
        });

        self.switch_to(present);
        let address = self.value(InstKind::SlotAddr(slot), span);
        let raw = self.load(address, 0, IrType::I64, span);
        let value = self.narrow(raw, repr, span);
        let optional = if repr.is_pointer() {
            value
        } else {
            self.box_value(value, repr, span)
        };
        self.jump(join, vec![optional]);

        self.switch_to(join);
        result
    }

    fn expect_present(
        &mut self,
        optional: Value,
        message: Value,
        optional_type: TypeId,
        inner: TypeId,
        span: Span,
    ) -> Value {
        let null = self.const_null(span);
        let is_null = self.compare(CompareOp::IEq, optional, null, span);
        let absent = self.new_block(span);
        let present = self.new_block(span);
        self.branch(is_null, absent, present);

        self.switch_to(absent);
        self.runtime(RuntimeFn::Panic, vec![message], span);

        self.switch_to(present);
        self.coerce(optional, optional_type, inner, span)
    }

    fn optional_or(
        &mut self,
        optional: Value,
        fallback: Value,
        optional_type: TypeId,
        inner: TypeId,
        span: Span,
    ) -> Value {
        let repr = self.ir_type(inner);
        let null = self.const_null(span);
        let is_null = self.compare(CompareOp::IEq, optional, null, span);

        let present = self.new_block(span);
        let join = self.new_block(span);
        let result = self.function.add_block_param(join, repr);
        self.seal(Terminator::Branch {
            cond: is_null,
            then_block: join,
            then_args: vec![fallback],
            else_block: present,
            else_args: Vec::new(),
        });

        self.switch_to(present);
        let unwrapped = self.coerce(optional, optional_type, inner, span);
        self.jump(join, vec![unwrapped]);

        self.switch_to(join);
        result
    }
}

fn strip_groups(expr: &Expr) -> &Expr {
    let mut current = expr;
    while let ExprKind::Group(inner) = &current.kind {
        current = inner;
    }
    current
}

fn is_null_literal(expr: &Expr) -> bool {
    matches!(strip_groups(expr).kind, ExprKind::Null)
}

type PatternField<'c> = (u32, IrType, TypeId, &'c Pattern);

type Operands<'c> = HashMap<NodeId, &'c Expr>;

fn call_operands<'c>(callee: &'c Expr, args: &'c [Argument]) -> Operands<'c> {
    let mut operands = HashMap::with_capacity(args.len() + 1);
    if let ExprKind::Field { base, .. }
    | ExprKind::TupleField { base, .. }
    | ExprKind::Index { base, .. } = &strip_call_path(callee).kind
    {
        operands.insert(base.id, base.as_ref());
    }
    for argument in args {
        operands.insert(argument.value.id, &argument.value);
    }
    operands
}

fn strip_call_path(expr: &Expr) -> &Expr {
    let mut current = expr;
    loop {
        current = match &current.kind {
            ExprKind::Group(inner) => inner,
            ExprKind::TypeArgs { base, .. } => base,
            _ => return current,
        };
    }
}

fn signed_literal(magnitude: u64, negative: bool) -> i64 {
    let raw = magnitude as i64;
    if negative {
        raw.wrapping_neg()
    } else {
        raw
    }
}

fn integer_compare(op: SourceBinaryOp, unsigned: bool) -> CompareOp {
    match op {
        SourceBinaryOp::Eq => CompareOp::IEq,
        SourceBinaryOp::Ne => CompareOp::INe,
        SourceBinaryOp::Lt if unsigned => CompareOp::ULt,
        SourceBinaryOp::Lt => CompareOp::SLt,
        SourceBinaryOp::Gt if unsigned => CompareOp::UGt,
        SourceBinaryOp::Gt => CompareOp::SGt,
        SourceBinaryOp::Le if unsigned => CompareOp::ULe,
        SourceBinaryOp::Le => CompareOp::SLe,
        SourceBinaryOp::Ge if unsigned => CompareOp::UGe,
        SourceBinaryOp::Ge => CompareOp::SGe,
        // checker chi bao gio dua toan tu so sanh vao cai bang nay.
        _ => CompareOp::IEq,
    }
}
