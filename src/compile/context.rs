//! TODO: Write docs for this module

use std::alloc::Layout;

use wasm_encoder::{
    BranchHint, BranchHints, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection,
    FunctionSection, ImportSection, IndirectNameMap, MemorySection, MemoryType, Module, NameMap,
    NameSection, TypeSection, ValType,
};

/// This struct contains all the input and intermediate state needed to compile
/// the WASM module.
#[derive(Debug)]
pub struct CompileContext {
    pub nfa: regex_automata::nfa::thompson::NFA,
    pub config: crate::Config,
    pub sections: Sections,
}

/// TODO: Write docs for this item
#[derive(Debug, Default)]
pub struct Sections {
    #[cfg_attr(not(test), expect(dead_code))]
    config: crate::Config,
    types: TypeSection,
    imports: ImportSection,
    functions: FunctionSection,
    memories: MemorySection,
    exports: ExportSection,
    hint_section: BranchHints,
    codes: CodeSection,
    data: DataSection,
    function_names: NameMap,
    memory_names: NameMap,
    local_names: IndirectNameMap,
    label_names: IndirectNameMap,
    type_names: NameMap,
    data_names: NameMap,
}

impl CompileContext {
    /// TODO: Write docs for this item
    pub fn new(nfa: regex_automata::nfa::thompson::NFA, config: crate::Config) -> Self {
        Self {
            nfa,
            config,
            sections: Sections {
                config,
                ..Default::default()
            },
        }
    }
}

impl Sections {
    /// TODO: Write docs for this item
    pub fn add_function(&mut self, func: Function) -> FunctionIdx {
        let func_ty_idx = self.types.len();
        self.types.ty().function(
            func.params_ty.iter().copied(),
            func.results_ty.iter().copied(),
        );
        self.type_names.append(func_ty_idx, &func.type_name());

        let func_idx = self.functions.len();
        self.functions.function(func_ty_idx);
        self.function_names.append(func_idx, &func.name);

        #[cfg(test)]
        let override_export = self.config.get_export_all_functions();
        #[cfg(not(test))]
        let override_export = false;

        if func.export || override_export {
            self.exports.export(&func.name, ExportKind::Func, func_idx);
        }

        if let Some(hints) = &func.branch_hints {
            self.hint_section
                .function_hints(func_idx, hints.iter().copied());
        }

        self.codes.function(&func.body);

        self.local_names.append(func_idx, &func.locals_name_map);
        if let Some(label_names) = func.labels_name_map {
            self.label_names.append(func_idx, &label_names);
        }

        FunctionIdx(func_idx)
    }

    /// TODO: Write docs for this item
    pub fn add_block_signature(&mut self, signature: BlockSignature) -> TypeIdx {
        let block_ty_idx = self.types.len();
        self.types.ty().function(
            signature.params_ty.iter().copied(),
            signature.results_ty.iter().copied(),
        );
        self.type_names.append(block_ty_idx, &signature.type_name());

        TypeIdx(block_ty_idx)
    }

    /// TODO: Write docs for this item
    pub fn add_active_data_segment(&mut self, segment: ActiveDataSegment) {
        let offset = ConstExpr::i64_const(segment.position.try_into().unwrap());
        let data_idx = self.data.len();
        // All active data segments go in state memory
        self.data.active(1, &offset, segment.data);
        self.data_names.append(data_idx, &segment.name);
    }
}

impl CompileContext {
    /// This function takes all the individual settings/functions/data
    /// segments/layouts and compiles them into a single WASM [`Module`].
    pub fn compile(mut self, state_overall: &Layout) -> Module {
        let mut module = Module::new();

        // Section order
        //  Types
        //  Imports
        //  Functions
        //  Tables
        //  Memories
        //  Globals
        //  Exports
        //  Start
        //  Elements
        //  Data Count
        //  Code
        //  Data

        module.section(&self.sections.types);

        module.section(&self.sections.imports);

        module.section(&self.sections.functions);

        // Determine minimum (and maximum?) size based on data structure layout
        let haystack_mem_idx = self.sections.memories.len();
        self.sections.memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            // TODO: Make state memory64 default false by config
            memory64: true,
            shared: false,
            // TODO: Use custom page size
            page_size_log2: None,
        });
        let state_mem_idx = self.sections.memories.len();
        self.sections.memories.memory(MemoryType {
            // FIXME: determine state memory size exactly and bound it
            minimum: 1 + u64::try_from((state_overall.size() - 1) / self.config.get_page_size())
                .unwrap(),
            maximum: None,
            // TODO: Make state memory64 default false by config
            memory64: true,
            shared: false,
            // TODO: Use custom page size
            page_size_log2: None,
        });
        module.section(&self.sections.memories);

        self.sections
            .exports
            .export("haystack", ExportKind::Memory, haystack_mem_idx);

        #[cfg(test)]
        let export_state = self.config.get_export_state();
        #[cfg(not(test))]
        let export_state = false;
        if export_state {
            self.sections
                .exports
                .export("state", ExportKind::Memory, state_mem_idx);
        }
        module.section(&self.sections.exports);

        module.section(&self.sections.hint_section);

        module.section(&self.sections.codes);

        module.section(&self.sections.data);

        let mut name_section = NameSection::new();
        {
            name_section.functions(&self.sections.function_names);

            {
                self.sections
                    .memory_names
                    .append(haystack_mem_idx, "haystack");
                self.sections.memory_names.append(state_mem_idx, "state");
            };
            name_section.memories(&self.sections.memory_names);

            name_section.locals(&self.sections.local_names);

            name_section.labels(&self.sections.label_names);

            name_section.types(&self.sections.type_names);

            name_section.data(&self.sections.data_names);
        }
        module.section(&name_section);

        module
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct ActiveDataSegment {
    pub name: String,
    pub position: usize,
    pub data: Vec<u8>,
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params_ty: &'static [ValType],
    pub results_ty: &'static [ValType],
    pub export: bool,
    pub body: wasm_encoder::Function,
    pub locals_name_map: NameMap,
    pub labels_name_map: Option<NameMap>,
    pub branch_hints: Option<Vec<BranchHint>>,
}

impl Function {
    fn type_name(&self) -> String {
        format!("{}_fn", self.name)
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct BlockSignature {
    pub name: &'static str,
    pub params_ty: &'static [ValType],
    pub results_ty: &'static [ValType],
}

impl BlockSignature {
    fn type_name(&self) -> String {
        format!("{}_block_sig", self.name)
    }
}

/// This index type represents a pointer to a specific [`Function`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionIdx(u32);

impl From<FunctionIdx> for u32 {
    fn from(idx: FunctionIdx) -> Self {
        idx.0
    }
}

/// This index type represents a pointer to a specific type, be it function or
/// block signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeIdx(u32);

impl From<TypeIdx> for u32 {
    fn from(idx: TypeIdx) -> Self {
        idx.0
    }
}
