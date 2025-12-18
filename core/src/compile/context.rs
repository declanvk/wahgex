//! This module defines the `CompileContext` and associated structures
//! used for compiling a regular expression NFA into a WASM module.

use std::{alloc::Layout, borrow::Cow, collections::BTreeMap};

use regex_automata::nfa::thompson::NFA;
use wasm_encoder::{
    BranchHint, BranchHints, CodeSection, ConstExpr, DataSection, ElementSection, Elements,
    ExportKind, ExportSection, FunctionSection, ImportSection, IndirectNameMap, MemorySection,
    MemoryType, Module, NameMap, NameSection, RefType, TableSection, TableType, TypeSection,
    ValType,
};

/// This struct contains all the input and intermediate state needed to compile
/// the WASM module.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompileContext {
    pub nfa: NFA,
    pub config: crate::Config,
    pub sections: Sections,
    state_id_layout: Layout,
}

/// Contains the various sections of a WASM module being built.
/// Declarations are added here, and definitions are stored for later assembly.
#[derive(Debug, Default)]
pub struct Sections {
    types: TypeSection,
    imports: ImportSection,
    functions: FunctionSection,
    tables: TableSection,
    memories: MemorySection,
    exports: ExportSection,
    elements: ElementSection,
    data: DataSection,

    // Name map
    function_names: NameMap,
    table_names: NameMap,
    memory_names: NameMap,
    type_names: NameMap,
    data_names: NameMap,
    element_names: NameMap,

    // Stores function definitions, keyed by FunctionIdx.0, to be assembled later.
    function_definitions: BTreeMap<u32, FunctionDefinition>,
}

impl Sections {
    /// Adds an active data segment to the data section.
    /// These segments are copied into a linear memory at a specified offset
    /// during instantiation. Currently, all active data segments are
    /// hardcoded to target memory index 1 (state memory).
    pub fn add_active_data_segment(&mut self, segment: ActiveDataSegment) {
        let offset = ConstExpr::i64_const(
            segment
                .position
                .try_into()
                .expect("Data segment position too large for i64"),
        );
        let data_idx = self.data.len();
        // TODO: Make the memory index configurable or determined dynamically if
        // multiple memories are used beyond haystack (0) and state (1).
        self.data.active(1, &offset, segment.data.iter().copied());
        self.data_names.append(data_idx, &segment.name);
    }
}

impl CompileContext {
    /// Creates a new `CompileContext` with the given NFA and configuration.
    pub fn new(nfa: NFA, config: crate::Config) -> Self {
        let state_id_layout = Self::compute_state_id_layout(&nfa);

        Self {
            nfa,
            config,
            sections: Sections::default(),
            state_id_layout,
        }
    }

