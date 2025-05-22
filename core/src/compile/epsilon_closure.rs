//! This module contains types and functions related to computing the epsilon
//! closure of a given NFA state.

use std::collections::{HashMap, HashSet};

use regex_automata::{
    nfa::thompson::State,
    util::{look::Look, primitives::StateID},
};
use wasm_encoder::{BlockType, NameMap, ValType};

use super::{
    context::{Function, FunctionDefinition, FunctionIdx, FunctionSignature},
    lookaround::LookFunctions,
    BuildError, CompileContext,
};

/// This struct contains a map of functions that are the pre-computed epsilon
/// closure for each NFA state.
#[derive(Debug)]
pub struct EpsilonClosureFunctions {
    state_closures: HashMap<StateID, FunctionIdx>,
    pub branch_to_epsilon_closure: FunctionIdx,
}

impl EpsilonClosureFunctions {
    /// Create a new set of epsilon closure functions for the given input.
    pub fn new(
        ctx: &mut CompileContext,
        sparse_set_insert: FunctionIdx,
        look_funcs: &LookFunctions,
    ) -> Result<Self, BuildError> {
        let state_closures = Self::all_epsilon_closure_fns(ctx, sparse_set_insert, look_funcs)?;
        let branch_to_epsilon_closure = ctx.add_function(Self::branch_to_epsilon_closure_fn(
            &state_closures,
            sparse_set_insert,
        ));

        Ok(Self {
            state_closures,
            branch_to_epsilon_closure,
        })
    }

    fn all_epsilon_closure_fns(
        ctx: &mut CompileContext,
        sparse_set_insert: FunctionIdx,
        look_funcs: &LookFunctions,
    ) -> Result<HashMap<StateID, FunctionIdx>, BuildError> {
        // NOTE: The indexes of the `states` array correspond to the `StateID` value.
        let mut state_to_epsilon_closure_fn = HashMap::new();

        let num_states = ctx.nfa.states().len();
        for for_sid in (0..num_states).map(StateID::new).map(Result::unwrap) {
            let states = ctx.nfa.states();
            let closure = compute_epsilon_closure(for_sid, states)?;
            if Self::can_omit_epsilon_closure(&closure, for_sid) {
                continue;
            }

            let sig = Self::epsilon_closure_fn_sig(for_sid);
            let func_idx = ctx.declare_function(sig);

            state_to_epsilon_closure_fn.insert(for_sid, func_idx);
        }

        for (for_sid, func_idx) in &state_to_epsilon_closure_fn {
            let states = ctx.nfa.states();
            let closure = compute_epsilon_closure(*for_sid, states)?;
            let def = Self::epsilon_closure_fn_def(
                closure,
                &state_to_epsilon_closure_fn,
                sparse_set_insert,
                look_funcs,
            )?;
            ctx.define_function(*func_idx, def);
        }

        Ok(state_to_epsilon_closure_fn)
    }

    /// Get the epsilon closure function for the given state ID, if present.
    #[expect(dead_code)]
    pub fn get(&self, sid: StateID) -> Option<FunctionIdx> {
        self.state_closures.get(&sid).copied()
    }

    fn branch_to_epsilon_closure_fn(
        epsilon_closures: &HashMap<StateID, FunctionIdx>,
        sparse_set_insert: FunctionIdx,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "next_set_ptr");
        locals_name_map.append(4, "next_set_len");
        locals_name_map.append(5, "state_id");

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        let mut states = epsilon_closures.keys().copied().collect::<Vec<_>>();
        states.sort();

        // This loop will cover any states where [`Self::can_omit_epsilon_closure`]
        // returned false. All other states will fall through to the code below
        // which inserts only the self-state.
        for sid in states {
            let epsilon_closure_fn = epsilon_closures.get(&sid).copied().unwrap();
            instructions
                .local_get(5)
                .i32_const(i32::from_ne_bytes(sid.as_u32().to_ne_bytes()))
                .i32_eq()
                .if_(BlockType::Empty)
                .local_get(0)
                .local_get(1)
                .local_get(2)
                .local_get(3)
                .local_get(4)
                .call(epsilon_closure_fn.into())
                .return_()
                .end();
        }

