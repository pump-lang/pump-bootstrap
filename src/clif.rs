// sinh ma bang Cranelift. mot ir::Program thanh ma may.
//
// CANH BAO CHO CHINH T SAU NAY: file nay ban nhat repo. T viet no trong hai
// tuan he, vua xem video vua mo doc, va co nhung doan t chi biet la no chay
// chu khong biet han tai sao. Doan nao t chua chac thi t de lai ban cu ngay
// tren duoi dang comment, dung xoa voi.
//
// Cranelift ghim dung 0.126.0. Dung nang len neu chua doc release notes, no
// doi nhanh lam.
//
// Ca hai backend deu di vao qua define_program, ham nay generic tren Module:
// jit.rs dua vao mot JITModule, con emit_object dua vao mot ObjectModule.
// Viet ban dich mot lan thoi thi hai ben khong the tu lech nhau sau lung t.
//
// Ba cho ma Cranelift KHONG hieu giong Pump, moi cho phai no ra may lenh chu
// khong duoc mot lenh:
//
//  * dich bit. Cranelift che so dem theo do rong toan hang, nen `x << 64` ra
//    lai dung `x`. Pump thi bao dem bang hoac vuot do rong la ra 0, hoac ra
//    bit dau neu la dich phai so hoc.
//  * chia va du co dau. Cranelift trap khi int.min / -1, con Pump thi cuon ve
//    int.min voi so du 0. Nen chia cho -1 thi t doi thanh chia cho 1 roi doi
//    dau thuong.
//  * du cua so thuc. Khong co lenh frem, cung khong co libcall fmod nao het,
//    nen `float % float` bien thanh mot loi goi den fmod cua nen tang. Trong
//    .exe da link thi cai do den tu CRT cua MSVC, con jit.rs thi dang ky mot
//    cai shim cung ten.

use std::collections::HashMap;

use cranelift_codegen::ir as cl;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, TrapCode};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};

use crate::abi::{self, IrType, RuntimeFn, TypeDescriptor, VariantDescriptor};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::{
    BinaryOp, BlockRef, CompareOp, ConvertOp, FuncRef, Function, Inst, InstKind, InstRef, Program,
    SigRef, Terminator, UnaryOp, Value,
};
use crate::token::Span;

// ===== bang intrinsic =====
//
// t go tay bang nay. Thu tu KHONG phai thu tu trong abi.rs va cung khong phai
// abc: no la thu tu t hay phai mo ra xem nhat luc di do bug, tu hay nhat den
// it hay nhat. May cai nay duoc khai bao truoc, con lai lay not tu
// RuntimeFn::ALL nen khong sot cai nao.
//
// Dung sap xep lai cho "gon". Doc tu tren xuong la thay ngay t hay hong cho
// nao.
const HAY_DUNG: &[RuntimeFn] = &[
    RuntimeFn::Alloc,
    RuntimeFn::Panic,
    RuntimeFn::StringNew,
    RuntimeFn::ArrayGet,
    RuntimeFn::ErrorPending,
    RuntimeFn::StringConcat,
    RuntimeFn::ArrayPush,
    RuntimeFn::PanicIndex,
    RuntimeFn::MapLookup,
    RuntimeFn::IfaceNew,
    RuntimeFn::GcCollect,
    RuntimeFn::BoxNew,
    RuntimeFn::Println,
    RuntimeFn::ErrorTake,
    RuntimeFn::ClosureNew,
    RuntimeFn::StringEq,
    RuntimeFn::AllocBuffer,
    RuntimeFn::PanicNull,
    RuntimeFn::MapSet,
    RuntimeFn::CollectionModcount,
    RuntimeFn::ArrayLen,
    RuntimeFn::ErrorSet,
    RuntimeFn::StringFromInt,
    RuntimeFn::SetAdd,
    RuntimeFn::Exit,
];

/// The C `fmod`, the one symbol outside `RuntimeFn` that generated code may
/// reference.
pub const SYMBOL_FMOD: &str = "fmod";

const TRAP_UNREACHABLE: TrapCode = TrapCode::unwrap_user(1);

const UNHANDLED_ERROR_MESSAGE: &[u8] = b"unhandled error returned from main";

fn mem_flags() -> MemFlags {
    MemFlags::trusted()
}

/// Settings shared by the JIT and object paths.
#[derive(Clone, Debug)]
pub struct CodegenOptions {
    pub triple: String,
    pub opt_level: &'static str,
    pub dump_clif: bool,
    pub emit_c_main: bool,
}

impl Default for CodegenOptions {
    fn default() -> CodegenOptions {
        CodegenOptions {
            triple: target_lexicon::Triple::host().to_string(),
            opt_level: "speed",
            dump_clif: false,
            emit_c_main: false,
        }
    }
}

/// The Cranelift shared flags both back ends use.
pub fn shared_flags(options: &CodegenOptions) -> Result<settings::Flags, CompileError> {
    let mut builder = settings::builder();
    for (name, value) in [
        ("opt_level", options.opt_level),
        ("is_pic", "false"),
        ("use_colocated_libcalls", "false"),
    ] {
        builder.set(name, value).map_err(|error| {
            failure(format!(
                "cannot set the Cranelift flag `{name}` to `{value}`: {error}"
            ))
        })?;
    }
    Ok(settings::Flags::new(builder))
}

/// Builds the target ISA for `options.triple`.
pub fn target_isa(options: &CodegenOptions) -> Result<OwnedTargetIsa, CompileError> {
    let unsupported = |message: String| {
        CompileError::at(ErrorCode::UnsupportedTarget, Span::synthetic(), message)
    };
    let triple: target_lexicon::Triple = options.triple.parse().map_err(|error| {
        unsupported(format!(
            "`{}` is not a target triple: {error}",
            options.triple
        ))
    })?;
    let builder = cranelift_codegen::isa::lookup(triple).map_err(|error| {
        unsupported(format!(
            "Cranelift cannot target `{}`: {error}",
            options.triple
        ))
    })?;
    builder
        .finish(shared_flags(options)?)
        .map_err(|error| failure(format!("cannot configure the target: {error}")))
}

/// Translates a whole program into `module`: every function, every static,
/// and the compiler-emitted entry points of `docs/abi.md` section 7.
pub fn define_program<M: Module>(
    module: &mut M,
    program: &Program,
    options: &CodegenOptions,
) -> Result<(), CompileError> {
    let mut codegen = Codegen::new(module, program, options)?;
    codegen.declare_imports()?;
    codegen.declare_functions()?;
    codegen.declare_statics()?;
    codegen.define_statics()?;
    codegen.define_functions()?;
    codegen.define_program_main()?;
    if options.emit_c_main {
        codegen.define_c_main()?;
    }
    Ok(())
}

/// Compiles a program to a native object file, for `pump build`.
pub fn emit_object(program: &Program, options: &CodegenOptions) -> Result<Vec<u8>, CompileError> {
    let isa = target_isa(options)?;
    let builder = cranelift_object::ObjectBuilder::new(
        isa,
        "pump".to_string(),
        cranelift_module::default_libcall_names(),
    )
    .map_err(|error| failure(format!("cannot start the object file: {error}")))?;
    let mut module = cranelift_object::ObjectModule::new(builder);

    let mut options = options.clone();
    options.emit_c_main = true;
    define_program(&mut module, program, &options)?;

    module.finish().emit().map_err(|error| {
        CompileError::at(
            ErrorCode::ObjectEmissionFailed,
            Span::synthetic(),
            format!("cannot encode the object file: {error}"),
        )
    })
}

fn failure(message: impl Into<String>) -> CompileError {
    CompileError::at(ErrorCode::CodegenFailed, Span::synthetic(), message)
}

fn failure_at(span: Span, message: impl Into<String>) -> CompileError {
    CompileError::at(ErrorCode::CodegenFailed, span, message)
}

// === anh byte tinh ==========================

#[derive(Default)]
struct Image {
    bytes: Vec<u8>,
}

impl Image {
    fn with_capacity(capacity: usize) -> Image {
        Image {
            bytes: Vec::with_capacity(capacity),
        }
    }

