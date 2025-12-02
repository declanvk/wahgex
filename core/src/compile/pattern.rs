//! This module contains functions and lookup tables relating to multiple
//! patterns in one compiled regex.

use std::alloc::{Layout, LayoutError};

use regex_automata::nfa::thompson::NFA;
use wasm_encoder::{NameMap, ValType};

use crate::util::repeat;

use super::{
    context::{
        ActiveDataSegment, CompileContext, Function, FunctionDefinition, FunctionIdx,
        FunctionSignature,
    },
    instructions::InstructionSinkExt,
};

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct PatternLayout {
    pattern_start_table_pos: usize,
    pattern_start_stride: usize,
}

impl PatternLayout {
    /// TODO: Write docs for this item
    pub fn new(ctx: &mut CompileContext, overall: Layout) -> Result<(Layout, Self), LayoutError> {
        let pattern_start_table_data = ctx
            .nfa
            .patterns()
            // WASM assumes little endian byte ordering: https://webassembly.org/docs/portability/
            .flat_map(|pid| ctx.nfa.start_pattern(pid).unwrap().as_u32().to_le_bytes())
            .collect::<Vec<_>>();

        let (pattern_start_table, pattern_start_stride) =
            repeat(ctx.state_id_layout(), ctx.nfa.pattern_len())?;
        let (overall, pattern_start_table_pos) = overall.extend(pattern_start_table)?;

        ctx.sections.add_active_data_segment(ActiveDataSegment {
            name: "pattern_start_table".into(),
            position: pattern_start_table_pos,
            data: pattern_start_table_data,
        });

        Ok((
            overall,
            Self {
                pattern_start_table_pos,
                pattern_start_stride,
            },
        ))
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct PatternFunctions {
    pub lookup_start: FunctionIdx,
}

impl PatternFunctions {
    pub fn new(ctx: &mut CompileContext, layout: &PatternLayout) -> Self {
        let start_id = ctx.add_function(Self::lookup_start_fn(
            &ctx.nfa,
            layout,
            ctx.state_id_layout(),
        ));

        Self {
            lookup_start: start_id,
        }
    }

    fn lookup_start_fn(nfa: &NFA, layout: &PatternLayout, state_id_layout: &Layout) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "pattern_id");

        // Sketch:
        // ```rust
        // if pattern_id >= nfa.patterns_len() {
        //     return (0, false);
        // }
        //
        // start_state_id = pattern_start_table[pattern_id];
        // return (start_state_id, true);
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // if pattern_id >= nfa.patterns_len() {
            .local_get(0)
            .u32_const(u32::try_from(nfa.pattern_len()).expect("pattern len should fit in u32"))
            .i32_ge_u()
            .if_(wasm_encoder::BlockType::Empty)
            // return (0, false);
            .i32_const(0)
            .i32_const(false as i32)
            .return_()
            .end()
            // start_state_id = pattern_start_table[pattern_id];
            .local_get(0)
            .i64_extend_i32_u()
            .u64_const(u64::try_from(layout.pattern_start_stride).unwrap())
            .i64_mul()
            .state_id_load(
                u64::try_from(layout.pattern_start_table_pos).unwrap(),
                // state memory
                state_id_layout,
            )
            .i32_const(true as i32)
            .end();

        Function {
            sig: FunctionSignature {
                name: "lookup_start_id".into(),
                // [pattern_id]
                params_ty: &[ValType::I32],
                // [start_state_id, is_some]
                results_ty: &[ValType::I32, ValType::I32],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }
}
