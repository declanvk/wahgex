//! This module contains types and functions related to laying out the input
//! options and haystack in the WASM memory.

use std::alloc::{Layout, LayoutError};

use regex_automata::{nfa::thompson::NFA, Anchored};
use wasm_encoder::{BlockType, NameMap, ValType};

use super::context::{
    BlockSignature, CompileContext, Function, FunctionDefinition, FunctionIdx, FunctionSignature,
    TypeIdx,
};

/// This type is a mirror of [`regex_automata::Input`], with guaranteed
/// alignment and no-substructs.
#[derive(Debug)]
#[repr(C)]
pub struct InputOpts {
    /// Whether to execute an "earliest" search or not.
    pub earliest: i32,
    /// Sets the anchor mode of a search.
    ///
    /// The translation:
    ///   - [`Anchored::No`] => `0`
    ///   - [`Anchored::Yes`] => `1`
    ///   - [`Anchored::Pattern`] => `2`
    pub anchored: i32,
    /// If `anchored` is equivalent to [`Anchored::Pattern`], then this is the
    /// [`PatternID`][regex_automata::util::primitives::PatternID].
    ///
    /// Otherwise, it is set to 0.
    pub anchored_pattern: i32,
}

impl InputOpts {
    /// Creates a new `InputOpts` from a [`regex_automata::Input`].
    ///
    /// This translates the anchor mode and earliest flag into i32 values
    /// suitable for WASM.
    pub fn new(input: &regex_automata::Input<'_>) -> InputOpts {
        let (anchored, anchored_pattern) = match input.get_anchored() {
            Anchored::No => (0, 0),
            Anchored::Yes => (1, 0),
            Anchored::Pattern(id) => (2, i32::from_ne_bytes(id.to_ne_bytes())),
        };

        InputOpts {
            earliest: input.get_earliest() as i32,
            anchored,
            anchored_pattern,
        }
    }
}

/// Defines the memory layout for input-related data within the WebAssembly
/// module.
///
/// This includes the starting position of the haystack.
#[derive(Debug)]
pub struct InputLayout {
    pub haystack_start_pos: usize,
    _overall: Layout,
}

impl InputLayout {
    /// Creates a new [`InputLayout`].
    ///
    /// Currently, this primarily determines the starting offset for the
    /// haystack.
    pub fn new(_ctx: &mut CompileContext) -> Result<Self, LayoutError> {
        let overall = Layout::new::<()>();

        // We use a zero-size array here to mark the start of the haystack, since we
        // don't know the length of it until runtime.
        let (overall, haystack_start_pos) = overall.extend(Layout::array::<u8>(0)?)?;

        Ok(Self {
            _overall: overall,
            haystack_start_pos,
        })
    }
}

/// Holds indices to WebAssembly functions related to input processing.
///
/// These functions are used by the compiled regex to manage and interpret the
/// input haystack.
#[derive(Debug)]
pub struct InputFunctions {
    #[expect(dead_code)]
    prepare_input: FunctionIdx,
    pub assert_input_args_wf: FunctionIdx,
    pub utf8_is_boundary: Option<FunctionIdx>,
    pub start_config: FunctionIdx,
}

impl InputFunctions {
    /// Creates and registers the necessary WebAssembly functions for input
    /// handling.
    ///
    /// This includes functions for preparing input memory, asserting argument
    /// well-formedness, checking UTF-8 boundaries, and configuring start
    /// conditions.
    pub fn new(
        ctx: &mut CompileContext,
        input_layout: &InputLayout,
        pattern_lookup_start: FunctionIdx,
    ) -> Self {
        let prepare_input = ctx.add_function(Self::prepare_input_fn(
            ctx.config.get_page_size(),
            input_layout,
        ));
        let assert_input_args_wf = ctx.add_function(Self::assert_input_args_wf_fn(
            ctx.config.get_page_size(),
            input_layout,
        ));

        let utf8_is_boundary = (ctx.nfa.has_empty() && ctx.nfa.is_utf8())
            .then(|| ctx.add_function(Self::utf8_is_boundary_fn()));

        let pattern_lookup_start_result_block_sig = ctx.add_block_signature(BlockSignature {
            name: "pattern_lookup_start_result",
            params_ty: &[ValType::I32],
            results_ty: &[],
        });

        let start_config = ctx.add_function(Self::start_config_fn(
            &ctx.nfa,
            pattern_lookup_start,
            pattern_lookup_start_result_block_sig,
        ));

        Self {
            prepare_input,
            assert_input_args_wf,
            utf8_is_boundary,
            start_config,
        }
    }

