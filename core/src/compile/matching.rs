//! This module contains types and functions related to the actual PikeVM
//! execution of `is_match`, `find`, `captures`, etc.

use wasm_encoder::{BlockType, NameMap, ValType};

use super::{
    context::{
        BlockSignature, CompileContext, Function, FunctionDefinition, FunctionIdx,
        FunctionSignature, TypeIdx,
    },
    input::{InputFunctions, InputLayout},
    state::{StateFunctions, StateLayout},
};

#[derive(Debug)]
pub struct MatchingFunctions {
    _is_match: FunctionIdx,
}

impl MatchingFunctions {
    pub fn new(
        ctx: &mut CompileContext,
        state_layout: &StateLayout,
        state_funcs: &StateFunctions,
        input_layout: &InputLayout,
        input_funcs: &InputFunctions,
    ) -> Self {
        let start_config_is_some_block_sig = ctx.add_block_signature(BlockSignature {
            name: "start_config_is_some",
            params_ty: &[ValType::I32, ValType::I32],
            results_ty: &[ValType::I32, ValType::I32],
        });

        let is_match_block_sig = ctx.add_block_signature(BlockSignature {
            name: "make_current_transitions_is_match",
            params_ty: &[ValType::I32],
            results_ty: &[],
        });

        let is_match = ctx.add_function(Self::is_match_fn(
            state_layout,
            state_funcs,
            input_layout,
            input_funcs,
            start_config_is_some_block_sig,
            is_match_block_sig,
        ));

        Self {
            _is_match: is_match,
        }
    }

    fn is_match_fn(
        state_layout: &StateLayout,
        state_funcs: &StateFunctions,
        input_layout: &InputLayout,
        input_funcs: &InputFunctions,
        start_config_is_some_block_sig: TypeIdx,
        is_match_block_sig: TypeIdx,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "anchored");
        locals_name_map.append(1, "anchored_pattern");
        locals_name_map.append(2, "span_start");
        locals_name_map.append(3, "span_end");
        locals_name_map.append(4, "haystack_len");
        // Locals
        locals_name_map.append(5, "at_offset");
        locals_name_map.append(6, "curr_set_ptr");
        locals_name_map.append(7, "next_set_ptr");
        locals_name_map.append(8, "curr_set_len");
        locals_name_map.append(9, "next_set_len");
        locals_name_map.append(10, "start_state_id");
        locals_name_map.append(11, "is_anchored");

        let mut labels_name_map = NameMap::new();
        labels_name_map.append(1, "haystack_search_loop");

        // Sketch:
        // ```
        // assert_input_args_wf(true, anchored, anchored_pattern, span_start, span_end, haystack_len)
        // (start_state_id, is_anchored, is_some) = start_config(anchored, anchored_pattern)
        // if !is_some {
        //     return false;
        // }
        //
        // curr_set_ptr = first_set_start_pos;
        // curr_set_len = 0;
        // next_set_ptr = second_set_start_pos;
        // next_set_len = 0;
        // at_offset = span_start;
        // loop {
        //     if at_offset > span_end {
        //         return false;
        //     }
        //
        //     if curr_set_len == 0 && is_anchored && at_offset > span_start {
        //         return false;
        //     }
        //
        //     if !is_anchored || at_offset == span_start {
        //         curr_set_len = branch_to_epsilon_closure(haystack_ptr, haystack_len, at_offset, curr_set_ptr, curr_set_len, start_state_id)
        //     }
        //
        //     new_next_set_len, is_match = make_current_transitions(haystack_ptr, haystack_len, at_offset, curr_set_ptr, curr_set_len, next_set_ptr, next_set_len)
        //     if is_match && utf8_is_boundary(haystack_ptr, haystack_len, at_offset) {
        //         return true;
        //     }
        //     curr_set_ptr, next_set_ptr = next_set_ptr, curr_set_ptr;
        //     curr_set_len, next_set_len = next_set_len, curr_set_len;
        //     next_set_len = 0;
        //     at = at + 1;
        // }
        // ```