    fn len(&self) -> u32 {
        self.bytes.len() as u32
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn raw(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    fn pad_to(&mut self, size: u64) {
        self.bytes.resize(size as usize, 0);
    }

    fn finish(self) -> Box<[u8]> {
        self.bytes.into_boxed_slice()
    }
}

#[derive(Clone, Copy, Debug)]
struct GlobalStorage {
    data: DataId,
    offset: i64,
}

// --- ban than cai bo dich ------------------------------

struct Codegen<'a, M: Module> {
    module: &'a mut M,
    program: &'a Program,
    options: &'a CodegenOptions,
    pointer_type: cl::Type,
    runtime: HashMap<RuntimeFn, FuncId>,
    fmod: FuncId,
    functions: Vec<FuncId>,
    strings: Vec<DataId>,
    itables: Vec<DataId>,
    singletons: HashMap<(u32, u32), DataId>,
    globals: Vec<GlobalStorage>,
    scalar_globals: Vec<DataId>,
    global_roots: DataId,
    root_count: u64,
    type_table: DataId,
    program_main: Option<FuncId>,
}

impl<'a, M: Module> Codegen<'a, M> {
    fn new(
        module: &'a mut M,
        program: &'a Program,
        options: &'a CodegenOptions,
    ) -> Result<Codegen<'a, M>, CompileError> {
        let pointer_type = module.target_config().pointer_type();
        let global_roots = module
            .declare_data(abi::SYMBOL_GLOBAL_ROOTS, Linkage::Export, true, false)
            .map_err(|error| failure(format!("cannot declare the global root table: {error}")))?;
        let type_table = module
            .declare_data(abi::SYMBOL_TYPE_TABLE, Linkage::Export, false, false)
            .map_err(|error| failure(format!("cannot declare the type table: {error}")))?;
        let fmod_signature = {
            let mut signature = cl::Signature::new(module.isa().default_call_conv());
            signature.params.push(AbiParam::new(types::F64));
            signature.params.push(AbiParam::new(types::F64));
            signature.returns.push(AbiParam::new(types::F64));
            signature
        };
        let fmod = module
            .declare_function(SYMBOL_FMOD, Linkage::Import, &fmod_signature)
            .map_err(|error| failure(format!("cannot declare `{SYMBOL_FMOD}`: {error}")))?;

        Ok(Codegen {
            module,
            program,
            options,
            pointer_type,
            runtime: HashMap::new(),
            fmod,
            functions: Vec::new(),
            strings: Vec::new(),
            itables: Vec::new(),
            singletons: HashMap::new(),
            globals: Vec::new(),
            scalar_globals: Vec::new(),
            global_roots,
            root_count: program.root_globals().count() as u64,
            type_table,
            program_main: None,
        })
    }

    fn clif_type(&self, ty: IrType) -> cl::Type {
        match ty {
            IrType::I8 => types::I8,
            IrType::I32 => types::I32,
            IrType::I64 => types::I64,
            IrType::F64 => types::F64,
            IrType::Ptr => self.pointer_type,
        }
    }

    fn signature_of(&self, params: &[IrType], ret: Option<IrType>) -> cl::Signature {
        let mut signature = cl::Signature::new(self.module.isa().default_call_conv());
        for &param in params {
            signature.params.push(AbiParam::new(self.clif_type(param)));
        }
        if let Some(ret) = ret {
            signature.returns.push(AbiParam::new(self.clif_type(ret)));
        }
        signature
    }

    // ---- khai bao ----

    fn declare_imports(&mut self) -> Result<(), CompileError> {
        // HAY_DUNG truoc, roi den con lai. Khai bao hai lan mot cai la
        // Cranelift no keu, nen check map truoc.
        let order = HAY_DUNG.iter().copied().chain(RuntimeFn::ALL.iter().copied());
        for entry in order {
            if self.runtime.contains_key(&entry) {
                continue;
            }
            let runtime_signature = entry.signature();
            let signature = self.signature_of(runtime_signature.params, runtime_signature.ret);
            let id = self
                .module
                .declare_function(entry.symbol(), Linkage::Import, &signature)
                .map_err(|error| {
                    failure(format!("cannot declare `{}`: {error}", entry.symbol()))
                })?;
            self.runtime.insert(entry, id);
        }
        Ok(())
    }

    fn declare_functions(&mut self) -> Result<(), CompileError> {
        for function in &self.program.functions {
            let signature = self.signature_of(&function.signature.params, function.signature.ret);
            let linkage = if function.exported {
                Linkage::Export
            } else {
                Linkage::Local
            };
            let id = self
                .module
                .declare_function(&function.name, linkage, &signature)
                .map_err(|error| {
                    failure_at(
                        function.span,
                        format!("cannot declare `{}`: {error}", function.name),
                    )
                })?;
            self.functions.push(id);
        }
        Ok(())
    }

    fn declare_statics(&mut self) -> Result<(), CompileError> {
        for _ in &self.program.strings {
            let id = self
                .module
                .declare_anonymous_data(true, false)
                .map_err(|error| failure(format!("cannot declare a string literal: {error}")))?;
            self.strings.push(id);
        }

        for itable in &self.program.itables {
            let id = self
                .module
                .declare_data(&itable.name, Linkage::Local, false, false)
                .map_err(|error| {
                    failure(format!("cannot declare itable `{}`: {error}", itable.name))
                })?;
            self.itables.push(id);
        }

        for singleton in &self.program.enum_singletons {
            let id = self
                .module
                .declare_data(&singleton.name, Linkage::Local, true, false)
                .map_err(|error| {
                    failure(format!(
                        "cannot declare enum singleton `{}`: {error}",
                        singleton.name
                    ))
                })?;
            self.singletons
                .insert((singleton.type_id.0, singleton.tag), id);
        }

        let mut root_slot = 0i64;
        for global in &self.program.globals {
            let storage = if global.ty.is_pointer() {
                let storage = GlobalStorage {
                    data: self.global_roots,
                    offset: root_slot * abi::SLOT_SIZE as i64,
                };
                root_slot += 1;
                storage
            } else {
                let id = self
                    .module
                    .declare_data(&global.name, Linkage::Local, true, false)
                    .map_err(|error| {
                        failure_at(
                            global.span,
                            format!("cannot declare `{}`: {error}", global.name),
                        )
                    })?;
                self.scalar_globals.push(id);
                GlobalStorage {
                    data: id,
                    offset: 0,
                }
            };
            self.globals.push(storage);
        }
        Ok(())
    }

    // ---- du lieu tinh ----

    fn define_statics(&mut self) -> Result<(), CompileError> {
        self.define_global_roots()?;
        self.define_scalar_globals()?;
        self.define_strings()?;
        self.define_singletons()?;
        self.define_itables()?;
        self.define_type_table()
    }

    fn define_data(
        &mut self,
        id: DataId,
        description: &DataDescription,
        what: &str,
    ) -> Result<(), CompileError> {
        self.module
            .define_data(id, description)
            .map_err(|error| failure(format!("cannot define {what}: {error}")))
    }

    fn define_global_roots(&mut self) -> Result<(), CompileError> {
        let mut description = DataDescription::new();
        // data object dai 0 thi ca hai backend deu kho chiu, nen bang nay
        // luon co cho cho it nhat mot o, ke ca khi chuong trinh khong co hang
        // nao kieu tham chieu. root_count van bao do dai that.
        let size = (self.root_count.max(1) * abi::SLOT_SIZE as u64) as usize;
        description.define_zeroinit(size);
        description.set_align(abi::SLOT_SIZE as u64);
        let id = self.global_roots;
        self.define_data(id, &description, "the global root table")
    }

    fn define_scalar_globals(&mut self) -> Result<(), CompileError> {
        for index in 0..self.scalar_globals.len() {
            let mut description = DataDescription::new();
            description.define_zeroinit(abi::SLOT_SIZE as usize);
            description.set_align(abi::SLOT_SIZE as u64);
            let id = self.scalar_globals[index];
            self.define_data(id, &description, "a module constant")?;
        }
        Ok(())
    }

    fn define_strings(&mut self) -> Result<(), CompileError> {
        for (index, text) in self.program.strings.iter().enumerate() {
            let length = text.len() as u64;
            let size = abi::align_object_size(abi::string::unaligned_size(length));

            let mut image = Image::with_capacity(size as usize);
            image.u32(abi::TYPE_ID_STRING);
            image.u32(abi::FLAG_IMMORTAL);
            image.u64(size);
            image.u64(length);
            // hash tinh luoi, so 0 nghia la "chua tinh".
            image.u64(0);
            image.raw(text.as_bytes());
            // cai NUL o cuoi va phan dem cho du kich thuoc da can le.
            image.pad_to(size);

            let mut description = DataDescription::new();
            description.define(image.finish());
            description.set_align(abi::OBJECT_ALIGN as u64);
            let id = self.strings[index];
            self.define_data(id, &description, "a string literal")?;
        }
        Ok(())
    }