    fn start_config_fn(
        nfa: &NFA,
        pattern_lookup_start: FunctionIdx,
        pattern_lookup_start_result_block_sig: TypeIdx,
    ) -> Function {
        // Copied from https://github.com/rust-lang/regex/blob/1a069b9232c607b34c4937122361aa075ef573fa/regex-automata/src/nfa/thompson/pikevm.rs#L1751-L1785

        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "anchored");
        locals_name_map.append(1, "anchored_pattern");
        // Locals
        locals_name_map.append(2, "pattern_start");

        // Sketch:
        // ```rust
        // if anchored == Anchored::No {
        //     return (nfa.start_anchored(), nfa.is_always_start_anchored(), true);
        // }
        // if anchored == Anchored::Yes {
        //     return (nfa.start_anchored(), true, true);
        // }
        // if anchored == Anchored::Pattern {
        //     (pattern_start, is_some) = pattern_lookup_start(anchored_pattern);
        //     if is_some {
        //         return (pattern_start, true, true);
        //     }
        // }
        // return (0, 0, false);
        // ```

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);
        body.instructions()
            // if anchored == Anchored::No {
            .local_get(0)
            .i32_const(0) // Anchored::No
            .i32_eq()
            .if_(BlockType::Empty)
            //  return (nfa.start_anchored(), nfa.is_always_start_anchored(), true);
            .i32_const(i32::from_ne_bytes(nfa.start_anchored().to_ne_bytes()))
            .i32_const(nfa.is_always_start_anchored() as i32)
            .i32_const(true as i32)
            .return_()
            .end()
            // if anchored == Anchored::Yes {
            .local_get(0)
            .i32_const(1) // Anchored::Yes
            .i32_eq()
            .if_(BlockType::Empty)
            //  return (nfa.start_anchored(), true, true);
            .i32_const(i32::from_ne_bytes(nfa.start_anchored().to_ne_bytes()))
            .i32_const(true as i32)
            .i32_const(true as i32)
            .return_()
            .end()
            // if anchored == Anchored::Pattern {
            .local_get(0)
            .i32_const(2) // Anchored::Pattern
            .i32_eq()
            .if_(BlockType::Empty)
            // (pattern_start, is_some) = pattern_lookup_start(anchored_pattern);
            .local_get(1)
            .call(pattern_lookup_start.into())
            .if_(BlockType::FunctionType(
                pattern_lookup_start_result_block_sig.into(),
            ))
            //  return (pattern_start, true, true);
            .i32_const(true as i32)
            .i32_const(true as i32)
            .return_()
            .else_()
            .drop()
            .end()
            .end()
            // return (0, 0, false);
            .i32_const(0)
            .i32_const(0)
            .i32_const(false as i32)
            .end();

        Function {
            sig: FunctionSignature {
                name: "start_config".into(),
                // [anchored, anchored_pattern]
                params_ty: &[ValType::I32, ValType::I32],
                // [start_state_id, is_anchored, is_some]
                results_ty: &[ValType::I32, ValType::I32, ValType::I32],
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

    fn utf8_is_boundary_fn() -> Function {
        // Copied from https://github.com/rust-lang/regex/blob/1a069b9232c607b34c4937122361aa075ef573fa/regex-automata/src/util/utf8.rs#L117-L137

        // Sketch:
        // ```rust
        // if at_offset >= haystack_len {
        //     return at_offset == haystack_len;
        // }
        //
        // byte = haystack_ptr[at_offset];
        // return (byte <= 0b0111_1111 || byte >= 0b1100_0000);
        // ```

        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        // Locals
        locals_name_map.append(3, "byte");

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);
        body.instructions()
            // if at_offset >= haystack_len {
            .local_get(2)
            .local_get(1)
            .i64_ge_u()
            .if_(BlockType::Empty)
            // return at_offset == haystack_len
            .local_get(2)
            .local_get(1)
            .i64_eq() // returns either 0 or 1 as i32
            .return_()
            .end()
            // byte = haystack_ptr[at_offset];
            .local_get(0)
            .local_get(2)
            .i64_add()
            .i32_load8_u(wasm_encoder::MemArg {
                offset: 0,       // no compile-time offset
                align: 0,        // align of 1 since we're loading a byte
                memory_index: 0, // loading from haystack
            })
            .local_set(3)
            // return (byte <= 0b0111_1111 || byte >= 0b1100_0000);
            .local_get(3)
            .i32_const(0b0111_1111)
            .i32_le_u()
            .local_get(3)
            .i32_const(0b1100_0000)
            .i32_ge_u()
            .i32_or()
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_is_boundary".into(),
                // [haystack_ptr, haystack_len, at_offset]
                params_ty: &[ValType::I64, ValType::I64, ValType::I64],
                // [is_boundary]
                results_ty: &[ValType::I32],
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

    fn assert_input_args_wf_fn(page_size: usize, input_layout: &InputLayout) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "earliest");
        locals_name_map.append(1, "anchored");
        locals_name_map.append(2, "anchored_pattern");
        locals_name_map.append(3, "span_start");
        locals_name_map.append(4, "span_end");
        locals_name_map.append(5, "haystack_len");

        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_earliest_wf");
        labels_name_map.append(1, "check_anchored_wf");
        labels_name_map.append(2, "check_anchored_pattern_wf");
        labels_name_map.append(3, "check_span_start_wf");
        labels_name_map.append(4, "check_span_end_wf");

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // if earliest != true && earliest != false {
            .local_get(0)
            .i32_const(true as i32)
            .i32_ne()
            .local_get(0)
            .i32_const(false as i32)
            .i32_ne()
            .i32_and()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            // if anchored != 0 && anchored != 1 && anchored != 2 {
            .local_get(1)
            .i32_const(0)
            .i32_ne()
            .local_get(1)
            .i32_const(1)
            .i32_ne()
            .local_get(1)
            .i32_const(2)
            .i32_ne()
            .i32_and()
            .i32_and()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            // if anchored != 2 && anchored_pattern != 0 {
            .local_get(1)
            .i32_const(2)
            .i32_ne()
            .local_get(2)
            .i32_const(0)
            .i32_ne()
            .i32_and()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            // if span_start > span_end {
            .local_get(3)
            .local_get(4)
            .i64_gt_u()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            // if span_end >= haystack_len {
            .local_get(4)
            .local_get(5)
            .i64_gt_u()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            // if haystack_start_pos + haystack_len > memory.size * page_size
            .i64_const(i64::from_ne_bytes(
                u64::try_from(input_layout.haystack_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .local_get(5)
            .i64_add()
            .memory_size(0)
            .i64_const(i64::from_ne_bytes(
                u64::try_from(page_size).unwrap().to_ne_bytes(),
            ))
            .i64_mul()
            .i64_gt_u()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            .end();

        Function {
            sig: FunctionSignature {
                name: "assert_input_args_wf".into(),
                params_ty: &[
                    ValType::I32,
                    ValType::I32,
                    ValType::I32,
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                ],
                results_ty: &[],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }

    fn prepare_input_fn(page_size: usize, input_layout: &InputLayout) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_len");
        // Locals
        locals_name_map.append(1, "num_new_page_required");

        let mut body = wasm_encoder::Function::new([(1, ValType::I64)]);
        body.instructions()
            // if haystack_len == 0 {
            .local_get(0) // haystack_len
            .i64_const(i64::from_ne_bytes(
                u64::try_from(input_layout.haystack_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .i64_add()
            .i64_const(0)
            .i64_eq()
            .if_(BlockType::Empty)
            // return SuccessNoGrowth
            .i32_const(PrepareInputResult::SuccessNoGrowth as i32)
            .return_()
            .end()
            // memory_grow = ((haystack_len + haystack_start_pos - 1) / page_size) + 1 - memory_size
            .local_get(0) // haystack_len
            .i64_const(i64::from_ne_bytes(
                u64::try_from(input_layout.haystack_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .i64_add()
            .i64_const(1)
            .i64_sub()
            .i64_const(i64::from_ne_bytes(
                u64::try_from(page_size).unwrap().to_ne_bytes(),
            ))
            .i64_div_u()
            .i64_const(1)
            .i64_add()
            .memory_size(0)
            .i64_sub()
            .local_tee(1)
            .i64_const(0)
            // Use signed comparison: num_new_page_required > 0
            // otherwise negative values of num_new_page_required would register as very large
            // positive numbers
            .i64_gt_s()
            .if_(BlockType::Result(ValType::I32))
            .local_get(1)
            .memory_grow(0)
            .i64_const(-1)
            .i64_eq()
            .if_(BlockType::Empty)
            // If the memory.grow returns -1, then trap since I don't want to handle this
            .i32_const(PrepareInputResult::Failure as i32)
            .return_()
            .end()
            .i32_const(PrepareInputResult::SuccessGrowth as i32)
            .else_()
            .i32_const(PrepareInputResult::SuccessNoGrowth as i32)
            .end()
            .end();

        Function {
            sig: FunctionSignature {
                name: "prepare_input".into(),
                // [haystack_len]
                params_ty: &[ValType::I64],
                // [prepare_input_result]
                results_ty: &[ValType::I32],
                export: true,
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

/// This enum represents the results of the
/// [`prepare_input`][InputFunctions::prepare_input_fn] function.
#[derive(Debug)]
pub enum PrepareInputResult {
    /// Indicates that the input preparation was successful and no memory growth
    /// was needed.
    SuccessNoGrowth = 0,
    /// Indicates that the input preparation was successful and memory was grown
    /// to accommodate the haystack.
    SuccessGrowth = 1,
    /// Indicates that input preparation failed, likely due to an inability to
    /// grow memory.
    Failure = 2,
}

#[cfg(test)]
mod tests {
    use regex_automata::nfa::thompson::NFA;

    use crate::compile::{
        pattern::{PatternFunctions, PatternLayout},
        tests::setup_interpreter,
    };

    use super::*;

    #[test]
    fn prepare_input() {
        let mut ctx = CompileContext::new(
            NFA::always_match(),
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );

        let state_overall = Layout::new::<()>();
        let (state_overall, pattern_layout) = PatternLayout::new(&mut ctx, state_overall).unwrap();
        let pattern_functions = PatternFunctions::new(&mut ctx, &pattern_layout);

        let input_layout = InputLayout::new(&mut ctx).unwrap();
        let _input_functions =
            InputFunctions::new(&mut ctx, &input_layout, pattern_functions.lookup_start);
        let page_size = ctx.config.get_page_size();

        let module = ctx.compile(&state_overall);
        let module_bytes = module.finish();
        let (_engine, _module, mut store, instance) = setup_interpreter(&module_bytes);
        let haystack_memory = instance.get_memory(&store, "haystack").unwrap();
        let prepare_input = instance
            .get_typed_func::<i64, i32>(&store, "prepare_input")
            .unwrap();

        let haystack_size = haystack_memory.size(&store);
        assert_eq!(haystack_size, 1);

        let haystack_len = 0;
        let res = prepare_input.call(&mut store, haystack_len).unwrap();
        assert_eq!(res, PrepareInputResult::SuccessNoGrowth as i32);

        let haystack_size = haystack_memory.size(&store);
        assert_eq!(haystack_size, 1);

        let haystack_len = 1;
        let res = prepare_input.call(&mut store, haystack_len).unwrap();
        assert_eq!(res, PrepareInputResult::SuccessNoGrowth as i32);

        let haystack_size = haystack_memory.size(&store);
        assert_eq!(haystack_size, 1);

        // This haystack_len should fill the entire extent of the default-sized haystack
        // memory
        let haystack_len = i64::try_from(page_size - input_layout._overall.size()).unwrap();
        let res = prepare_input.call(&mut store, haystack_len).unwrap();
        assert_eq!(res, PrepareInputResult::SuccessNoGrowth as i32);

        let haystack_size = haystack_memory.size(&store);
        assert_eq!(haystack_size, 1);

        // This haystack_len should cause the haystack memory to increase by 1 page size
        let haystack_len =
            i64::try_from(page_size - input_layout._overall.size() + page_size).unwrap();
        let res = prepare_input.call(&mut store, haystack_len).unwrap();
        assert_eq!(res, PrepareInputResult::SuccessGrowth as i32);

        let haystack_size = haystack_memory.size(&store);
        assert_eq!(haystack_size, 2);

        // Test case: num_new_page_required is negative
        // At this point, memory has 2 pages.
        // We'll request a haystack_len that only requires 1 page.
        // input_layout.haystack_start_pos is 0.
        // So, total_bytes_needed = haystack_len_for_negative_case.
        // If haystack_len_for_negative_case = 1, then required_total_pages =
        // ceil_div(1, page_size) = 1. num_new_page_required =
        // required_total_pages (1) - current_pages (2) = -1. The function
        // should return SuccessNoGrowth and memory should remain at 2 pages.
        let haystack_len_for_negative_case = 1_i64; // Fits in 1 page
        let res = prepare_input
            .call(&mut store, haystack_len_for_negative_case)
            .unwrap();
        assert_eq!(
            res,
            PrepareInputResult::SuccessNoGrowth as i32,
            "Should be SuccessNoGrowth when current pages > required pages"
        );
        assert_eq!(
            haystack_memory.size(&store),
            2,
            "Memory size should remain 2 pages"
        );
    }
}
