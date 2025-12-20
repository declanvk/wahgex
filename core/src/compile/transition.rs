//! This module contains type and functions related to the NFA transition
//! function.

use std::{
    alloc::{Layout, LayoutError},
    collections::{BTreeMap, HashMap},
    mem,
};

use regex_automata::{
    nfa::thompson::{DenseTransitions, NFA, SparseTransitions, State, Transition},
    util::primitives::StateID,
};
use wasm_encoder::{BlockType, InstructionSink, MemArg, NameMap, ValType};

use crate::compile::context::FunctionTypeSignature;

use super::{
    CompileContext,
    context::{
        ActiveDataSegment, BlockSignature, Function, FunctionDefinition, FunctionIdx,
        FunctionSignature, TypeIdx,
    },
    epsilon_closure::EpsilonClosureFunctions,
    instructions::InstructionSinkExt,
    util::repeat,
};

const SPARSE_RANGE_LOOKUP_TABLE_ELEM: Layout = const {
    match Layout::array::<u8>(2) {
        Ok(val) => val,
        Err(_) => panic!("invalid layout"),
    }
};

/// This struct contains the layout for the lookup tables used by the
/// [`TransitionFunctions`].
///
/// Each transition function may have a differently shaped lookup table.
///  - For [`Transition`]s, there is no lookup table and we just embed the
///    `start` and `end` directly into the function.
///  - For [`SparseTransitions`]s, the table is represented as 2 arrays. The
///    first array the `start` `end` tuples from the [`Transition`]s. The second
///    array contains the [`StateID`]s arranged to match the same order as the
///    tuples.
///  - For [`DenseTransitions`], it will be an array of length 256 containing
///    [`StateID`]s.
#[derive(Debug)]
pub struct TransitionLayout {
    lookup_tables: HashMap<StateID, LookupTable>,
}

/// This enum represents the different type of lookup tables and their offsets.
///
/// See [`TransitionLayout`] for more details.
#[derive(Debug, Clone, Copy)]
enum LookupTable {
    Sparse(SparseTable),
    Dense(DenseTable),
}

impl LookupTable {
    fn unwrap_sparse(self) -> SparseTable {
        match self {
            LookupTable::Sparse(table) => table,
            _ => panic!("not a sparse table offset"),
        }
    }