    fn define_singletons(&mut self) -> Result<(), CompileError> {
        for singleton in &self.program.enum_singletons {
            let size = abi::align_object_size(abi::enumeration::PAYLOAD_OFFSET as u64);

            let mut image = Image::with_capacity(size as usize);
            image.u32(singleton.type_id.0);
            image.u32(abi::FLAG_IMMORTAL);
            image.u64(size);
            image.u32(singleton.tag);
            image.u32(0);
            image.pad_to(size);

            let mut description = DataDescription::new();
            description.define(image.finish());
            description.set_align(abi::OBJECT_ALIGN as u64);
            let id = self.singletons[&(singleton.type_id.0, singleton.tag)];
            self.define_data(id, &description, "an enum singleton")?;
        }
        Ok(())
    }

    fn define_itables(&mut self) -> Result<(), CompileError> {
        for (index, itable) in self.program.itables.iter().enumerate() {
            let method_count = itable.methods.len() as u32;

            let mut image = Image::with_capacity(abi::itable_size(method_count) as usize);
            image.u64(itable.interface_id);
            image.u64(itable.concrete_type.0 as u64);
            image.u64(method_count as u64);
            for _ in &itable.methods {
                image.u64(0);
            }

            let mut description = DataDescription::new();
            description.define(image.finish());
            description.set_align(abi::SLOT_SIZE as u64);
            for (slot, &method) in itable.methods.iter().enumerate() {
                let target = self.function_id(method)?;
                let handle = self.module.declare_func_in_data(target, &mut description);
                description.write_function_addr(abi::itable_method_offset(slot as u32), handle);
            }

            let id = self.itables[index];
            self.define_data(id, &description, "an itable")?;
        }
        Ok(())
    }

    fn define_type_table(&mut self) -> Result<(), CompileError> {
        let descriptors = &self.program.type_descriptors;
        for (index, descriptor) in descriptors.iter().enumerate() {
            if descriptor.type_id as usize != index {
                return Err(failure(format!(
                    "the type table is indexed by type id, but entry {index} declares id {}",
                    descriptor.type_id
                )));
            }
        }

        // con cua tung dong phai co truoc thi bang moi tro vao duoc, nen
        // dinh nghia het chung truoc roi gom lai day theo dung thu tu.
        let mut children: Vec<(Option<DataId>, Option<DataId>, DataId)> =
            Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            let ref_offsets = self.define_offset_array(&descriptor.ref_offsets)?;
            let variants = self.define_variants(&descriptor.variants)?;
            let name = self.define_cstring(&descriptor.name)?;
            children.push((ref_offsets, variants, name));
        }

        let mut image =
            Image::with_capacity(descriptors.len() * abi::TYPE_DESCRIPTOR_SIZE as usize);
        for descriptor in descriptors {
            write_descriptor(&mut image, descriptor);
        }

        let mut description = DataDescription::new();
        description.define(image.finish());
        description.set_align(abi::SLOT_SIZE as u64);
        for (index, (ref_offsets, variants, name)) in children.into_iter().enumerate() {
            let base = index as u32 * abi::TYPE_DESCRIPTOR_SIZE;
            if let Some(ref_offsets) = ref_offsets {
                let handle = self
                    .module
                    .declare_data_in_data(ref_offsets, &mut description);
                description.write_data_addr(base + abi::descriptor::REF_OFFSETS_OFFSET, handle, 0);
            }
            if let Some(variants) = variants {
                let handle = self.module.declare_data_in_data(variants, &mut description);
                description.write_data_addr(base + abi::descriptor::VARIANTS_OFFSET, handle, 0);
            }
            let handle = self.module.declare_data_in_data(name, &mut description);
            description.write_data_addr(base + abi::descriptor::NAME_OFFSET, handle, 0);
        }