        // If it falls through to this point, then we must assume thats its a state
        // which has no epsilon transitions. In which case, we need to add the current
        // state to the next set and return.
        instructions
            .local_get(4) // next_set_len
            .local_get(5) // state_id
            .local_get(3) // next_set_ptr
            .call(sparse_set_insert.into())
            .end();

        Function {
            sig: FunctionSignature {
                name: "branch_to_epsilon_closure".into(),
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
                // [new_next_set_len]
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

    /// Return true if we can omit the epsilon closure function for the given
    /// state and closure.
    ///
    /// We can omit epsilon closures which only contain the self-state, since
    /// branch_to_epsilon_closure will always include a default branch to
    /// populate the singleton set.
    fn can_omit_epsilon_closure(closure: &EpsilonClosure, for_sid: StateID) -> bool {
        closure.unconditional.len() == 1
            && closure.unconditional.contains(&for_sid)
            // Return false if there are conditional lookaround transitions from for_sid
            && closure.lookaround.is_empty()
    }

    fn epsilon_closure_fn_sig(for_sid: StateID) -> FunctionSignature {
        FunctionSignature {
            name: format!("epsilon_closure_s{}", for_sid.as_usize()),
            // [haystack_ptr, haystack_len, at_offset, next_set_ptr, next_set_len]
            params_ty: &[
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
            ],
            // [new_next_set_len]
            results_ty: &[ValType::I32],
            export: false,
        }
    }

    fn epsilon_closure_fn_def(
        closure: EpsilonClosure,
        state_to_epsilon_closure_fn: &HashMap<StateID, FunctionIdx>,
        sparse_set_insert: FunctionIdx,
        look_funcs: &LookFunctions,
    ) -> Result<FunctionDefinition, BuildError> {
        let mut unconditional = closure.unconditional.into_iter().collect::<Vec<_>>();
        // need this to keep consistency of snapshot tests
        unconditional.sort();

        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        locals_name_map.append(3, "next_set_ptr");
        locals_name_map.append(4, "next_set_len");
        // Locals
        locals_name_map.append(5, "new_next_set_len");

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);
        let mut instructions = body.instructions();
        // TODO: `haystack_ptr`, `haystack_len`, and `at_offset` will be unused until we
        // support lookaround and need to check the stack

        instructions.local_get(4);

        // TODO(opt): Could optimize this by adding a bulk insert method and loading all
        // of these from a memory location initialized by an active data segment
        for closure_sid in unconditional {
            instructions
                // new_next_set_len is already on the stack from the prelude or the previous call to
                // sparse_set_insert
                .i32_const(i32::from_ne_bytes(closure_sid.as_u32().to_ne_bytes()))
                .local_get(3) // next_set_ptr
                // TODO(opt): Instead of creating a separate function for every state's epsilon
                // transition, have some of them be inlined depending on size.
                .call(sparse_set_insert.into());
        }

        // At this point the stack is [new_next_set_len]

        // Implementation strategy for lookaround:
        //  1. For epsilon transitions that include a `Look`, add a conditional block
        //     after inserting all the unconditional states. The block should be keyed
        //     on whether or not new states were added to the next_set.
        //  2. Inside the block, we should have the actual `Look` conditionals, based on
        //     the haystack.
        //  3. If the look conditional passes, then recurse into the epsilon closure
        //     function of the `next` state. If that function was omitted (see
        //     `can_omit_epsilon_closure`) then just emit some code that adds the `next`
        //     state to the `next_set`.

        if !closure.lookaround.is_empty() {
            instructions
                .local_tee(5)
                .local_get(4)
                .i32_ne()
                .if_(BlockType::Empty);
            for look in closure.lookaround {
                instructions
                    .local_get(0)
                    .local_get(1)
                    .local_get(2)
                    .call(look_funcs.look_matcher(look.look).unwrap().into())
                    .if_(BlockType::Empty);
                // conditional look did match, now call into epsilon transition
                if let Some(epsilon_closure_fn_idx) =
                    state_to_epsilon_closure_fn.get(&look.next).copied()
                {
                    // Recursive call to the next state's epsilon closure fn
                    instructions
                        // Args needed [haystack_ptr, haystack_len, at_offset, next_set_ptr,
                        // new_next_set_len]
                        .local_get(0)
                        .local_get(1)
                        .local_get(2)
                        .local_get(3)
                        .local_get(5)
                        .call(epsilon_closure_fn_idx.into())
                        .local_set(5);
                } else {
                    // Single state insert
                    instructions
                        // Args needed [new_next_set_len, state_id, next_set_ptr]
                        .local_get(5)
                        .i32_const(i32::from_ne_bytes(look.next.as_u32().to_ne_bytes()))
                        .local_get(3)
                        .call(sparse_set_insert.into())
                        .local_set(5);
                }

                instructions.end();
            }

            instructions.end().local_get(5);
        }

        instructions.end();

        Ok(FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        })
    }
}