    fn unwrap_dense(self) -> DenseTable {
        match self {
            LookupTable::Dense(table) => table,
            _ => panic!("not a dense table offset"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SparseTable {
    range_table_pos: usize,
    range_table_len: usize,
    range_lookup_table_stride: usize,
    state_id_table_pos: usize,
    state_id_table_stride: usize,
}

#[derive(Debug, Clone, Copy)]
struct DenseTable {
    table_pos: usize,
    table_stride: usize,
}

impl TransitionLayout {
    /// Creates a new `TransitionLayout` by calculating the memory offsets for
    /// transition lookup tables.
    pub fn new(
        ctx: &mut CompileContext,
        mut overall: Layout,
    ) -> Result<(Layout, Self), LayoutError> {
        let mut lookup_table_offsets = HashMap::new();

        let states = ctx.nfa.states();
        let state_id_layout = *ctx.state_id_layout();
        for for_sid in (0..states.len()).map(StateID::new).map(Result::unwrap) {
            let state = &states[for_sid.as_usize()];
            match state {
                State::ByteRange { .. } => {
                    // no lookup table
                },
                State::Sparse(SparseTransitions { transitions }) => {
                    let (range_data, state_data) =
                        flatten_sparse_transition(transitions, &state_id_layout);

                    // (start, end) tuples arranged together
                    let (range_lookup_table, range_lookup_table_stride) =
                        repeat(&SPARSE_RANGE_LOOKUP_TABLE_ELEM, transitions.len())?;
                    let (new_overall, range_table_pos) = overall.extend(range_lookup_table)?;
                    overall = new_overall;

                    assert_eq!(
                        range_lookup_table.size(),
                        range_data.len(),
                        "Segment data length must match layout size"
                    );

                    ctx.sections.add_active_data_segment(ActiveDataSegment {
                        name: format!("sparse_range_table_{}", for_sid.as_u32()),
                        position: range_table_pos,
                        data: range_data,
                    });

                    // state IDs all packed together
                    let (state_id_table, state_id_table_stride) =
                        repeat(&state_id_layout, transitions.len())?;
                    let (new_overall, state_id_table_pos) = overall.extend(state_id_table)?;
                    overall = new_overall;

                    assert_eq!(
                        state_id_table.size(),
                        state_data.len(),
                        "Segment data length must match layout size"
                    );

                    ctx.sections.add_active_data_segment(ActiveDataSegment {
                        name: format!("sparse_state_id_table_{}", for_sid.as_u32()),
                        position: state_id_table_pos,
                        data: state_data,
                    });

                    lookup_table_offsets.insert(
                        for_sid,
                        LookupTable::Sparse(SparseTable {
                            range_table_pos,
                            range_lookup_table_stride,
                            range_table_len: transitions.len(),
                            state_id_table_pos,
                            state_id_table_stride,
                        }),
                    );
                },
                State::Dense(DenseTransitions { transitions }) => {
                    let (lookup_table_layout, table_stride) = repeat(&state_id_layout, 256)?;
                    let (new_overall, table_pos) = overall.extend(lookup_table_layout)?;
                    overall = new_overall;
                    lookup_table_offsets.insert(
                        for_sid,
                        LookupTable::Dense(DenseTable {
                            table_pos,
                            table_stride,
                        }),
                    );

                    let data = flatten_dense_transition(transitions, &state_id_layout);

                    assert_eq!(
                        lookup_table_layout.size(),
                        data.len(),
                        "Segment data length must match layout size"
                    );

                    ctx.sections.add_active_data_segment(ActiveDataSegment {
                        name: format!("dense_table_{}", for_sid.as_u32()),
                        position: table_pos,
                        data,
                    });
                },
                _ => {
                    // no lookup table
                },
            }
        }

        Ok((
            overall,
            Self {
                lookup_tables: lookup_table_offsets,
            },
        ))
    }

    fn get(&self, sid: StateID) -> Option<LookupTable> {
        self.lookup_tables.get(&sid).copied()
    }
}

fn flatten_sparse_transition(
    sparse: &[Transition],
    state_id_layout: &Layout,
) -> (Vec<u8>, Vec<u8>) {
    let mut range_output = Vec::with_capacity(mem::size_of::<u8>() * sparse.len() * 2);

    for transition in sparse {
        range_output.push(transition.start);
        range_output.push(transition.end);
    }

    let mut state_output = Vec::with_capacity(state_id_layout.size() * sparse.len());

    for transition in sparse {
        // WASM assumes little endian byte ordering: https://webassembly.org/docs/portability/
        let bytes = transition.next.as_u32().to_le_bytes();
        state_output.extend_from_slice(&bytes[..state_id_layout.size()]);
    }

    (range_output, state_output)
}

fn flatten_dense_transition(dense: &[StateID], state_id_layout: &Layout) -> Vec<u8> {
    assert_eq!(dense.len(), 256);

    let mut output = Vec::with_capacity(state_id_layout.size() * dense.len());

    for state_id in dense {
        // WASM assumes little endian byte ordering: https://webassembly.org/docs/portability/
        output.extend_from_slice(&state_id.as_u32().to_le_bytes());
    }

    output
}

/// This struct contains a map of functions that are the transitions
/// for each NFA state.
///
/// This corresponds to the `next` function in
/// [`PikeVM`][regex_automata::nfa::thompson::pikevm::PikeVM].
#[derive(Debug)]
pub struct TransitionFunctions {
    #[expect(dead_code)]
    state_transitions: BTreeMap<StateID, FunctionIdx>,
    #[expect(dead_code)]
    branch_to_transition: FunctionIdx,
    pub make_current_transitions: FunctionIdx,
}

impl TransitionFunctions {
    /// Creates and registers all WebAssembly functions required for handling
    /// NFA state transitions.
    pub fn new(
        ctx: &mut CompileContext,
        epsilon_closures: &EpsilonClosureFunctions,
        transition_layout: &TransitionLayout,
    ) -> Self {
        // NOTE: The indexes of the `states` array correspond to the `StateID` value.
        let mut state_transitions = BTreeMap::new();

        let transition_fn_type = ctx.declare_fn_type(&FunctionTypeSignature {
            name: "transition",
            // [haystack_ptr, haystack_len, at_offset, next_set_ptr, next_set_len]
            params_ty: &[
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
            ],
            // [new_next_set_len, is_match]
            results_ty: &[ValType::I32, ValType::I32],
        });

        let num_states = ctx.nfa.states().len();
        for for_sid in (0..num_states).map(StateID::new).map(Result::unwrap) {
            if !Self::needs_transition_fn(&ctx.nfa, for_sid) {
                continue;
            }

            let transition_fn_def = Self::transition_fn(
                for_sid,
                ctx.nfa.states(),
                epsilon_closures.branch_to_epsilon_closure,
                transition_layout.get(for_sid),
                ctx.state_id_layout(),
            );
            let transition_idx = ctx.declare_function_with_type(
                transition_fn_type,
                &format!("transition_s{}", for_sid.as_usize()),
                false,
            );
            ctx.define_function(transition_idx, transition_fn_def);
            state_transitions.insert(for_sid, transition_idx);
        }

        let branch_to_transition = if !state_transitions.is_empty() {
            ctx.add_function(Self::branch_to_transition_fn(
                StateID::try_from(num_states - 1).expect("should be expressible as state ID"),
                &state_transitions,
            ))
        } else {
            ctx.add_function(Self::empty_branch_to_transition_fn())
        };

        let branch_to_transition_is_match_block_sig = ctx.add_block_signature(BlockSignature {
            name: "branch_to_transition_is_match",
            params_ty: &[ValType::I32],
            results_ty: &[],
        });

        let make_current_transitions = ctx.add_function(Self::make_current_transitions_fn(
            branch_to_transition,
            branch_to_transition_is_match_block_sig,
            ctx.state_id_layout(),
        ));

        Self {
            state_transitions,
            branch_to_transition,
            make_current_transitions,
        }
    }

    fn make_current_transitions_fn(
        branch_to_transition: FunctionIdx,
        branch_to_transition_is_match_block_sig: TypeIdx,
        state_id_layout: &Layout,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "current_set_ptr");
        locals_name_map.append(4, "current_set_len");
        locals_name_map.append(5, "next_set_ptr");
        locals_name_map.append(6, "next_set_len");
        // Locals
        locals_name_map.append(7, "loop_index");
        locals_name_map.append(8, "state_id");
        locals_name_map.append(9, "new_next_set_len");

        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "set_iter_loop");

        let mut body = wasm_encoder::Function::new([(3, ValType::I32)]);
        let mut instructions = body.instructions();

        // loop_index = 0 // local 7
        // new_next_set_len = next_set_len // local 9
        // loop {
        //     if loop_index >= current_set_len {
        //         return (new_next_set_len, false);
        //     }
        //
        //     state_id = current_set_ptr.dense[loop_index]; // local 8
        //     is_match, new_next_set_len = branch_to_transition(
        //         haystack_ptr,
        //         haystack_len,
        //         at_offset,
        //         next_set_ptr,
        //         new_next_set_len,
        //         state_id
        //     );
        //     if is_match {
        //         return (new_next_set_len, true);
        //     }
        //
        //     loop_index = loop_index + 1;
        // }
        // return (next_set_len, false); // just in case

        instructions
            // loop_index = 0 // local 7
            .i32_const(0)
            .local_set(7)
            // new_next_set_len = next_set_len // local 9
            .local_get(6)
            .local_set(9)
            .loop_(BlockType::Empty)
            // if loop_index >= current_set_len {
            .local_get(7)
            .local_get(4)
            .i32_ge_u()
            .if_(BlockType::Empty)
            // return (new_next_set_len, false);
            .local_get(9)
            .bool_const(false)
            .return_()
            .end()
            // state_id = current_set_ptr.dense[loop_index]; // local 8
            .local_get(7)
            .i64_extend_i32_u()
            .u64_const(u64::try_from(state_id_layout.align()).unwrap())
            .i64_mul()
            .local_get(3)
            .i64_add()
            .state_id_load(0, state_id_layout)
            .local_set(8)
            // is_match, new_next_set_len = branch_to_transition(..)
            .local_get(0)
            .local_get(1)
            .local_get(2)
            .local_get(5)
            .local_get(9)
            .local_get(8)
            .call(branch_to_transition.into())
            // if is_match {
            .if_(BlockType::FunctionType(
                branch_to_transition_is_match_block_sig.into(),
            ))
            // return (new_next_set_len, true);
            .bool_const(true)
            .return_()
            .else_()
            .local_set(9) // need to update new_next_set_len on non-match
            .end()
            // loop_index = loop_index + 1;
            .local_get(7)
            .i32_const(1)
            .i32_add()
            .local_set(7)
            .br(0) // continue loop
            .end() // end loop
            // return (new_next_set_len, false);
            .local_get(9)
            .bool_const(false)
            .end();

        Function {
            sig: FunctionSignature {
                name: "make_current_transitions".into(),
                // [haystack_ptr, haystack_len, at_offset, current_set_ptr, current_set_len,
                // next_set_ptr, next_set_len]
                params_ty: &[
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I32,
                    ValType::I64,
                    ValType::I32,
                ],
                // current_set is not modified by this function, so we don't return a new length
                // [new_next_set_len, is_match]
                results_ty: &[ValType::I32, ValType::I32],
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

    fn empty_branch_to_transition_fn() -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "next_set_ptr");
        locals_name_map.append(4, "next_set_len");
        locals_name_map.append(5, "state_id");

        // Rust sketch:
        // ```rust
        // return [new_next_set_len, is_match]
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions().local_get(4).bool_const(false).end();

        Function {
            sig: FunctionSignature {
                name: "branch_to_transition".into(),
                // [haystack_ptr, haystack_len, at_offset, next_set_ptr, next_set_len, state_id]
                params_ty: &[
                    // TODO(opt): Remove haystack_ptr and assume that haystack always starts at
                    // offset 0 in memory 0
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I32,
                    ValType::I32,
                ],
                // [new_next_set_len, is_match]
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

    fn branch_to_transition_fn(
        high: StateID,
        state_transitions: &BTreeMap<StateID, FunctionIdx>,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "next_set_ptr");
        locals_name_map.append(4, "next_set_len");
        locals_name_map.append(5, "state_id");

        let mut labels_name_map = NameMap::new();

        // Rust sketch:
        // ```rust
        // switch state_id {
        //     s0 => transition_s0(...),
        //     s1 => transition_s1(...),
        //     s2 => transition_s2(...),
        //     ...
        //     other => [new_next_set_len, is_match]
        // }
        // ```
        //
        // Switch statement expands to a bunch of blocks like:
        // ```wasm
        // (block
        //     (block
        //         (block ...)))
        // ```

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        debug_assert_eq!(
            StateID::ZERO.as_u32(),
            0,
            "Need to have this representation so that we don't need to manipulate state ID value \
             in WASM to adjust"
        );
        debug_assert!(
            StateID::try_from(high.as_u32() + 1).is_ok(),
            "Need to make sure the +1 is still in range"
        );

        let state_ids: Vec<_> = (StateID::ZERO.as_u32()..=high.as_u32())
            .map(|s| {
                StateID::try_from(s)
                    .expect("all state IDs between two valid state IDs must be representable")
            })
            .collect();

        instructions.block(BlockType::Empty); // start fallback block
        labels_name_map.append(0, "fallback_block");

        let mut state_id_to_block = HashMap::new();
        for state_id in state_ids.iter() {
            if state_transitions.contains_key(state_id) {
                instructions.block(BlockType::Empty);
                state_id_to_block.insert(
                    *state_id,
                    u32::try_from(state_id_to_block.len())
                        .expect("counter in state ID space should fit in u32"),
                );
            }
        }
        let fallback_block_label =
            u32::try_from(state_id_to_block.len() + 1).expect("number of states should fit in u32");
        let labels: Vec<_> = state_ids
            .iter()
            .map(|s| {
                if let Some(label) = state_id_to_block.get(s) {
                    *label
                } else {
                    fallback_block_label
                }
            })
            .collect();
        instructions
            .block(BlockType::Empty)
            .local_get(5)
            .br_table(labels, fallback_block_label)
            .end();
        for state_id in &state_ids {
            let Some(transition_fn) = state_transitions.get(state_id).copied() else {
                continue;
            };
            instructions
                .local_get(0)
                .local_get(1)
                .local_get(2)
                .local_get(3)
                .local_get(4)
                .call(transition_fn.into())
                .return_()
                .end();
        }

        instructions
            .end() // end fallback block
            //     return [new_next_set_len, is_match]
            .local_get(4)
            .bool_const(false)
            .end();

        Function {
            sig: FunctionSignature {
                name: "branch_to_transition".into(),
                // [haystack_ptr, haystack_len, at_offset, next_set_ptr, next_set_len, state_id]
                params_ty: &[
                    // TODO(opt): Remove haystack_ptr and assume that haystack always starts at
                    // offset 0 in memory 0
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I32,
                    ValType::I32,
                ],
                // [new_next_set_len, is_match]
                results_ty: &[ValType::I32, ValType::I32],
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

    fn transition_fn(
        for_sid: StateID,
        states: &[State],
        branch_to_epsilon_closure: FunctionIdx,
        lookup_table: Option<LookupTable>,
        state_id_layout: &Layout,
    ) -> FunctionDefinition {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "next_set_ptr");
        locals_name_map.append(4, "next_set_len");
        // Locals added in transition_fn_need_locals

        fn transition_fn_need_locals(
            for_sid: StateID,
            state: &State,
            locals_name_map: &mut NameMap,
        ) -> Vec<(u32, ValType)> {
            match state {
                State::Fail
                | State::Look { .. }
                | State::Union { .. }
                | State::BinaryUnion { .. }
                | State::Capture { .. } => {
                    // return None
                    unreachable!(
                        "We should never generate transitions for state [{for_sid:?}] since \
                         they're excluded by `needs_transition_fn`."
                    );
                },
                State::ByteRange { .. } | State::Sparse { .. } | State::Dense { .. } => {
                    let mut num_i32s = 2;

                    locals_name_map.append(5, "byte");
                    locals_name_map.append(6, "next_state");

                    if matches!(state, State::Sparse { .. } | State::Dense { .. }) {
                        num_i32s += 1;
                        locals_name_map.append(7, "loop_index");

                        if matches!(state, State::Sparse { .. }) {
                            num_i32s += 2;
                            locals_name_map.append(8, "transition_start");
                            locals_name_map.append(9, "transition_end");
                        }
                    }

                    vec![(num_i32s, ValType::I32)]
                },
                State::Match { .. } => vec![],
            }
        }

        let mut labels_name_map = NameMap::new();

        let mut body = wasm_encoder::Function::new(transition_fn_need_locals(
            for_sid,
            &states[for_sid.as_usize()],
            &mut locals_name_map,
        ));
        let mut instructions = body.instructions();
        match &states[for_sid.as_usize()] {
            State::Fail
            | State::Look { .. }
            | State::Union { .. }
            | State::BinaryUnion { .. }
            | State::Capture { .. } => {
                // return None
                unreachable!(
                    "We should never generate transitions for state [{for_sid:?}] since they're \
                     excluded by `needs_transition_fn`."
                );
            },
            State::ByteRange { trans } => {
                Self::non_terminal_transition_prefix(&mut instructions);
                Self::byte_range_transition_body(&mut instructions, trans);
                Self::non_terminal_transition_suffix(&mut instructions, branch_to_epsilon_closure);
            },
            State::Sparse(_) => {
                // We don't need the transition data here, since we've already emitted the
                // lookup tables
                let sparse_table = lookup_table.unwrap().unwrap_sparse();
                Self::non_terminal_transition_prefix(&mut instructions);
                Self::sparse_transition_body(
                    &mut instructions,
                    sparse_table,
                    &mut labels_name_map,
                    state_id_layout,
                );
                Self::non_terminal_transition_suffix(&mut instructions, branch_to_epsilon_closure);
            },
            State::Dense(_) => {
                // We don't need the transition data here, since we've already emitted the
                // lookup tables
                let dense_table = lookup_table.unwrap().unwrap_dense();
                Self::non_terminal_transition_prefix(&mut instructions);
                Self::dense_transition_body(&mut instructions, dense_table, state_id_layout);
                Self::non_terminal_transition_suffix(&mut instructions, branch_to_epsilon_closure);
            },
            State::Match { .. } => {
                // TODO: Need to update for pattern matches
                // return Some(...)
                instructions.local_get(4).bool_const(true);
            },
        }
        instructions.end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: Some(labels_name_map),
            branch_hints: None,
        }
    }

    /// Return true if the given state needs a transition function.
    fn needs_transition_fn(nfa: &NFA, sid: StateID) -> bool {
        matches!(
            nfa.state(sid),
            State::ByteRange { .. }
                | State::Sparse { .. }
                | State::Dense { .. }
                | State::Match { .. }
        )
    }

    fn non_terminal_transition_prefix(instructions: &mut InstructionSink<'_>) {
        instructions // check haystack length load haystack byte
            // if at_offset >= haystack_len
            .local_get(2) // at_offset
            .local_get(1) // haystack_len
            .i64_ge_u()
            .if_(BlockType::Empty)
            // return None
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            .end() // end if at_offset >= haystack_len
            // TODO(opt): We can make haystack_ptr a constant if we thread through the input layout
            // and expose it as a field instead. Since we always expect to place the input haystack
            // in the same spot, while the haystack_len may vary
            .local_get(0) // haystack_ptr
            .local_get(2) // at_offset
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            })
            .local_set(5); // byte
    }

    fn non_terminal_transition_suffix(
        instructions: &mut InstructionSink<'_>,
        branch_to_epsilon_closure: FunctionIdx,
    ) {
        instructions
            // let at = at.wrapping_add(1);
            .local_get(2) // at_offset
            .i64_const(1)
            .i64_add()
            .local_set(2) // at_offset
            // self.epsilon_closure(
            //     stack, slots, next, input, at, trans.next,
            // );
            .local_get(0) // haystack_ptr
            .local_get(1) // haystack_len
            .local_get(2) // at_offset
            .local_get(3) // next_set_ptr
            .local_get(4) // next_set_len
            .local_get(6) // next_state
            // TODO(opt): Instead of calling indirectly to find get the right epsilon closure
            // function, we could pipe through what the expected next state is and branch directly
            // to that epsilon closure function if present (or add next state to the set if not
            // present)
            .call(branch_to_epsilon_closure.into()) // returns new_next_set_len
            // return None
            .bool_const(false);
    }

    fn sparse_transition_body(
        instructions: &mut InstructionSink<'_>,
        sparse_table: SparseTable,
        labels_name_map: &mut NameMap,
        state_id_layout: &Layout,
    ) {
        // Range table is laid out as `[(start: u8, end: u8), ...]`
        // Need to iterate and find the index where the `start` <= byte && byte
        // <= end`.
        //
        // State table is just a flat array of state IDs like
        // `[state ID, ...]` If we find a match in the range table, the
        // corresponding index in the state table is the next transition
        // state.

        labels_name_map.append(1, "table_break_block");
        labels_name_map.append(2, "table_iter_loop");

        // loop_index = 0 // local 7
        // loop {
        //     if loop_index >= sparse_table.range_table_len {
        //         return None;
        //     }
        //
        //     start = range_table[loop_index].0 // local 8
        //     if start > byte {
        //         return None;
        //     } else { // start <= byte
        //         end = range_table[loop_index].1
        //         if byte <= end {
        //             next_state = state_table[loop_index]
        //             break;
        //         }
        //     }
        //     loop_index = loop_index + 1
        // }
        // ... continue to epsilon

        instructions
            .i32_const(0)
            .local_set(7) // loop_index
            // This block is needed so that we can break out of the loop
            .block(BlockType::Empty)
            .loop_(BlockType::Empty)
            // if loop_index >= sparse_table.range_table_len {
            .local_get(7) // loop_index
            .u32_const(
                u32::try_from(sparse_table.range_table_len)
                    .expect("table length should fit within u32"),
            )
            .i32_ge_u()
            .if_(BlockType::Empty)
            // return None
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            .end() // end if loop_index >= sparse_table.range_table_len {
            // start = range_table[loop_index].0
            .local_get(7) // loop_index
            .i64_extend_i32_u()
            .u64_const(u64::try_from(sparse_table.range_lookup_table_stride).unwrap())
            .i64_mul()
            .i32_load8_u(MemArg {
                offset: u64::try_from(sparse_table.range_table_pos).unwrap(), // start is at offset 0
                align: 0,
                memory_index: 1,
            })
            .local_tee(8) // transition_start
            // if start > byte {
            .local_get(5) // byte
            .i32_gt_u()
            .if_(BlockType::Empty)
            // return None
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            // } else { // start <= byte
            .else_()
            // end = range_table[loop_index].1
            .local_get(7) // loop_index
            .i64_extend_i32_u()
            .u64_const(u64::try_from(sparse_table.range_lookup_table_stride).unwrap())
            .i64_mul()
            .i32_load8_u(MemArg {
                offset: u64::try_from(sparse_table.range_table_pos).unwrap() + 1, // end is at offset 1
                align: 0,
                memory_index: 1,
            })
            .local_set(9) // transition_end
            // if byte <= end {
            .local_get(5) // byte
            .local_get(9) // transition_end
            .i32_le_u()
            .if_(BlockType::Empty)
            // next_state = state_table[loop_index]
            .local_get(7) // loop_index
            .i64_extend_i32_u()
            .u64_const(u64::try_from(sparse_table.state_id_table_stride).unwrap())
            .i64_mul()
            .state_id_load(
                u64::try_from(sparse_table.state_id_table_pos).unwrap(),
                state_id_layout,
            )
            .local_set(6) // next_state
            // break;
            // jump to the end of the block outside of loop
            // Depth: 0=inner if, 1=outer if, 2=`loop`, 3=enclosing `block`
            .br(3)
            .end() // end if byte <= end {
            .end() // end } else { // start <= byte
            // loop_index = loop_index + 1
            .local_get(7) // loop_index
            .i32_const(1)
            .i32_add()
            .local_set(7) // loop_index
            .br(0)
            .end() // end loop
            .end(); // end block
    }

    fn dense_transition_body(
        instructions: &mut InstructionSink<'_>,
        table: DenseTable,
        state_id_layout: &Layout,
    ) {
        // Dense transition table is laid out as a 256-length array of state
        // IDs. To lookup the next state, just use the byte as an index. If the
        // state is non-zero, then the transition is present.

        instructions
            .local_get(5) // byte
            .i64_extend_i32_u()
            .u64_const(u64::try_from(table.table_stride).unwrap())
            .i64_mul() // offset in table
            .state_id_load(u64::try_from(table.table_pos).unwrap(), state_id_layout)
            .local_tee(6) // next_state
            // if next == StateID::ZERO
            .i32_eqz()
            .if_(BlockType::Empty)
            // return None
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            .end();
    }

    fn byte_range_transition_body(instructions: &mut InstructionSink<'_>, trans: &Transition) {
        instructions
            // self.start <= byte
            .i32_const(trans.start.into()) // self.start
            .local_get(5) // byte
            // we invert the condition here, since we're testing the failure
            .i32_gt_u() // >
            .if_(BlockType::Empty)
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            .end()
            // byte <= self.end
            .local_get(5) // byte
            .i32_const(trans.end.into()) // self.end
            // we invert the condition here, since we're testing the failure
            .i32_gt_u() // >
            .if_(BlockType::Empty)
            .local_get(4) // next_set_len
            .bool_const(false)
            .return_()
            .end()
            .u32_const(trans.next.as_u32())
            .local_set(6); // next_state
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        RegexBytecode,
        compile::{
            lookaround::{LookFunctions, LookLayout},
            sparse_set::{SparseSetFunctions, SparseSetLayout, tests::get_sparse_set_fns},
        },
    };

    use super::*;

    fn branch_to_transition_test_closure(
        nfa: NFA,
        haystack: &[u8],
    ) -> impl FnMut(i32, usize, &[u8], bool) + '_ {
        let mut ctx = CompileContext::new(
            nfa,
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );

        // We're going to assume all states use less then u8::MAX states
        assert_eq!(*ctx.state_id_layout(), Layout::new::<u8>());

        let overall = Layout::new::<()>();
        let (overall, sparse_set_layout) = SparseSetLayout::new(&mut ctx, overall).unwrap();
        let sparse_set_functions = SparseSetFunctions::new(&mut ctx, &sparse_set_layout);
        let (overall, look_layout) = LookLayout::new(&mut ctx, overall).unwrap();
        let look_funcs = LookFunctions::new(&mut ctx, &look_layout);
        let epsilon_closures =
            EpsilonClosureFunctions::new(&mut ctx, sparse_set_functions.insert, &look_funcs)
                .unwrap();
        let (overall, transition_layout) = TransitionLayout::new(&mut ctx, overall).unwrap();
        let _transition_functions =
            TransitionFunctions::new(&mut ctx, &epsilon_closures, &transition_layout);

        let module_bytes = ctx.compile(&overall).finish();
        let module_bytes = RegexBytecode::from_bytes_unchecked(module_bytes);
        let mut regex =
            crate::engines::wasmi::Executor::with_engine(::wasmi::Engine::default(), &module_bytes)
                .unwrap();

        let branch_to_transition = regex
            .instance()
            .get_typed_func::<(i64, i64, i64, i64, i32, i32), (i32, i32)>(
                regex.store(),
                "branch_to_transition",
            )
            .unwrap();

        let haystack_memory = regex
            .instance()
            .get_memory(regex.store(), "haystack")
            .unwrap();
        let state_memory = regex.instance().get_memory(regex.store(), "state").unwrap();

        // Write haystack byte into memory ahead of transition call
        haystack_memory.data_mut(regex.store_mut())[0..haystack.len()].copy_from_slice(haystack);

        move |state_id: i32,
              at_offset: usize,
              expected_next_states: &[u8],
              expected_is_match: bool| {
            let haystack_ptr = 0;
            let haystack_len = haystack.len() as i64;
            let set_ptr = 0;
            let set_len = 0;

            let (new_set_len, is_match) = branch_to_transition
                .call(
                    regex.store_mut(),
                    (
                        haystack_ptr,
                        haystack_len,
                        at_offset as i64,
                        set_ptr,
                        set_len,
                        state_id,
                    ),
                )
                .unwrap();

            let byte = haystack.get(at_offset).copied().unwrap_or(u8::MAX);

            assert_eq!(
                is_match, expected_is_match as i32,
                "{state_id} @ {at_offset} => {byte}/{}",
                byte as char
            );

            let states = &unsafe { state_memory.data(regex.store()).align_to::<u8>().1 }
                [0..usize::try_from(new_set_len).unwrap()];
            assert_eq!(
                states, expected_next_states,
                "{state_id} @ {at_offset} => {byte}/{}",
                byte as char
            );
        }
    }

    #[test]
    fn branch_to_normal_transition() {
        // thompson::NFA(
        //     >000000: binary-union(2, 1)
        //      000001: \x00-\xFF => 0
        //     ^000002: capture(pid=0, group=0, slot=0) => 3
        //      000003: a => 4
        //      000004: b => 5
        //      000005: c => 6
        //      000006: binary-union(3, 7)
        //      000007: capture(pid=0, group=0, slot=1) => 8
        //      000008: MATCH(0)
        let nfa = NFA::new("(?:abc)+").unwrap();

        let mut test = branch_to_transition_test_closure(nfa, b"abc");

        // State 0:
        test(0, 0, &[], false);
        test(0, 1, &[], false);
        test(0, 2, &[], false);

        // State 1: \x00-\xFF => 0
        test(1, 0, &[0, 1, 2, 3], false);
        test(1, 1, &[0, 1, 2, 3], false);
        test(1, 2, &[0, 1, 2, 3], false);

        // State 2: capture(pid=0, group=0, slot=0) => 3
        test(2, 0, &[], false);
        test(2, 1, &[], false);
        test(2, 2, &[], false);

        // State 3: a => 4
        test(3, 0, &[4], false);
        test(3, 1, &[], false);
        test(3, 2, &[], false);

        // State 4: b => 5
        test(4, 0, &[], false);
        test(4, 1, &[5], false);
        test(4, 2, &[], false);

        // State 5: c => 6 + epsilon transitions
        test(5, 0, &[], false);
        test(5, 1, &[], false);
        test(5, 2, &[3, 6, 7, 8], false);

        // State 6: binary-union(3, 7)
        test(6, 0, &[], false);
        test(6, 1, &[], false);
        test(6, 2, &[], false);

        // State 7: capture(pid=0, group=0, slot=1) => 8
        test(7, 0, &[], false);
        test(7, 1, &[], false);
        test(7, 2, &[], false);

        // State 8: MATCH(0)
        test(8, 0, &[], true);
    }

    #[test]
    fn branch_to_sparse_transition() {
        // thompson::NFA(
        //     >000000: binary-union(2, 1)
        //      000001: \x00-\xFF => 0
        //     ^000002: capture(pid=0, group=0, slot=0) => 6
        //      000003: c => 7
        //      000004: c => 7
        //      000005: c => 7
        //      000006: sparse(a => 3, b => 4, d => 5, e => 7, g => 7)
        //      000007: capture(pid=0, group=0, slot=1) => 8
        //      000008: MATCH(0)
        let nfa = NFA::new("ac|bc|dc|e|g").unwrap();

        let mut test = branch_to_transition_test_closure(nfa, b"acbcdceg");

        // State 0: binary-union(2, 1)
        for offset in [0, 2, 4, 6, 7] {
            test(0, offset, &[], false);
        }

        // State 1: \x00-\xFF => 0
        for offset in [0, 2, 4, 6, 7] {
            test(1, offset, &[0, 1, 2, 6], false);
        }

        // State 2: capture(pid=0, group=0, slot=0) => 8
        for offset in [0, 2, 4, 6, 7] {
            test(2, offset, &[], false);
        }

        for state in [3, 4, 5] {
            // State 3/4/5: c => 7
            test(state, 0, &[], false);
            test(state, 2, &[], false);
            test(state, 1, &[7, 8], false);
        }

        // State 6: sparse(a => 3, b => 4, d => 5, e => 7, g => 7)
        test(6, 0, &[3], false);
        test(6, 2, &[4], false);
        test(6, 4, &[5], false);
        test(6, 6, &[7, 8], false);
        test(6, 7, &[7, 8], false);

        // State 7: capture(pid=0, group=0, slot=1) => 8
        for offset in [0, 2, 4, 6, 7] {
            test(7, offset, &[], false);
        }

        // State 8: MATCH(0)
        for offset in [0, 2, 4, 6, 7] {
            test(8, offset, &[], true);
        }
    }

    #[test]
    fn branch_to_simple_lookaround_transitions() {
        // thompson::NFA(
        // ^000000: capture(pid=0, group=0, slot=0) => 1
        //  000001: Start => 2
        //  000002: h => 3
        //  000003: e => 4
        //  000004: l => 5
        //  000005: l => 6
        //  000006: ' ' => 7
        //  000007: w => 8
        //  000008: o => 9
        //  000009: r => 10
        //  000010: m => 11
        //  000011: End => 12
        //  000012: capture(pid=0, group=0, slot=1) => 13
        //  000013: MATCH(0)
        let nfa = NFA::new("^hell worm$").unwrap();

        let mut test = branch_to_transition_test_closure(nfa, b"hell worm");

        test(0, 0, &[], false);
        test(1, 0, &[], false);
        test(2, 0, &[3], false);
        test(3, 1, &[4], false);
        test(4, 2, &[5], false);
        test(5, 3, &[6], false);
        test(6, 4, &[7], false);
        test(7, 5, &[8], false);
        test(8, 6, &[9], false);
        test(9, 7, &[10], false);
        test(10, 8, &[11, 12, 13], false);
        test(11, 9, &[], false);
        test(12, 0, &[], false);
        test(13, 0, &[], true);
    }

    // It seems like `DenseTransitions` are not constructed in the internal
    // `regex-automata` code

    fn make_current_transitions_test_closure(
        nfa: NFA,
    ) -> impl FnMut(&[i32], u8, Option<&[u8]>, bool) {
        let mut ctx = CompileContext::new(
            nfa,
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );

        // Assume all tests use less than 255 states
        assert_eq!(ctx.state_id_layout(), &Layout::new::<u8>());

        let overall = Layout::new::<()>();

        let (overall, current_set_layout) = SparseSetLayout::new(&mut ctx, overall).unwrap();
        let (overall, next_set_layout) = SparseSetLayout::new(&mut ctx, overall).unwrap();
        let (overall, look_layout) = LookLayout::new(&mut ctx, overall).unwrap();

        let sparse_set_functions = SparseSetFunctions::new(&mut ctx, &current_set_layout);
        let look_funcs = LookFunctions::new(&mut ctx, &look_layout);

        let epsilon_closures =
            EpsilonClosureFunctions::new(&mut ctx, sparse_set_functions.insert, &look_funcs)
                .unwrap();

        let (overall, transition_layout) = TransitionLayout::new(&mut ctx, overall).unwrap();
        let _transition_functions =
            TransitionFunctions::new(&mut ctx, &epsilon_closures, &transition_layout);

        let module_bytes = ctx.compile(&overall).finish();
        let module_bytes = RegexBytecode::from_bytes_unchecked(module_bytes);
        let mut regex =
            crate::engines::wasmi::Executor::with_engine(::wasmi::Engine::default(), &module_bytes)
                .unwrap();

        let make_current_transitions = regex
            .instance()
            // [haystack_ptr, haystack_len, at_offset, current_set_ptr, current_set_len,
            // next_set_ptr, next_set_len]
            .get_typed_func::<(i64, i64, i64, i64, i32, i64, i32), (i32, i32)>(
                regex.store(),
                "make_current_transitions",
            )
            .unwrap();

        let (_, set_insert) = get_sparse_set_fns(regex.instance(), regex.store());

        let haystack_memory = regex
            .instance()
            .get_memory(regex.store(), "haystack")
            .unwrap();
        let state_memory = regex.instance().get_memory(regex.store(), "state").unwrap();

        move |current_states: &[i32],
              byte: u8,
              expected_next_states: Option<&[u8]>,
              expected_is_match: bool| {
            let haystack_ptr = 0;
            let haystack_len = 1;
            let at_offset = 0;
            let current_set_ptr = current_set_layout.set_start_pos as i64;
            let mut current_set_len = 0;
            let next_set_ptr = next_set_layout.set_start_pos as i64;
            let next_set_len = 0;

            // Write haystack byte into memory ahead of transition call
            haystack_memory.data_mut(regex.store_mut())
                [haystack_ptr as usize + at_offset as usize] = byte;
            // Write all current states into set
            for state in current_states {
                current_set_len = set_insert
                    .call(
                        regex.store_mut(),
                        (current_set_len, *state, current_set_ptr),
                    )
                    .unwrap();
            }

            let (new_next_set_len, is_match) = make_current_transitions
                .call(
                    regex.store_mut(),
                    (
                        haystack_ptr,
                        haystack_len,
                        at_offset,
                        current_set_ptr,
                        current_set_len,
                        next_set_ptr,
                        next_set_len,
                    ),
                )
                .unwrap();

            assert_eq!(
                is_match, expected_is_match as i32,
                "{current_states:?} => {byte}/{}",
                byte as char
            );

            if let Some(expected_next_states) = expected_next_states {
                assert_eq!(
                    new_next_set_len,
                    expected_next_states.len() as i32,
                    "{current_states:?} => {byte}/{}",
                    byte as char
                );
                let states = &unsafe {
                    state_memory.data(regex.store())[next_set_layout.set_start_pos
                        ..(next_set_layout.set_start_pos + next_set_layout.set_overall.size())]
                        .align_to::<u8>()
                        .1
                }[0..expected_next_states.len()];
                assert_eq!(
                    states, expected_next_states,
                    "{current_states:?} => {byte}/{}",
                    byte as char
                );
            } else {
                assert_eq!(
                    new_next_set_len, 0,
                    "{current_states:?} => {byte}/{}",
                    byte as char
                );
            }
        }
    }

    #[test]
    fn make_current_transitions_normal() {
        // thompson::NFA(
        //     >000000: binary-union(2, 1)
        //      000001: \x00-\xFF => 0
        //     ^000002: capture(pid=0, group=0, slot=0) => 3
        //      000003: a => 4
        //      000004: b => 5
        //      000005: c => 6
        //      000006: binary-union(3, 7)
        //      000007: capture(pid=0, group=0, slot=1) => 8
        //      000008: MATCH(0)
        let nfa = NFA::new("(?:abc)+").unwrap();

        let mut test = make_current_transitions_test_closure(nfa);

        // No states:
        test(&[], b'a', None, false);
        test(&[], b'b', None, false);
        test(&[], b'c', None, false);

        // Initial states
        test(&[0, 1, 2, 3], b'a', Some(&[0, 1, 2, 3, 4]), false);
        test(&[0, 1, 2, 3], b'b', Some(&[0, 1, 2, 3]), false);
        test(&[0, 1, 2, 3], b'c', Some(&[0, 1, 2, 3]), false);

        // Normal transition states
        test(&[3, 4, 5], b'a', Some(&[4]), false);
        test(&[3, 4, 5], b'b', Some(&[5]), false);
        test(&[3, 4, 5], b'c', Some(&[3, 6, 7, 8]), false);

        // Terminal transition states
        test(&[6, 7], b'a', None, false);
        test(&[6, 7], b'b', None, false);
        test(&[6, 7], b'c', None, false);

        // Success transition states
        test(&[6, 7, 8], b'a', None, true);
        test(&[6, 7, 8], b'b', None, true);
        test(&[6, 7, 8], b'c', None, true);

        // Mixed states
        test(&[3, 4, 5, 8], b'a', Some(&[4]), true);
        test(&[3, 4, 5, 8], b'b', Some(&[5]), true);
        test(&[3, 4, 5, 8], b'c', Some(&[3, 6, 7, 8]), true);
    }
}
