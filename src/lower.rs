// lower. AST da ...
//
// day la cho ngon ngu dung lai va may bat dau, nen moi thu frontend da biet
// ma backend khong phai nghi lai deu chot o day:
//
//  * mono - moi ban duoc dung toi la mot ham IR, worklist moi tu main, tu
//    danh sach ban da ghi lai, va tu tung itable,
//  * layout ...
// cai nay  * type
//    ref_offsets la GC no do, nen phai dung crate::abi ma dung, tuyet doi
//    khong go tay,
//  * itable - moi conformance mot cai, method xep theo dung thu tu cua
//    interface,
//  * duong hoa - vong for,
//  nhanh re theo o loi, `?`
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
            // loi goi nam trong than mot
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
                // source doc thi set la mot cai
                // (docs/abi.md 8).
                let flags = self.reference_flag( element, abi::DESC_FLAG_ELEM_IS_REF | abi::DESC_FLAG_KEY_IS_REF, );
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
            // cai nay doc kich thuoc
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

    // cai nay ---- day cai

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