#[derive(Debug)]
struct EpsilonClosure {
    /// This is the set of states that are unconditionally epsilon-reachable.
    ///
    /// This is contrast to those states that are conditionally
    /// epsilon-reachable through a [`State::Look`] (lookaround).
    unconditional: HashSet<StateID>,
    /// This is the list of lookaround states that are directly reachable from
    /// the `pure` set with no conditional epsilon transitions.
    lookaround: Vec<EpsilonLook>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EpsilonLook {
    next: StateID,
    look: Look,
}

fn compute_epsilon_closure(sid: StateID, states: &[State]) -> Result<EpsilonClosure, BuildError> {
    let mut unconditional: HashSet<_> = HashSet::new();

    let mut lookaround = Vec::new();

    let mut stack = vec![sid];
    'stack: while let Some(mut sid) = stack.pop() {
        loop {
            if !unconditional.insert(sid) {
                continue 'stack;
            }

            match &states[sid.as_usize()] {
                State::Fail
                | State::Match { .. }
                | State::ByteRange { .. }
                | State::Sparse { .. }
                | State::Dense { .. } => {
                    // TODO: Need to integrate here for slot/matching support
                    continue 'stack;
                },
                State::Look { look, next } => {
                    lookaround.push(EpsilonLook {
                        next: *next,
                        look: *look,
                    });
                },
                State::Union { alternates } => {
                    sid = match alternates.first() {
                        None => continue 'stack,
                        Some(&sid) => sid,
                    };
                    stack.extend(alternates[1..].iter().copied().rev());
                },
                State::BinaryUnion { alt1, alt2 } => {
                    sid = *alt1;
                    stack.push(*alt2);
                },
                State::Capture { next, .. } => {
                    // TODO: Need to integrate here for slot/matching support
                    sid = *next;
                },
            }
        }
    }

    Ok(EpsilonClosure {
        unconditional,
        lookaround,
    })
}

#[cfg(test)]
mod tests {
    use std::alloc::Layout;

    use regex_automata::nfa::thompson::NFA;

    use crate::compile::{
        lookaround::LookLayout,
        sparse_set::{SparseSetFunctions, SparseSetLayout},
        tests::setup_interpreter,
    };

    use super::*;

    #[test]
    fn test_epsilon_closures() {
        let re = NFA::new("(Hello)* world").unwrap();
        // thompson::NFA(
        //     >000000: binary-union(2, 1)
        //      000001: \x00-\xFF => 0
        //     ^000002: capture(pid=0, group=0, slot=0) => 3
        //      000003: binary-union(4, 11)
        //      000004: capture(pid=0, group=1, slot=2) => 5
        //      000005: H => 6
        //      000006: e => 7
        //      000007: l => 8
        //      000008: l => 9
        //      000009: o => 10
        //      000010: capture(pid=0, group=1, slot=3) => 3
        //      000011: ' ' => 12
        //      000012: w => 13
        //      000013: o => 14
        //      000014: r => 15
        //      000015: l => 16
        //      000016: d => 17
        //      000017: capture(pid=0, group=0, slot=1) => 18
        //      000018: MATCH(0)

        let test = |sid: StateID, expected_states: &[usize]| {
            let closure = compute_epsilon_closure(sid, re.states()).unwrap();
            assert_eq!(
                closure.unconditional,
                expected_states
                    .iter()
                    .copied()
                    .map(StateID::new)
                    .map(Result::unwrap)
                    .collect(),
                "Closure from state {sid:?} on:\n{re:?}",
            );
        };

        test(StateID::ZERO, &[2, 1, 4, 11, 3, 0, 5]);
        test(StateID::new(3).unwrap(), &[3, 4, 5, 11]);
        test(StateID::new(4).unwrap(), &[4, 5]);
        test(StateID::new(5).unwrap(), &[5]);
    }