        let id = self.type_table;
        self.define_data(id, &description, "the type descriptor table")
    }

    fn define_offset_array(&mut self, offsets: &[u32]) -> Result<Option<DataId>, CompileError> {
        if offsets.is_empty() {
            return Ok(None);
        }
        let mut image = Image::with_capacity(offsets.len() * 4);
        for &offset in offsets {
            image.u32(offset);
        }
        let mut description = DataDescription::new();
        description.define(image.finish());
        description.set_align(4);
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|error| failure(format!("cannot declare an offset array: {error}")))?;
        self.define_data(id, &description, "an offset array")?;
        Ok(Some(id))
    }

    fn define_variants(
        &mut self,
        variants: &[VariantDescriptor],
    ) -> Result<Option<DataId>, CompileError> {
        if variants.is_empty() {
            return Ok(None);
        }

        let mut children: Vec<(Option<DataId>, DataId)> = Vec::with_capacity(variants.len());
        for variant in variants {
            let ref_offsets = self.define_offset_array(&variant.ref_offsets)?;
            let name = self.define_cstring(&variant.name)?;
            children.push((ref_offsets, name));
        }

        let mut image =
            Image::with_capacity(variants.len() * abi::VARIANT_DESCRIPTOR_SIZE as usize);
        for variant in variants {
            image.u32(variant.ref_offsets.len() as u32);
            image.u32(0);
            image.u64(0);
            image.u64(0);
        }

        let mut description = DataDescription::new();
        description.define(image.finish());
        description.set_align(abi::SLOT_SIZE as u64);
        for (index, (ref_offsets, name)) in children.into_iter().enumerate() {
            let base = index as u32 * abi::VARIANT_DESCRIPTOR_SIZE;
            if let Some(ref_offsets) = ref_offsets {
                let handle = self
                    .module
                    .declare_data_in_data(ref_offsets, &mut description);
                description.write_data_addr(
                    base + abi::variant_descriptor::REF_OFFSETS_OFFSET,
                    handle,
                    0,
                );
            }
            let handle = self.module.declare_data_in_data(name, &mut description);
            description.write_data_addr(base + abi::variant_descriptor::NAME_OFFSET, handle, 0);
        }

        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|error| failure(format!("cannot declare a variant table: {error}")))?;
        self.define_data(id, &description, "a variant table")?;
        Ok(Some(id))
    }

    fn define_cstring(&mut self, text: &str) -> Result<DataId, CompileError> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        self.define_bytes(&bytes)
    }

    fn define_bytes(&mut self, bytes: &[u8]) -> Result<DataId, CompileError> {
        let mut description = DataDescription::new();
        description.define(bytes.to_vec().into_boxed_slice());
        description.set_align(1);
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|error| failure(format!("cannot declare a byte string: {error}")))?;
        self.define_data(id, &description, "a byte string")?;
        Ok(id)
    }

    // ==== than ham ====

    fn function_id(&self, func: FuncRef) -> Result<FuncId, CompileError> {
        self.functions
            .get(func.index())
            .copied()
            .ok_or_else(|| failure(format!("the IR refers to an undeclared function {func:?}")))
    }

    fn define_functions(&mut self) -> Result<(), CompileError> {
        let mut context = Context::new();
        let mut builder_context = FunctionBuilderContext::new();
        for index in 0..self.program.functions.len() {
            context.clear();
            self.define_one_function(index, &mut context, &mut builder_context)?;
        }
        Ok(())
    }

    fn define_one_function(
        &mut self,
        index: usize,
        context: &mut Context,
        builder_context: &mut FunctionBuilderContext,
    ) -> Result<(), CompileError> {
        let function = &self.program.functions[index];
        let func_id = self.functions[index];

        context.func.signature =
            self.signature_of(&function.signature.params, function.signature.ret);
        context.func.name = cl::UserFuncName::user(0, func_id.as_u32());

        {
            let mut builder = FunctionBuilder::new(&mut context.func, builder_context);
            let mut frame = Frame::default();

            for _ in &function.blocks {
                let block = builder.create_block();
                frame.blocks.push(block);
            }
            for slot in &function.slots {
                let data = cl::StackSlotData::new(
                    cl::StackSlotKind::ExplicitSlot,
                    slot.size,
                    align_shift(slot.align),
                );
                let handle = builder.create_sized_stack_slot(data);
                frame.slots.push(handle);
            }
            frame.values.resize(function.values.len(), None);
            for (block_index, block) in function.blocks.iter().enumerate() {
                for &param in &block.params {
                    let ty = self.clif_type(function.value_type(param));
                    let value = builder.append_block_param(frame.blocks[block_index], ty);
                    frame.values[param.index()] = Some(value);
                }
            }

            for block in block_order(function) {
                builder.switch_to_block(frame.blocks[block.index()]);
                for &inst in &function.block(block).instructions {
                    self.translate_inst(&mut builder, &mut frame, function, inst)?;
                }
                let terminator = &function.block(block).terminator;
                self.translate_terminator(&mut builder, &mut frame, function, terminator)?;
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        if self.options.dump_clif {
            eprintln!("{}", context.func.display());
        }

        self.module
            .define_function(func_id, context)
            .map_err(|error| {
                failure_at(
                    function.span,
                    format!("cannot compile `{}`: {error}", function.name),
                )
            })
    }

    // #### tham chieu tu ben trong mot than ham ####

    fn func_ref(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        id: FuncId,
    ) -> cl::FuncRef {
        if let Some(&existing) = frame.func_refs.get(&id) {
            return existing;
        }
        let handle = self.module.declare_func_in_func(id, builder.func);
        frame.func_refs.insert(id, handle);
        handle
    }

    fn data_ref(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        id: DataId,
    ) -> cl::GlobalValue {
        if let Some(&existing) = frame.data_refs.get(&id) {
            return existing;
        }
        let handle = self.module.declare_data_in_func(id, builder.func);
        frame.data_refs.insert(id, handle);
        handle
    }

    fn data_address(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        id: DataId,
    ) -> cl::Value {
        let handle = self.data_ref(builder, frame, id);
        builder.ins().symbol_value(self.pointer_type, handle)
    }

    fn sig_ref(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        sig: SigRef,
    ) -> cl::SigRef {
        if let Some(&existing) = frame.sig_refs.get(&sig.0) {
            return existing;
        }
        let signature = self.program.signature(sig);
        let signature = self.signature_of(&signature.params, signature.ret);
        let handle = builder.import_signature(signature);
        frame.sig_refs.insert(sig.0, handle);
        handle
    }

    fn call_runtime(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        entry: RuntimeFn,
        args: &[cl::Value],
    ) -> Result<Option<cl::Value>, CompileError> {
        let id = *self
            .runtime
            .get(&entry)
            .ok_or_else(|| failure(format!("`{}` was never declared", entry.symbol())))?;
        let handle = self.func_ref(builder, frame, id);
        let call = builder.ins().call(handle, args);
        let result = builder.inst_results(call).first().copied();
        if entry.signature().diverges {
            builder.ins().trap(TRAP_UNREACHABLE);
            let unreachable = builder.create_block();
            builder.switch_to_block(unreachable);
        }
        Ok(result)
    }

    fn guard(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        condition: cl::Value,
        entry: RuntimeFn,
        args: &[cl::Value],
    ) -> Result<(), CompileError> {
        let panicking = builder.create_block();
        let surviving = builder.create_block();
        builder
            .ins()
            .brif(condition, panicking, &[], surviving, &[]);
        builder.switch_to_block(panicking);
        self.call_runtime(builder, frame, entry, args)?;
        builder.ins().trap(TRAP_UNREACHABLE);
        builder.switch_to_block(surviving);
        Ok(())
    }

    // ---- lenh ----

    fn translate_inst(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        function: &Function,
        inst: InstRef,
    ) -> Result<(), CompileError> {
        let inst = function.inst(inst);
        let produced = self.gen_expr(builder, frame, inst)?;
        if let Some(result) = inst.result {
            let value = produced.ok_or_else(|| {
                failure_at(
                    inst.span,
                    format!("{:?} defines a value but produced none", inst.kind),
                )
            })?;
            frame.values[result.index()] = Some(value);
        }
        Ok(())
    }

    // ban dau tien cua gen_expr, he 2025. No khong co Frame, khong tra
    // Result, gap cai gi la thi panic luon. T de day vi con hai cho trong ban
    // moi t chep y nguyen tu day sang ma van chua hieu het (cai vuot dau khi
    // load i8, va cho tinh stride cua PtrAdd). Bao gio hieu han thi xoa.
    //
    // fn gen_expr(&mut self, b: &mut FunctionBuilder, inst: &Inst) -> Option<cl::Value> {
    //     match &inst.kind {
    //         InstKind::ConstInt(v) => Some(b.ins().iconst(types::I64, *v)),
    //         InstKind::ConstFloat(v) => Some(b.ins().f64const(*v)),
    //         InstKind::ConstBool(v) => Some(b.ins().iconst(types::I8, *v as i64)),
    //         InstKind::ConstChar(v) => Some(b.ins().iconst(types::I32, *v as i64)),
    //         InstKind::ConstNull => Some(b.ins().iconst(self.pointer_type, 0)),
    //
    //         InstKind::ConstString(t) => {
    //             let id = self.strings[t.index()];
    //             let gv = self.module.declare_data_in_func(id, b.func);
    //             Some(b.ins().symbol_value(self.pointer_type, gv))
    //         }
    //         InstKind::ConstFuncAddr(f) => {
    //             let id = self.functions[f.index()];
    //             let fr = self.module.declare_func_in_func(id, b.func);
    //             Some(b.ins().func_addr(self.pointer_type, fr))
    //         }
    //         InstKind::ConstItable(i) => {
    //             let id = self.itables[i.index()];
    //             let gv = self.module.declare_data_in_func(id, b.func);
    //             Some(b.ins().symbol_value(self.pointer_type, gv))
    //         }
    //
    //         InstKind::SlotAddr(s) => {
    //             // luc do t chua biet stack_addr can pointer_type
    //             Some(b.ins().stack_addr(self.pointer_type, self.slots[s.index()], 0))
    //         }
    //         InstKind::GlobalAddr(g) => {
    //             let id = self.globals[g.index()];
    //             let gv = self.module.declare_data_in_func(id, b.func);
    //             Some(b.ins().symbol_value(self.pointer_type, gv))
    //         }
    //         InstKind::Load { ptr, offset, ty } => {
    //             let p = self.val(*ptr);
    //             let t = self.clif_type(*ty);
    //             // i8 nap len la phai vuot dau? khong, uextend. Cho nay t sai
    //             // hai lan roi, bool ra 255 thay vi 1.
    //             Some(b.ins().load(t, MemFlags::trusted(), p, *offset))
    //         }
    //         InstKind::Store { ptr, offset, value } => {
    //             let p = self.val(*ptr);
    //             let v = self.val(*value);
    //             b.ins().store(MemFlags::trusted(), v, p, *offset);
    //             None
    //         }
    //         InstKind::PtrAdd { ptr, index, stride } => {
    //             let p = self.val(*ptr);
    //             let i = self.val(*index);
    //             // stride luon la luy thua cua 2 nen dich bit duoc. Neu sau
    //             // nay khong con the nua thi phai doi sang imul.
    //             let sh = (*stride as u64).trailing_zeros() as i64;
    //             let off = b.ins().ishl_imm(i, sh);
    //             Some(b.ins().iadd(p, off))
    //         }
    //         InstKind::PtrOffset { ptr, offset } => {
    //             let p = self.val(*ptr);
    //             Some(b.ins().iadd_imm(p, *offset))
    //         }
    //
    //         InstKind::Binary { op, lhs, rhs } => {
    //             let a = self.val(*lhs);
    //             let c = self.val(*rhs);
    //             Some(match op {
    //                 BinaryOp::IAdd => b.ins().iadd(a, c),
    //                 BinaryOp::ISub => b.ins().isub(a, c),
    //                 BinaryOp::IMul => b.ins().imul(a, c),
    //                 BinaryOp::SDiv => b.ins().sdiv(a, c),   // trap o int.min/-1!!
    //                 BinaryOp::UDiv => b.ins().udiv(a, c),
    //                 BinaryOp::FAdd => b.ins().fadd(a, c),
    //                 BinaryOp::FSub => b.ins().fsub(a, c),
    //                 BinaryOp::FMul => b.ins().fmul(a, c),
    //                 BinaryOp::FDiv => b.ins().fdiv(a, c),
    //                 BinaryOp::BAnd => b.ins().band(a, c),
    //                 BinaryOp::BOr => b.ins().bor(a, c),
    //                 BinaryOp::BXor => b.ins().bxor(a, c),
    //                 _ => panic!("chua lam: {:?}", op),
    //             })
    //         }
    //         InstKind::Unary { op, value } => {
    //             let v = self.val(*value);
    //             Some(match op {
    //                 UnaryOp::INeg => b.ins().ineg(v),
    //                 UnaryOp::FNeg => b.ins().fneg(v),
    //                 UnaryOp::BNot => b.ins().bnot(v),
    //                 UnaryOp::LNot => b.ins().bxor_imm(v, 1),
    //             })
    //         }
    //         InstKind::Select { cond, then_value, else_value } => {
    //             let c = self.val(*cond);
    //             let t = self.val(*then_value);
    //             let e = self.val(*else_value);
    //             Some(b.ins().select(c, t, e))
    //         }
    //
    //         InstKind::Call { func, args } => {
    //             let id = self.functions[func.index()];
    //             let fr = self.module.declare_func_in_func(id, b.func);
    //             let a: Vec<cl::Value> = args.iter().map(|v| self.val(*v)).collect();
    //             let call = b.ins().call(fr, &a);
    //             b.inst_results(call).first().copied()
    //         }
    //         InstKind::CallRuntime { entry, args } => {
    //             let id = self.runtime[entry];
    //             let fr = self.module.declare_func_in_func(id, b.func);
    //             let a: Vec<cl::Value> = args.iter().map(|v| self.val(*v)).collect();
    //             let call = b.ins().call(fr, &a);
    //             b.inst_results(call).first().copied()
    //         }
    //         InstKind::CallIndirect { callee, sig, args } => {
    //             let c = self.val(*callee);
    //             let s = self.sigs[sig.index()];
    //             let a: Vec<cl::Value> = args.iter().map(|v| self.val(*v)).collect();
    //             let call = b.ins().call_indirect(s, c, &a);
    //             b.inst_results(call).first().copied()
    //         }
    //         InstKind::CallClosure { .. } => panic!("chua lam: closure"),
    //         InstKind::CallInterface { .. } => panic!("chua lam: iface"),
    //
    //         InstKind::Alloc { type_id, size } => {
    //             let id = self.runtime[&RuntimeFn::Alloc];
    //             let fr = self.module.declare_func_in_func(id, b.func);
    //             let t = b.ins().iconst(types::I32, type_id.0 as i64);
    //             let n = b.ins().iconst(types::I64, *size as i64);
    //             let call = b.ins().call(fr, &[t, n]);
    //             Some(b.inst_results(call)[0])
    //         }
    //
    //         InstKind::ErrorPending => {
    //             let id = self.runtime[&RuntimeFn::ErrorPending];
    //             let fr = self.module.declare_func_in_func(id, b.func);
    //             let call = b.ins().call(fr, &[]);
    //             Some(b.inst_results(call)[0])
    //         }
    //
    //         // may cai guard hoi do t chua lam, cu de no no ra roi tinh
    //         InstKind::BoundsCheck { .. } => None,
    //         InstKind::NullCheck { .. } => None,
    //         InstKind::DivisorCheck { .. } => None,
    //         InstKind::ShiftCountCheck { .. } => None,
    //
    //         other => panic!("gen_expr chua lam: {:?}", other),
    //     }
    // }

    fn gen_expr(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        inst: &Inst,
    ) -> Result<Option<cl::Value>, CompileError> {
        let span = inst.span;
        let produced = match &inst.kind {
            // ---- constants ----
            InstKind::ConstInt(value) => Some(builder.ins().iconst(types::I64, *value)),
            InstKind::ConstFloat(value) => Some(builder.ins().f64const(*value)),
            InstKind::ConstBool(value) => Some(builder.ins().iconst(types::I64, i64::from(*value))),
            InstKind::ConstChar(value) => Some(builder.ins().iconst(types::I32, i64::from(*value))),
            InstKind::ConstNull => Some(builder.ins().iconst(self.pointer_type, 0)),
            InstKind::ConstString(text) => {
                let id = *self.strings.get(text.index()).ok_or_else(|| {
                    failure_at(span, format!("the IR refers to an unknown string {text:?}"))
                })?;
                Some(self.data_address(builder, frame, id))
            }
            InstKind::ConstFuncAddr(func) => {
                let id = self.function_id(*func)?;
                let handle = self.func_ref(builder, frame, id);
                Some(builder.ins().func_addr(self.pointer_type, handle))
            }
            InstKind::ConstItable(itable) => {
                let id = *self.itables.get(itable.index()).ok_or_else(|| {
                    failure_at(
                        span,
                        format!("the IR refers to an unknown itable {itable:?}"),
                    )
                })?;
                Some(self.data_address(builder, frame, id))
            }
            InstKind::ConstEnumSingleton { type_id, tag } => {
                let id = *self.singletons.get(&(type_id.0, *tag)).ok_or_else(|| {
                    failure_at(
                        span,
                        format!(
                            "no singleton was emitted for variant {tag} of type {}",
                            type_id.0
                        ),
                    )
                })?;
                Some(self.data_address(builder, frame, id))
            }

            // ---- arithmetic and logic ----
            InstKind::Binary { op, lhs, rhs } => {
                let lhs = frame.value(*lhs, span)?; let rhs = frame.value(*rhs, span)?;
                Some(self.binary(builder, frame, *op, lhs, rhs))
            }
            InstKind::Unary { op, value } => {
                let value = frame.value(*value, span)?;
                Some(match op {
                    UnaryOp::INeg  => builder.ins().ineg(value),
                    UnaryOp::FNeg  => builder.ins().fneg(value),
                    UnaryOp::Not   => builder.ins().bxor_imm(value, 1),
                })
            }
            InstKind::Compare { op, lhs, rhs } => {
                let lhs = frame.value(*lhs, span)?; let rhs = frame.value(*rhs, span)?;
                // integer_condition tra None nghia la so thuc, khong phai la loi
                Some(match integer_condition(*op) { Some(condition) => builder.ins().icmp(condition, lhs, rhs), None => builder.ins().fcmp(float_condition(*op), lhs, rhs) })
            }
            InstKind::Convert { op, value } => { let value = frame.value(*value, span)?; Some(self.convert(builder, *op, value)) }
            InstKind::Select {
                cond,
                then_value,
                else_value,
            } => {
                let cond = frame.value(*cond, span)?;
                let then_value = frame.value(*then_value, span)?;
                let else_value = frame.value(*else_value, span)?;
                Some(builder.ins().select(cond, then_value, else_value))
            }

            // ==== memory ====
            InstKind::SlotAddr(slot) => {
                let handle = *frame.slots.get(slot.index()).ok_or_else(|| {
                    failure_at(span, format!("the IR refers to an unknown slot {slot:?}"))
                })?;
                Some(builder.ins().stack_addr(self.pointer_type, handle, 0))
            }
            InstKind::GlobalAddr(global) => {
                let storage = *self.globals.get(global.index()).ok_or_else(|| {
                    failure_at(
                        span,
                        format!("the IR refers to an unknown global {global:?}"),
                    )
                })?;
                let base = self.data_address(builder, frame, storage.data);
                Some(if storage.offset == 0 {
                    base
                } else {
                    builder.ins().iadd_imm(base, storage.offset)
                })
            }
            InstKind::Load { ptr, offset, ty } => {
                let ptr = frame.value(*ptr, span)?;
                let ty = self.clif_type(*ty);
                Some(builder.ins().load(ty, mem_flags(), ptr, *offset))
            }
            InstKind::Store { ptr, offset, value } => {
                let ptr = frame.value(*ptr, span)?;
                let value = frame.value(*value, span)?;
                builder.ins().store(mem_flags(), value, ptr, *offset);
                None
            }
            InstKind::PtrAdd { ptr, index, stride } => {
                let ptr = frame.value(*ptr, span)?;
                let index = frame.value(*index, span)?;
                let displacement = builder.ins().imul_imm(index, i64::from(*stride));
                Some(builder.ins().iadd(ptr, displacement))
            }
            InstKind::PtrOffset { ptr, offset } => {
                let ptr = frame.value(*ptr, span)?;
                Some(builder.ins().iadd_imm(ptr, *offset))
            }

            // #### allocation ####
            InstKind::Alloc { type_id, size } => {
                let type_id = builder.ins().iconst(types::I32, i64::from(type_id.0));
                let size = builder.ins().iconst(types::I64, *size as i64);
                self.call_runtime(builder, frame, RuntimeFn::Alloc, &[type_id, size])?
            }
            InstKind::AllocVariable { type_id, size } => {
                let size = frame.value(*size, span)?;
                let type_id = builder.ins().iconst(types::I32, i64::from(type_id.0));
                self.call_runtime(builder, frame, RuntimeFn::Alloc, &[type_id, size])?
            }

            // ==== calls ====
            InstKind::Call { func, args } => {
                let args = frame.values_of(args, span)?;
                let id = self.function_id(*func)?;
                let handle = self.func_ref(builder, frame, id);
                let call = builder.ins().call(handle, &args);
                builder.inst_results(call).first().copied()
            }
            InstKind::CallIndirect { callee, sig, args } => {
                let callee = frame.value(*callee, span)?;
                let args = frame.values_of(args, span)?;
                let signature = self.sig_ref(builder, frame, *sig);
                let call = builder.ins().call_indirect(signature, callee, &args);
                builder.inst_results(call).first().copied()
            }
            InstKind::CallClosure { closure, sig, args } => {
                let closure = frame.value(*closure, span)?;
                let code = builder.ins().load(
                    self.pointer_type,
                    mem_flags(),
                    closure,
                    abi::closure::CODE_OFFSET as i32,
                );
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(closure);
                all.extend(frame.values_of(args, span)?);
                let signature = self.sig_ref(builder, frame, *sig);
                let call = builder.ins().call_indirect(signature, code, &all);
                builder.inst_results(call).first().copied()
            }
            InstKind::CallInterface {
                object,
                slot,
                sig,
                args,
            } => {
                let object = frame.value(*object, span)?;
                let itable = builder.ins().load(
                    self.pointer_type,
                    mem_flags(),
                    object,
                    abi::interface::ITABLE_OFFSET as i32,
                );
                let data = builder.ins().load(
                    self.pointer_type,
                    mem_flags(),
                    object,
                    abi::interface::DATA_OFFSET as i32,
                );
                let method = builder.ins().load(
                    self.pointer_type,
                    mem_flags(),
                    itable,
                    abi::itable_method_offset(*slot) as i32,
                );
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(data);
                all.extend(frame.values_of(args, span)?);
                let signature = self.sig_ref(builder, frame, *sig);
                let call = builder.ins().call_indirect(signature, method, &all);
                builder.inst_results(call).first().copied()
            }
            InstKind::CallRuntime { entry, args } => {
                let args = frame.values_of(args, span)?;
                self.call_runtime(builder, frame, *entry, &args)?
            }

            // ---- the pending-error slot ----
            InstKind::ErrorPending => {
                self.call_runtime(builder, frame, RuntimeFn::ErrorPending, &[])?
            }
            InstKind::ErrorTake => self.call_runtime(builder, frame, RuntimeFn::ErrorTake, &[])?,
            InstKind::ErrorSet { error } => {
                let error = frame.value(*error, span)?;
                self.call_runtime(builder, frame, RuntimeFn::ErrorSet, &[error])?;
                None
            }

            // ---- guards ----
            InstKind::BoundsCheck { index, length } => {
                let index = frame.value(*index, span)?;
                let length = frame.value(*length, span)?;
                // mot phep so khong dau la loai duoc ca chi so am lan chi
                // so vuot qua cuoi mang.
                let out_of_range =
                    builder
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThanOrEqual, index, length);
                self.guard(
                    builder,
                    frame,
                    out_of_range,
                    RuntimeFn::PanicIndex,
                    &[index, length],
                )?;
                None
            }
            InstKind::NullCheck { value } => {
                let value = frame.value(*value, span)?;
                let is_null = builder.ins().icmp_imm(IntCC::Equal, value, 0);
                self.guard(builder, frame, is_null, RuntimeFn::PanicNull, &[])?;
                None
            }
            InstKind::DivisorCheck { divisor } => {
                let divisor = frame.value(*divisor, span)?;
                let is_zero = builder.ins().icmp_imm(IntCC::Equal, divisor, 0);
                self.guard(builder, frame, is_zero, RuntimeFn::PanicDivideByZero, &[])?;
                None
            }
            InstKind::ShiftCountCheck { count } => {
                let count = frame.value(*count, span)?;
                let is_negative = builder.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.guard(
                    builder,
                    frame,
                    is_negative,
                    RuntimeFn::PanicNegativeShift,
                    &[count],
                )?;
                None
            }
        };
        Ok(produced)
    }

    fn binary(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        op: BinaryOp,
        lhs: cl::Value,
        rhs: cl::Value,
    ) -> cl::Value {
        match op {
            BinaryOp::IAdd => builder.ins().iadd(lhs, rhs),
            BinaryOp::ISub => builder.ins().isub(lhs, rhs),
            BinaryOp::IMul => builder.ins().imul(lhs, rhs),
            BinaryOp::SDiv => signed_divide(builder, lhs, rhs),
            BinaryOp::UDiv => builder.ins().udiv(lhs, rhs),
            BinaryOp::SRem => signed_remainder(builder, lhs, rhs),
            BinaryOp::URem => builder.ins().urem(lhs, rhs),

            BinaryOp::FAdd => builder.ins().fadd(lhs, rhs),
            BinaryOp::FSub => builder.ins().fsub(lhs, rhs),
            BinaryOp::FMul => builder.ins().fmul(lhs, rhs),
            BinaryOp::FDiv => builder.ins().fdiv(lhs, rhs),
            BinaryOp::FRem => {
                let handle = self.func_ref(builder, frame, self.fmod);
                let call = builder.ins().call(handle, &[lhs, rhs]);
                builder.inst_results(call)[0]
            }

            BinaryOp::BitAnd => builder.ins().band(lhs, rhs),
            BinaryOp::BitOr  => builder.ins().bor(lhs, rhs),
            BinaryOp::BitXor => builder.ins().bxor(lhs, rhs),
            // ba cai dich bit deu di chung mot duong vi Cranelift che so dem theo do rong toan hang con Pump thi khong, xem ham shift() o duoi, dung goi thang ishl/sshr/ushr o day
            BinaryOp::Shl | BinaryOp::AShr | BinaryOp::LShr => shift(builder, op, lhs, rhs),
        }
    }

    fn convert(&self, builder: &mut FunctionBuilder, op: ConvertOp, value: cl::Value) -> cl::Value {
        match op {
            ConvertOp::FloatToInt       => builder.ins().fcvt_to_sint_sat(types::I64, value),
            ConvertOp::FloatToUint      => builder.ins().fcvt_to_uint_sat(types::I64, value),
            ConvertOp::IntToFloat       => builder.ins().fcvt_from_sint(types::F64, value),
            ConvertOp::UintToFloat      => builder.ins().fcvt_from_uint(types::F64, value),
            ConvertOp::CharToInt        => builder.ins().uextend(types::I64, value),
            ConvertOp::SignExtend32To64 => builder.ins().sextend(types::I64, value),
            ConvertOp::BoolToInt        => builder.ins().uextend(types::I64, value),
            ConvertOp::IntToBool => {
                let bit = builder.ins().band_imm(value, 1);
                builder.ins().ireduce(types::I8, bit)
            }
            ConvertOp::IntToChar => builder.ins().ireduce(types::I32, value),
            ConvertOp::BitcastFloatToInt => {
                builder.ins().bitcast(types::I64, MemFlags::new(), value)
            }
            ConvertOp::BitcastIntToFloat => {
                builder.ins().bitcast(types::F64, MemFlags::new(), value)
            }
            // con tro rong dung bang i64 tren moi target ma Pump chay,
            // nen hai cai nay chi la doi cach bieu dien, khong co lenh nao
            // dang sau ca.
            ConvertOp::IntToPtr => {
                if self.pointer_type == types::I64 {
                    value
                } else {
                    builder.ins().ireduce(self.pointer_type, value)
                }
            }
            ConvertOp::PtrToInt => {
                if self.pointer_type == types::I64 {
                    value
                } else {
                    builder.ins().uextend(types::I64, value)
                }
            }
        }
    }

    // ==== terminators ====

    fn translate_terminator(
        &mut self,
        builder: &mut FunctionBuilder,
        frame: &mut Frame,
        function: &Function,
        terminator: &Terminator,
    ) -> Result<(), CompileError> {
        let span = function.span;
        match terminator {
            Terminator::Jump { target, args } => {
                let args = frame.block_args(args, span)?;
                builder.ins().jump(frame.blocks[target.index()], &args);
            }
            Terminator::Branch {
                cond,
                then_block,
                then_args,
                else_block,
                else_args,
            } => {
                let cond = frame.value(*cond, span)?;
                let then_args = frame.block_args(then_args, span)?;
                let else_args = frame.block_args(else_args, span)?;
                builder.ins().brif(
                    cond,
                    frame.blocks[then_block.index()],
                    &then_args,
                    frame.blocks[else_block.index()],
                    &else_args,
                );
            }
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let default = frame.blocks[default.index()];
                if cases.is_empty() {
                    builder.ins().jump(default, &[]);
                } else {
                    let mut value = frame.value(*value, span)?;
                    // tag cua enum va char den duoi dang i32; ca hai deu
                    // khong am nen noi rong ra la chinh xac.
                    if builder.func.dfg.value_type(value) == types::I32 {
                        value = builder.ins().uextend(types::I64, value);
                    }
                    let mut switch = Switch::new();
                    for case in cases {
                        switch.set_entry(
                            case.value as u64 as u128,
                            frame.blocks[case.target.index()],
                        );
                    }
                    switch.emit(builder, value, default);
                }
            }
            Terminator::Return { value } => match value {
                Some(value) => {
                    let value = frame.value(*value, span)?;
                    builder.ins().return_(&[value]);
                }
                None => {
                    builder.ins().return_(&[]);
                }
            },
            Terminator::ReturnError { error } => {
                let error = frame.value(*error, span)?;
                self.call_runtime(builder, frame, RuntimeFn::ErrorSet, &[error])?;
                match function.signature.ret {
                    Some(ty) => {
                        let zero = self.zero(builder, ty);
                        builder.ins().return_(&[zero]);
                    }
                    None => {
                        builder.ins().return_(&[]);
                    }
                }
            }
            Terminator::Unreachable => {
                builder.ins().trap(TRAP_UNREACHABLE);
            }
        }
        Ok(())
    }

    fn zero(&self, builder: &mut FunctionBuilder, ty: IrType) -> cl::Value {
        match ty {
            IrType::F64 => builder.ins().f64const(0.0),
            other => builder.ins().iconst(self.clif_type(other), 0),
        }
    }

    // #### compiler-emitted entry points ####

    fn define_program_main(&mut self) -> Result<(), CompileError> {
        let signature = self.signature_of(&[], Some(IrType::I32));
        let func_id = self
            .module
            .declare_function(abi::SYMBOL_PROGRAM_MAIN, Linkage::Export, &signature)
            .map_err(|error| {
                failure(format!(
                    "cannot declare `{}`: {error}",
                    abi::SYMBOL_PROGRAM_MAIN
                ))
            })?;
        self.program_main = Some(func_id);

        let message = self.define_bytes(UNHANDLED_ERROR_MESSAGE)?;

        let mut context = Context::new();
        context.func.signature = signature;
        context.func.name = cl::UserFuncName::user(0, func_id.as_u32());
        let mut builder_context = FunctionBuilderContext::new();

        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let mut frame = Frame::default();
            let entry = builder.create_block();
            builder.switch_to_block(entry);

            let mut result = None;
            if let Some(main) = self.program.entry {
                let failable = self.program.function(main).failable;
                let id = self.function_id(main)?;
                let handle = self.func_ref(&mut builder, &mut frame, id);
                let call = builder.ins().call(handle, &[]);
                result = builder.inst_results(call).first().copied();

                if failable {
                    let pending = self
                        .call_runtime(&mut builder, &mut frame, RuntimeFn::ErrorPending, &[])?
                        .ok_or_else(|| failure("`pump_error_pending` returned nothing"))?;
                    let text = self.data_address(&mut builder, &mut frame, message);
                    let length = builder
                        .ins()
                        .iconst(types::I64, UNHANDLED_ERROR_MESSAGE.len() as i64);
                    self.guard(
                        &mut builder,
                        &mut frame,
                        pending,
                        RuntimeFn::PanicCstr,
                        &[text, length],
                    )?;
                }
            }

            let code = match result {
                Some(value) if builder.func.dfg.value_type(value) == types::I64 => {
                    builder.ins().ireduce(types::I32, value)
                }
                _ => builder.ins().iconst(types::I32, 0),
            };
            builder.ins().return_(&[code]);

            builder.seal_all_blocks();
            builder.finalize();
        }

        if self.options.dump_clif {
            eprintln!("{}", context.func.display());
        }

        self.module
            .define_function(func_id, &mut context)
            .map_err(|error| {
                failure(format!(
                    "cannot compile `{}`: {error}",
                    abi::SYMBOL_PROGRAM_MAIN
                ))
            })
    }

    fn define_c_main(&mut self) -> Result<(), CompileError> {
        let signature = self.signature_of(&[IrType::I32, IrType::Ptr], Some(IrType::I32));
        let func_id = self
            .module
            .declare_function(abi::SYMBOL_C_MAIN, Linkage::Export, &signature)
            .map_err(|error| {
                failure(format!("cannot declare `{}`: {error}", abi::SYMBOL_C_MAIN))
            })?;

        let module_init = self
            .program
            .module_init
            .ok_or_else(|| failure("the IR has no `pump_module_init`"))?;
        let module_init = self.function_id(module_init)?;
        let program_main = self
            .program_main
            .ok_or_else(|| failure("`pump_program_main` was not emitted"))?;

        let type_count = self.program.type_descriptors.len() as i64;
        let root_count = self.root_count as i64;
        let global_roots = self.global_roots;
        let type_table = self.type_table;

        let mut context = Context::new();
        context.func.signature = signature;
        context.func.name = cl::UserFuncName::user(0, func_id.as_u32());
        let mut builder_context = FunctionBuilderContext::new();

        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let mut frame = Frame::default();
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            let argc = builder.block_params(entry)[0];
            let argv = builder.block_params(entry)[1];

            // dia chi mot bien cuc bo trong frame nay la dau xa cua khoang
            // ma GC quet kieu bao thu.
            let anchor = builder.create_sized_stack_slot(cl::StackSlotData::new(
                cl::StackSlotKind::ExplicitSlot,
                abi::SLOT_SIZE,
                align_shift(abi::SLOT_SIZE),
            ));
            let stack_bottom = builder.ins().stack_addr(self.pointer_type, anchor, 0);

            let table = self.data_address(&mut builder, &mut frame, type_table);
            let types_len = builder.ins().iconst(types::I64, type_count);
            let roots = self.data_address(&mut builder, &mut frame, global_roots);
            let roots_len = builder.ins().iconst(types::I64, root_count);
            self.call_runtime(
                &mut builder,
                &mut frame,
                RuntimeFn::RtInit,
                &[stack_bottom, table, types_len, roots, roots_len, argc, argv],
            )?;

            let handle = self.func_ref(&mut builder, &mut frame, module_init);
            builder.ins().call(handle, &[]);

            let handle = self.func_ref(&mut builder, &mut frame, program_main);
            let call = builder.ins().call(handle, &[]);
            let code = builder.inst_results(call)[0];

            self.call_runtime(&mut builder, &mut frame, RuntimeFn::RtShutdown, &[code])?;
            builder.ins().return_(&[code]);

            builder.seal_all_blocks();
            builder.finalize();
        }

        if self.options.dump_clif {
            eprintln!("{}", context.func.display());
        }

        self.module
            .define_function(func_id, &mut context)
            .map_err(|error| failure(format!("cannot compile `{}`: {error}", abi::SYMBOL_C_MAIN)))
    }
}