    /// Declare a new table and active element that will fill the table with the
    /// provided elements.
    pub fn add_call_indirect_table(
        &mut self,
        name: String,
        functions: impl IntoIterator<Item = FunctionIdx>,
    ) -> TableIdx {
        let functions: Vec<_> = functions.into_iter().map(u32::from).collect();
        let size =
            u64::try_from(functions.len()).expect("number of table elements should fit in u64");
        let table_idx = self.sections.tables.len();
        self.sections.tables.table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: size,
            maximum: Some(size),
            shared: false,
        });

        // We use an active element to copy all the function references into the table
        // on module initialization
        let element_idx = self.sections.elements.len();
        self.sections.elements.active(
            Some(table_idx),
            &ConstExpr::i32_const(0),
            Elements::Functions(Cow::Borrowed(&functions)),
        );
        self.sections
            .table_names
            .append(table_idx, &format!("{name}_table"));
        self.sections
            .element_names
            .append(element_idx, &format!("{name}_elements"));
        TableIdx(table_idx)
    }

    /// Declare and define a function.
    pub fn add_function(&mut self, func: Function) -> FunctionIdx {
        let func_idx = self.declare_function_and_fn_type(func.sig);
        self.define_function(func_idx, func.def);
        func_idx
    }

    /// Declare a function type.
    pub fn declare_fn_type(&mut self, sig: &FunctionTypeSignature) -> TypeIdx {
        let func_ty_idx = self.sections.types.len();
        self.sections.types.ty().function(
            sig.params_ty.iter().copied(),
            sig.results_ty.iter().copied(),
        );
        self.sections
            .type_names
            .append(func_ty_idx, &sig.type_name());
        TypeIdx(func_ty_idx)
    }

    /// Declares a function's signature (name, parameters, return types, export
    /// status).
    pub fn declare_function_with_type(
        &mut self,
        func_ty_idx: TypeIdx,
        name: &str,
        export: bool,
    ) -> FunctionIdx {
        let func_idx_val = self.sections.functions.len();
        self.sections.functions.function(func_ty_idx.into());
        self.sections.function_names.append(func_idx_val, name);

        #[cfg(test)]
        let override_export = self.config.get_export_all_functions();
        #[cfg(not(test))]
        let override_export = false;

        if export || override_export {
            self.sections
                .exports
                .export(name, ExportKind::Func, func_idx_val);
        }
        FunctionIdx(func_idx_val)
    }

    /// Declares a function's signature (name, parameters, return types, export
    /// status).
    ///
    /// This adds entries to the Type, Function, and potentially
    /// Export sections. A `FunctionIdx` is returned, which should be used
    /// later to provide the definition.
    pub fn declare_function_and_fn_type(&mut self, sig: FunctionSignature) -> FunctionIdx {
        let func_ty_idx = self.declare_fn_type(&FunctionTypeSignature {
            name: &sig.name,
            params_ty: sig.params_ty,
            results_ty: sig.results_ty,
        });
        self.declare_function_with_type(func_ty_idx, &sig.name, sig.export)
    }

    /// Defines a previously declared function.
    ///
    /// The `func_idx` must correspond to a function previously returned by
    /// `declare_function`. The definition includes the body, local names,
    /// label names, and branch hints.
    pub fn define_function(&mut self, func_idx: FunctionIdx, def: FunctionDefinition) {
        if func_idx.0 >= self.sections.functions.len() {
            panic!(
                "Defining function with index {} which has not been declared (max declared index: \
                 {})",
                func_idx.0,
                if !self.sections.functions.is_empty() {
                    self.sections.functions.len() - 1
                } else {
                    0
                }
            );
        }
        if self
            .sections
            .function_definitions
            .insert(func_idx.0, def)
            .is_some()
        {
            panic!("Warning: Redefining function at index {}", func_idx.0);
        }
    }

    /// Adds a block signature to the type section.
    ///
    /// This is used for block types in control flow instructions.
    pub fn add_block_signature(&mut self, signature: BlockSignature) -> TypeIdx {
        let block_ty_idx = self.sections.types.len();
        self.sections.types.ty().function(
            signature.params_ty.iter().copied(),
            signature.results_ty.iter().copied(),
        );
        self.sections
            .type_names
            .append(block_ty_idx, &signature.type_name());
        TypeIdx(block_ty_idx)
    }

    /// Return the minimal layout for a state ID for the current NFA.
    ///
    /// This function will minimize the size of the state ID layout based on the
    /// number of states in the current NFA.
    pub fn state_id_layout(&self) -> &Layout {
        &self.state_id_layout
    }

    fn compute_state_id_layout(nfa: &NFA) -> Layout {
        let num_states = nfa.states().len();

        if num_states <= u8::MAX as usize {
            Layout::from_size_align(1, 1).unwrap()
        } else if num_states <= u16::MAX as usize {
            Layout::from_size_align(2, 2).unwrap()
        } else {
            Layout::from_size_align(4, 4).unwrap()
        }
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

        module.section(&self.sections.tables);

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
        let state_mem_size =
            1 + u64::try_from((state_overall.size() - 1) / self.config.get_page_size()).unwrap();
        self.sections.memories.memory(MemoryType {
            minimum: state_mem_size,
            maximum: Some(state_mem_size),
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

        module.section(&self.sections.elements);

        // Build CodeSection, BranchHints, and name maps for locals/labels from
        // definitions
        let mut codes = CodeSection::new();
        let mut hint_section = BranchHints::new();
        let mut local_names = IndirectNameMap::new();
        let mut label_names = IndirectNameMap::new();

        let num_declared_functions = self.sections.functions.len();

        // Ensure all declared functions have corresponding definitions.
        // BTreeMap iteration is ordered by key, which is FunctionIdx.0.
        // We iterate 0..num_declared_functions to ensure correct order and that all are
        // present.
        for func_idx_val in 0..num_declared_functions {
            match self.sections.function_definitions.get(&func_idx_val) {
                Some(def) => {
                    codes.function(&def.body);
                    local_names.append(func_idx_val, &def.locals_name_map);
                    if let Some(labels) = &def.labels_name_map {
                        label_names.append(func_idx_val, labels);
                    }
                    if let Some(hints) = &def.branch_hints {
                        hint_section.function_hints(func_idx_val, hints.iter().copied());
                    }
                },
                None => {
                    panic!("Function at index {func_idx_val} was declared but not defined.");
                },
            }
        }

        module.section(&hint_section);

        module.section(&codes);

        module.section(&self.sections.data);

        let mut name_section = NameSection::new();
        {
            name_section.functions(&self.sections.function_names);

            name_section.locals(&local_names);

            name_section.labels(&label_names);

            name_section.types(&self.sections.type_names);

            name_section.tables(&self.sections.table_names);

            {
                self.sections
                    .memory_names
                    .append(haystack_mem_idx, "haystack");
                self.sections.memory_names.append(state_mem_idx, "state"); // Assuming state_mem_idx is valid
            }
            name_section.memories(&self.sections.memory_names);

            name_section.elements(&self.sections.element_names);

            name_section.data(&self.sections.data_names);
        }
        module.section(&name_section);

        module
    }
}

/// Represents an active data segment to be included in the WASM module.
#[derive(Debug)]
pub struct ActiveDataSegment {
    pub name: String,
    pub position: usize,
    pub data: Vec<u8>,
}

/// Describes the signature of a function: its name, parameters, results, and
/// export status.
#[derive(Debug)]
pub struct FunctionSignature {
    pub name: String,
    pub params_ty: &'static [ValType],
    pub results_ty: &'static [ValType],
    pub export: bool,
}

#[derive(Debug)]
pub struct FunctionTypeSignature<'n> {
    pub name: &'n str,
    pub params_ty: &'static [ValType],
    pub results_ty: &'static [ValType],
}