    #[test]
    fn test_large_union_epsilon_closure() {
        let re = NFA::new("a*|b*|c*|d*|e*").unwrap();
        // thompson::NFA(
        //     >000000: binary-union(2, 1)
        //      000001: \x00-\xFF => 0
        //     ^000002: capture(pid=0, group=0, slot=0) => 7
        //      000003: binary-union(4, 14)
        //      000004: a => 3
        //      000005: binary-union(6, 14)
        //      000006: b => 5
        //      000007: union(3, 5, 8, 10, 12)
        //      000008: binary-union(9, 14)
        //      000009: c => 8
        //      000010: binary-union(11, 14)
        //      000011: d => 10
        //      000012: binary-union(13, 14)
        //      000013: e => 12
        //      000014: capture(pid=0, group=0, slot=1) => 15
        //      000015: MATCH(0)

        let test = |sid: StateID, expected_states: &[usize]| {
            let closure = compute_epsilon_closure(sid, re.states()).unwrap();
            assert_eq!(
                closure.unconditional,
                expected_states
                    .iter()
                    .copied()
                    .map(StateID::new)
                    .map(Result::unwrap)
                    .collect(),
                "Closure from state {sid:?} on:\n{re:?}",
            );
        };

        test(
            StateID::new(7).unwrap(),
            &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        );
        test(StateID::new(3).unwrap(), &[3, 4, 14, 15]);
        test(StateID::new(4).unwrap(), &[4]);
        test(StateID::new(14).unwrap(), &[14, 15]);
    }

    #[test]
    fn lookaround_epsilon_closure_panic() {
        let re = NFA::new(r"^hell (?:worm$|world)").unwrap();
        // thompson::NFA(
        // ^000000: capture(pid=0, group=0, slot=0) => 1
        //  000001: Start => 2
        //  000002: h => 3
        //  000003: e => 4
        //  000004: l => 5
        //  000005: l => 6
        //  000006: ' ' => 17
        //  000007: w => 8
        //  000008: o => 9
        //  000009: r => 10
        //  000010: m => 11
        //  000011: End => 18
        //  000012: w => 13
        //  000013: o => 14
        //  000014: r => 15
        //  000015: l => 16
        //  000016: d => 18
        //  000017: binary-union(7, 12)
        //  000018: capture(pid=0, group=0, slot=1) => 19
        //  000019: MATCH(0)

        {
            let closure = compute_epsilon_closure(StateID::new(0).unwrap(), re.states()).unwrap();

            assert_eq!(
                closure.unconditional,
                [0, 1]
                    .iter()
                    .copied()
                    .map(StateID::new)
                    .map(Result::unwrap)
                    .collect()
            );
            assert_eq!(
                closure.lookaround,
                vec![EpsilonLook {
                    next: StateID::new(2).unwrap(),
                    look: Look::Start,
                }]
            );
        }

        {
            let closure = compute_epsilon_closure(StateID::new(11).unwrap(), re.states()).unwrap();

            assert_eq!(
                closure.unconditional,
                [11].iter()
                    .copied()
                    .map(StateID::new)
                    .map(Result::unwrap)
                    .collect()
            );
            assert_eq!(
                closure.lookaround,
                vec![EpsilonLook {
                    next: StateID::new(18).unwrap(),
                    look: Look::End,
                }]
            );
        }
    }

    fn compile_test_module(nfa: NFA) -> Vec<u8> {
        let mut ctx = CompileContext::new(
            nfa,
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );
        // Assume all tests use less than 255 states
        assert_eq!(ctx.state_id_layout(), &Layout::new::<u8>());

        let overall = Layout::new::<()>();
        let (overall, sparse_set_layout) = SparseSetLayout::new(&mut ctx, overall).unwrap();
        let (overall, look_layout) = LookLayout::new(&mut ctx, overall).unwrap();
        let sparse_set_functions = SparseSetFunctions::new(&mut ctx, &sparse_set_layout);
        let look_funcs = LookFunctions::new(&mut ctx, &look_layout).unwrap();

        let _epsilon_closure_functions =
            EpsilonClosureFunctions::new(&mut ctx, sparse_set_functions.insert, &look_funcs);

        let module = ctx.compile(&overall);
        module.finish()
    }