// ### per-function bookkeeping ##############################################

#[derive(Default)]
struct Frame {
    blocks: Vec<cl::Block>,
    values: Vec<Option<cl::Value>>,
    slots: Vec<cl::StackSlot>,
    func_refs: HashMap<FuncId, cl::FuncRef>,
    data_refs: HashMap<DataId, cl::GlobalValue>,
    sig_refs: HashMap<u32, cl::SigRef>,
}

impl Frame {
    fn value(&self, value: Value, span: Span) -> Result<cl::Value, CompileError> {
        self.values
            .get(value.index())
            .copied()
            .flatten()
            .ok_or_else(|| failure_at(span, format!("{value:?} is used before it is defined")))
    }

    fn values_of(&self, values: &[Value], span: Span) -> Result<Vec<cl::Value>, CompileError> {
        values
            .iter()
            .map(|&value| self.value(value, span))
            .collect()
    }

    fn block_args(&self, values: &[Value], span: Span) -> Result<Vec<cl::BlockArg>, CompileError> {
        values
            .iter()
            .map(|&value| self.value(value, span).map(cl::BlockArg::Value))
            .collect()
    }
}

// --- translation helpers -------------------------------------

fn block_order(function: &Function) -> Vec<BlockRef> {
    let successors: Vec<Vec<BlockRef>> = function
        .blocks
        .iter()
        .map(|block| block.terminator.successors())
        .collect();

    let mut visited = vec![false; function.blocks.len()];
    let mut postorder = Vec::with_capacity(function.blocks.len());
    let mut stack = vec![(function.entry, 0usize)];
    visited[function.entry.index()] = true;

    while let Some((block, next)) = stack.pop() {
        let edges = &successors[block.index()];
        if next < edges.len() {
            stack.push((block, next + 1));
            let successor = edges[next];
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                stack.push((successor, 0));
            }
        } else {
            postorder.push(block);
        }
    }

    postorder.reverse();
    for index in 0..function.blocks.len() {
        if !visited[index] {
            postorder.push(BlockRef(index as u32));
        }
    }
    postorder
}