        let mut body = wasm_encoder::Function::new([(3, ValType::I64), (4, ValType::I32)]);
        body.instructions()
            // assert_input_args_wf(true, anchored, anchored_pattern, span_start, span_end,
            // haystack_len)
            .i32_const(true as i32) // earliest
            .local_get(0) // anchored
            .local_get(1) // anchored_pattern
            .local_get(2) // span_start
            .local_get(3) // span_end
            .local_get(4) // haystack_len
            .call(input_funcs.assert_input_args_wf.into())
            // (start_state_id, is_anchored, is_some) = start_config(anchored, anchored_pattern)
            .local_get(0) // anchored
            .local_get(1) // anchored_pattern
            .call(input_funcs.start_config.into())
            // if !is_some {
            .i32_const(false as i32)
            .i32_eq()
            .if_(BlockType::FunctionType(
                start_config_is_some_block_sig.into(),
            ))
            // return false;
            .drop()
            .drop()
            .i32_const(false as i32)
            .return_()
            .end()
            .local_set(11) // is_anchored
            .local_set(10) // start_state_id
            // curr_set_ptr = first_set_start_pos;
            .i64_const(i64::from_ne_bytes(
                u64::try_from(state_layout.first_sparse_set.set_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .local_set(6) // curr_set_ptr
            // next_set_ptr = second_set_start_pos;
            .i64_const(i64::from_ne_bytes(
                u64::try_from(state_layout.second_sparse_set.set_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .local_set(7) // next_set_ptr
            // at_offset = span_start
            .local_get(2) // span_start
            .local_set(5) // at_offset
            // loop {
            .loop_(BlockType::Empty)
            // if at_offset > span_end {
            .local_get(5) // at_offset
            .local_get(3) // span_end
            .i64_gt_u()
            .if_(BlockType::Empty)
            // return false;
            .i32_const(false as i32)
            .return_()
            .end()
            // if curr_set_len == 0 && is_anchored && at_offset > span_start {
            .local_get(8) // curr_set_len
            .i32_const(0)
            .i32_eq()
            .local_get(11) // is_anchored
            .local_get(5) // at_offset
            .local_get(2) // span_start
            .i64_gt_u()
            .i32_and()
            .i32_and()
            .if_(BlockType::Empty)
            // return false;
            .i32_const(false as i32)
            .return_()
            .end()
            // if !is_anchored || at_offset == span_start {
            .local_get(11) // is_anchored
            .i32_const(false as i32)
            .i32_eq()
            .local_get(5) // at_offset
            .local_get(2) // span_start
            .i64_eq()
            .i32_or()
            .if_(BlockType::Empty)
            // curr_set_len = branch_to_epsilon_closure(haystack_ptr, haystack_len, at_offset,
            // curr_set_ptr, curr_set_len, start_state_id)
            .i64_const(i64::from_ne_bytes(
                u64::try_from(input_layout.haystack_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .local_get(4) // haystack_len
            .local_get(5) // at_offset
            .local_get(6) // curr_set_ptr
            .local_get(8) // curr_set_len
            .local_get(10) // start_state_id
            .call(state_funcs.epsilon_closure.branch_to_epsilon_closure.into())
            .local_set(8) // curr_set_len
            .end()
            // new_next_set_len, is_match = make_current_transitions(haystack_ptr, haystack_len,
            // at_offset, curr_set_ptr, curr_set_len, next_set_ptr, next_set_len)
            .i64_const(i64::from_ne_bytes(
                u64::try_from(input_layout.haystack_start_pos)
                    .unwrap()
                    .to_ne_bytes(),
            ))
            .local_get(4) // haystack_len
            .local_get(5) // at_offset
            .local_get(6) // curr_set_ptr
            .local_get(8) // curr_set_len
            .local_get(7) // next_set_ptr
            .local_get(9) // next_set_len
            .call(state_funcs.transition.make_current_transitions.into());

        // stack: [new_next_set_len, is_match]
        // if is_match && utf8_is_boundary(haystack_ptr, haystack_len, at_offset)

        // This should only be `Some` if the input NFA can match the empty string and
        // UTF-8 is enabled
        if let Some(utf8_is_boundary) = input_funcs.utf8_is_boundary {
            body.instructions()
                // utf8_is_boundary(haystack_ptr, haystack_len, at_offset)
                .i64_const(i64::from_ne_bytes(
                    u64::try_from(input_layout.haystack_start_pos)
                        .unwrap()
                        .to_ne_bytes(),
                ))
                .local_get(4) // haystack_len
                .local_get(5) // at_offset
                .call(utf8_is_boundary.into())
                .i32_and();
        }

        body.instructions()
            .if_(BlockType::FunctionType(is_match_block_sig.into()))
            .drop()
            .i32_const(true as i32)
            .return_()
            .else_()
            // next_set_len = new_next_set_len;
            .local_set(9) // next_set_len
            .end()
            // curr_set_ptr, next_set_ptr = next_set_ptr, curr_set_ptr;
            .local_get(6) // curr_set_ptr
            .local_get(7) // next_set_ptr
            .local_set(6)
            .local_set(7)
            // curr_set_len, next_set_len = next_set_len, curr_set_len;
            .local_get(8) // curr_set_len
            .local_get(9) // next_set_len
            .local_set(8)
            .local_set(9)
            // next_set_len = 0;
            .i32_const(0)
            .local_set(9)
            // at = at + 1;
            .local_get(5) // at_offset
            .i64_const(1)
            .i64_add()
            .local_set(5) // at_offset
            .br(0) // continue loop
            .end()
            // } end loop
            .i32_const(false as i32)
            .end();

        Function {
            sig: FunctionSignature {
                name: "is_match".into(),
                // [anchored, anchored_pattern, span_start, span_end, haystack_len]
                params_ty: &[
                    ValType::I32,
                    ValType::I32,
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                ],
                // [is_match]
                results_ty: &[ValType::I32],
                export: true,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }
}