impl<'n> FunctionTypeSignature<'n> {
    /// Generates a unique name for this function's type signature.
    fn type_name(&self) -> String {
        format!("{}_fn", self.name)
    }
}

/// Contains the definition of a function: its body, local names, label names,
/// and branch hints.
///
/// This is associated with a `FunctionIdx` obtained from
/// [`CompileContext::declare_function_and_fn_type`].
#[derive(Debug)]
pub struct FunctionDefinition {
    pub body: wasm_encoder::Function,
    pub locals_name_map: NameMap,
    pub labels_name_map: Option<NameMap>,
    pub branch_hints: Option<Vec<BranchHint>>,
}

/// Contains the full definition of a function: signature and definition.
#[derive(Debug)]
pub struct Function {
    pub sig: FunctionSignature,
    pub def: FunctionDefinition,
}

/// Describes the signature of a block type (e.g., for `if`, `loop`, `block`).
/// It includes a descriptive name, parameter types, and result types.
#[derive(Debug)]
pub struct BlockSignature {
    pub name: &'static str,
    pub params_ty: &'static [ValType],
    pub results_ty: &'static [ValType],
}

impl BlockSignature {
    /// Generates a unique name for this block's type signature.
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

/// This index type represents a pointer to a specific table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableIdx(u32);

impl From<TableIdx> for u32 {
    fn from(idx: TableIdx) -> Self {
        idx.0
    }
}