fn align_shift(align: u32) -> u8 {
    align.max(1).trailing_zeros() as u8
}


// TODO: bo cai nay di, viet ra roi khong dung
fn dbg_dump(tag: &str, n: usize) {
    if std::env::var("PUMP_DBG").is_ok() {
        eprintln!("[{}] {}", tag, n);
    }
}
fn signed_divide(b: &mut FunctionBuilder, lhs: cl::Value, rhs: cl::Value) -> cl::Value {
    // Cranelift trap o int.min / -1, Pump thi cuon ve int.min. Nen chia cho
    // -1 t doi thanh chia cho 1 roi tu doi dau lay thuong.
    let ty = b.func.dfg.value_type(rhs);
    let m1 = b.ins().icmp_imm(IntCC::Equal, rhs, -1);
    let one = b.ins().iconst(ty, 1);
    let d = b.ins().select(m1, one, rhs);
    let q = b.ins().sdiv(lhs, d);
    let neg = b.ins().ineg(lhs);
    b.ins().select(m1, neg, q)
}

fn signed_remainder(b: &mut FunctionBuilder, lhs: cl::Value, rhs: cl::Value) -> cl::Value {
    let ty = b.func.dfg.value_type(rhs);
    let m1 = b.ins().icmp_imm(IntCC::Equal, rhs, -1);
    let one = b.ins().iconst(ty, 1);
    let d = b.ins().select(m1, one, rhs);
    // so du thi khong phai doi dau gi ca, int.min % 1 == 0 san roi
    b.ins().srem(lhs, d)
}