    fn setup_epsilon_closure_test(nfa: NFA, haystack: &[u8]) -> impl FnMut(i32, i64, &[u8]) + '_ {
        let module_bytes = compile_test_module(nfa.clone());
        let (_engine, _module, mut store, instance) = setup_interpreter(&module_bytes);
        let branch_to_epsilon_closure = instance
            .get_typed_func::<(i64, i64, i64, i64, i32, i32), i32>(
                &store,
                "branch_to_epsilon_closure",
            )
            .unwrap();

        let state_memory = instance.get_memory(&store, "state").unwrap();
        let haystack_memory = instance.get_memory(&store, "haystack").unwrap();

        // Assuming that haystack starts at 0
        haystack_memory.data_mut(&mut store)[0..haystack.len()].copy_from_slice(haystack);

        move |state_id, at_offset: i64, expected_states: &[u8]| {
            let haystack_ptr = 0;
            let haystack_len = haystack.len() as i64;
            // Would be safer if we passed the layout through and we read the set start
            // position instead of assuming its at 0.
            let set_ptr = 0;
            let new_set_len = branch_to_epsilon_closure
                .call(
                    &mut store,
                    (
                        haystack_ptr,
                        haystack_len,
                        at_offset,
                        set_ptr,
                        0, /* set_len */
                        state_id,
                    ),
                )
                .unwrap();

            let new_set_len = usize::try_from(new_set_len).unwrap();

            assert_eq!(
                new_set_len,
                expected_states.len(),
                "state [{state_id}] @ {at_offset}"
            );
            let epsilon_states = compute_epsilon_closure(
                StateID::must(usize::try_from(state_id).unwrap()),
                nfa.states(),
            )
            .unwrap();
            assert!(
                epsilon_states.unconditional.len() <= expected_states.len(),
                "state [{state_id}] @ {at_offset}"
            );

            // Would be safer if we passed the layout through and we read the set start
            // position instead of assuming its at 0.
            let states = &unsafe { state_memory.data(&store).align_to::<u8>().1 }[0..new_set_len];
            assert_eq!(states, expected_states, "state [{state_id}] @ {at_offset}");
        }
    }

    #[test]
    fn basic_epsilon_closure() {
        // thompson::NFA(
        // >000000: binary-union(2, 1)
        //  000001: \x00-\xFF => 0
        // ^000002: capture(pid=0, group=0, slot=0) => 3
        //  000003: binary-union(4, 11)
        //  000004: capture(pid=0, group=1, slot=2) => 5
        //  000005: H => 6
        //  000006: e => 7
        //  000007: l => 8
        //  000008: l => 9
        //  000009: o => 10
        //  000010: capture(pid=0, group=1, slot=3) => 3
        //  000011: ' ' => 12
        //  000012: w => 13
        //  000013: o => 14
        //  000014: r => 15
        //  000015: l => 16
        //  000016: d => 17
        //  000017: capture(pid=0, group=0, slot=1) => 18
        //  000018: MATCH(0)
        let nfa = NFA::new("(Hello)* world").unwrap();

        let mut test = setup_epsilon_closure_test(nfa, b"");

        test(0, 0, &[0, 1, 2, 3, 4, 5, 11]);
        test(3, 0, &[3, 4, 5, 11]);
        test(4, 0, &[4, 5]);
        test(5, 0, &[5]);
    }

    #[test]
    fn simple_lookaround_epsilon_closure() {
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
        let mut test = setup_epsilon_closure_test(nfa, b"hell worm");

        // 2 state is reachable because we're at position 0 and the `Start` state
        // matches
        test(0, 0, &[0, 1, 2]);
        // It doesn't match for this state
        test(0, 1, &[0, 1]);

        // Similarly, we get all the end state matches here
        test(11, 9, &[11, 12, 13]);
    }
}