// con ma o block ba. Hoi thang 8 cai nay sinh ma sai, ma sai duy nhat khi so
// paramter cua ham la so le va co dung mot cai closure o trong. Ngoi tu chin
// gio toi den hai gio sang moi ra: t quen mat cai nhanh saturated, Cranelift
// no che so dem theo do rong toan hang nen `x << 64` no tra lai dung `x`.
// De nguyen may dong duoi nay, dung "don gian hoa" lai bang mot lenh ishl.
fn shift(b: &mut FunctionBuilder, op: BinaryOp, lhs: cl::Value, count: cl::Value) -> cl::Value {
    let ty = b.func.dfg.value_type(lhs);
    let width = i64::from(ty.bits());
    let masked = match op {
        BinaryOp::Shl  => b.ins().ishl(lhs, count),
        BinaryOp::AShr => b.ins().sshr(lhs, count),
        BinaryOp::LShr => b.ins().ushr(lhs, count),
        other => unreachable!("{other:?} is not a shift"),
    };
    let sat = match op {
        // dich phai so hoc qua do rong thi con lai moi bit dau, boi ra ca tu.
        BinaryOp::AShr => b.ins().sshr_imm(lhs, width - 1),
        _ => b.ins().iconst(ty, 0),
    };
    let wide = b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, count, width);
    // println!("DBG: shift op={:?} width={}", op, width);
    b.ins().select(wide, sat, masked)
}

fn integer_condition(op: CompareOp) -> Option<IntCC> {
    Some(match op {
        CompareOp::IEq => IntCC::Equal,
        CompareOp::INe => IntCC::NotEqual,
        CompareOp::SLt => IntCC::SignedLessThan,
        CompareOp::SGt => IntCC::SignedGreaterThan,
        CompareOp::SLe => IntCC::SignedLessThanOrEqual,
        CompareOp::SGe => IntCC::SignedGreaterThanOrEqual,
        CompareOp::ULt => IntCC::UnsignedLessThan,
        CompareOp::UGt => IntCC::UnsignedGreaterThan,
        CompareOp::ULe => IntCC::UnsignedLessThanOrEqual,
        CompareOp::UGe => IntCC::UnsignedGreaterThanOrEqual,
        _ => return None,
    })
}

fn float_condition(op: CompareOp) -> FloatCC {
    match op {
        CompareOp::FEq => FloatCC::Equal,
        // dung dang unordered, de `NaN != NaN` ra true.
        CompareOp::FNe => FloatCC::NotEqual,
        CompareOp::FLt => FloatCC::LessThan,
        CompareOp::FGt => FloatCC::GreaterThan,
        CompareOp::FLe => FloatCC::LessThanOrEqual,
        CompareOp::FGe => FloatCC::GreaterThanOrEqual,
        other => unreachable!("{other:?} is an integer comparison"),
    }
}

fn write_descriptor(image: &mut Image, descriptor: &TypeDescriptor) {
    let start = image.len();
    image.u32(descriptor.kind as u32);
    image.u32(descriptor.flags);
    image.u64(descriptor.size);
    image.u32(descriptor.ref_offsets.len() as u32);
    image.u32(descriptor.variants.len() as u32);
    image.u64(0);
    image.u64(0);
    image.u64(0);
    debug_assert_eq!(image.len() - start, abi::TYPE_DESCRIPTOR_SIZE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BlockRef, Signature};

    fn span() -> Span {
        Span::synthetic()
    }

    #[test]
    fn alignments_become_power_of_two_exponents() {
        assert_eq!(align_shift(1), 0);
        assert_eq!(align_shift(4), 2);
        assert_eq!(align_shift(8), 3);
        assert_eq!(align_shift(16), 4);
    }

    #[test]
    fn a_straight_line_function_keeps_its_block_order() {
        let mut function = Function::new("f", Signature::new(Vec::new(), None), span());
        let entry = function.entry;
        let middle = function.new_block(span());
        let exit = function.new_block(span());
        function.set_terminator(
            entry,
            Terminator::Jump {
                target: middle,
                args: Vec::new(),
            },
        );
        function.set_terminator(
            middle,
            Terminator::Jump {
                target: exit,
                args: Vec::new(),
            },
        );
        function.set_terminator(exit, Terminator::Return { value: None });

        assert_eq!(block_order(&function), vec![entry, middle, exit]);
    }

    #[test]
    fn a_definition_is_ordered_before_its_use() {
        // block dau nhay toi block SAU no, roi cai do lai roi vao block
        // truoc, nen di theo thu tu chi so thi gap cho dung truoc cho dinh
        // nghia.
        let mut function = Function::new("f", Signature::new(Vec::new(), None), span());
        let entry = function.entry;
        let user = function.new_block(span());
        let definer = function.new_block(span());
        function.set_terminator(
            entry,
            Terminator::Jump {
                target: definer,
                args: Vec::new(),
            },
        );
        function.set_terminator(
            definer,
            Terminator::Jump {
                target: user,
                args: Vec::new(),
            },
        );
        function.set_terminator(user, Terminator::Return { value: None });

        let order = block_order(&function);
        let position = |block: BlockRef| order.iter().position(|&other| other == block).unwrap();
        assert!(position(definer) < position(user));
    }

    #[test]
    fn an_unreachable_block_is_still_translated() {
        let mut function = Function::new("f", Signature::new(Vec::new(), None), span());
        let entry = function.entry;
        let orphan = function.new_block(span());
        function.set_terminator(entry, Terminator::Return { value: None });

        let order = block_order(&function);
        assert_eq!(order.len(), 2);
        assert!(order.contains(&orphan));
    }

    #[test]
    fn a_descriptor_entry_is_forty_eight_bytes() {
        let descriptor = TypeDescriptor {
            type_id: abi::FIRST_USER_TYPE_ID,
            kind: abi::DescriptorKind::Struct,
            flags: 0,
            size: 48,
            ref_offsets: vec![32],
            variants: Vec::new(),
            name: "User".to_string(),
        };
        let mut image = Image::default();
        write_descriptor(&mut image, &descriptor);
        assert_eq!(image.len(), abi::TYPE_DESCRIPTOR_SIZE);
    }
}
