//! This module contains types and functions related to computing the epsilon
//! closure of a given NFA state.

use std::collections::{HashMap, HashSet};

use regex_automata::{nfa::thompson::State, util::primitives::StateID};
use wasm_encoder::{NameMap, ValType};

use super::{
    context::{Function, FunctionIdx},
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
    ) -> Result<Self, BuildError> {
        // NOTE: The indexes of the `states` array correspond to the `StateID` value.
        let mut state_closures = HashMap::new();

        let states = ctx.nfa.states();

        for for_sid in (0..states.len()).map(StateID::new).map(Result::unwrap) {
            let closure_fn =
                Self::epsilon_closure_fn(for_sid, ctx.nfa.states(), sparse_set_insert)?;
            let closure_idx = ctx.sections.add_function(closure_fn);
            state_closures.insert(for_sid, closure_idx);
        }

        let branch_to_epsilon_closure =
            ctx.sections
                .add_function(Self::branch_to_epsilon_closure_fn(
                    &state_closures,
                    sparse_set_insert,
                ));

        Ok(Self {
            state_closures,
            branch_to_epsilon_closure,
        })
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

        for sid in states {
            let epsilon_closure_fn = epsilon_closures.get(&sid).copied().unwrap();
            instructions
                .local_get(5)
                .i32_const(i32::from_ne_bytes(sid.as_u32().to_ne_bytes()))
                .i32_eq()
                .if_(wasm_encoder::BlockType::Empty)
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
            .local_get(3)
            .local_get(4)
            .local_get(5)
            .call(sparse_set_insert.into())
            .end();

        Function {
            name: "branch_to_epsilon_closure".into(),
            // [haystack_ptr, haystack_len, at_offset, next_set_ptr, next_set_len, state_id]
            params_ty: &[
                // TODO(opt): Remove haystack_ptr and assume that haystack always starts at offset
                // 0 in memory 0
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
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn epsilon_closure_fn(
        for_sid: StateID,
        states: &[State],
        sparse_set_insert: FunctionIdx,
    ) -> Result<Function, BuildError> {
        let closure = compute_epsilon_closure(for_sid, states)?;
        let mut closure = closure.into_iter().collect::<Vec<_>>();
        // need this to keep consistency of snapshot tests
        closure.sort();

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
        // support lookahead and need to check the stack

        instructions.local_get(4).local_set(5);

        // TODO(opt): Could optimize this by adding a bulk insert method and loading all
        // of these from a memory location initialized by an active data segment
        for closure_sid in closure {
            instructions
                .local_get(3)
                .local_get(5)
                .i32_const(i32::from_ne_bytes(closure_sid.as_u32().to_ne_bytes()))
                // TODO(opt): Instead of creating a separate function for every state's epsilon
                // transition, have some of them be inlined depending on size.
                .call(sparse_set_insert.into())
                .local_set(5);
        }

        instructions.local_get(5).end();

        Ok(Function {
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
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        })
    }
}

fn compute_epsilon_closure(sid: StateID, states: &[State]) -> Result<HashSet<StateID>, BuildError> {
    let mut seen: HashSet<_> = HashSet::new();

    let mut stack = vec![sid];
    'stack: while let Some(mut sid) = stack.pop() {
        loop {
            if !seen.insert(sid) {
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
                State::Look { .. } => {
                    return Err(BuildError::unsupported("lookahead is not yet implemented"));
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

    Ok(seen)
}

#[cfg(test)]
mod tests {
    use std::alloc::Layout;

    use regex_automata::nfa::thompson::NFA;

    use crate::compile::{
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
                closure,
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
                closure,
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
    #[should_panic = "lookahead is not yet implemented"]
    fn lookahead_epsilon_closure_panic() {
        let re = NFA::new(r"(Hello)*\b").unwrap();
        let _closure = compute_epsilon_closure(StateID::ZERO, re.states()).unwrap();
    }

    fn compile_test_module(nfa: NFA) -> Vec<u8> {
        let mut ctx = CompileContext::new(
            nfa,
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );

        let overall = Layout::new::<()>();
        let (overall, sparse_set_layout) = SparseSetLayout::new(&mut ctx, overall).unwrap();
        let sparse_set_functions = SparseSetFunctions::new(&mut ctx, &sparse_set_layout);

        let _epsilon_closure_functions =
            EpsilonClosureFunctions::new(&mut ctx, sparse_set_functions.insert);

        let module = ctx.compile(&overall);
        module.finish()
    }

    #[test]
    fn basic_epsilon_closure() {
        let nfa = NFA::new("(Hello)* world").unwrap();

        let module_bytes = compile_test_module(nfa.clone());
        let (_engine, _module, mut store, instance) = setup_interpreter(&module_bytes);
        let branch_to_epsilon_closure = instance
            .get_typed_func::<(i64, i64, i64, i64, i32, i32), i32>(
                &store,
                "branch_to_epsilon_closure",
            )
            .unwrap();

        let state_memory = instance.get_memory(&store, "state").unwrap();

        let mut test = |state_id, expected_states: &[i32]| {
            let haystack_ptr = 0;
            let haystack_len = 0;
            let at_offset = 0;
            // Would be safer if we passed the layout through and we read the set start
            // position instead of assuming its at 0.
            let set_ptr = 0;
            let new_set_len = branch_to_epsilon_closure
                .call(
                    &mut store,
                    (haystack_ptr, haystack_len, at_offset, set_ptr, 0, state_id),
                )
                .unwrap();

            let new_set_len = usize::try_from(new_set_len).unwrap();

            assert_eq!(new_set_len, expected_states.len(), "for state [{state_id}]");
            let epsilon_states = compute_epsilon_closure(
                StateID::must(usize::try_from(state_id).unwrap()),
                nfa.states(),
            )
            .unwrap();
            assert_eq!(
                epsilon_states.len(),
                expected_states.len(),
                "for state [{state_id}]"
            );

            // Would be safer if we passed the layout through and we read the set start
            // position instead of assuming its at 0.
            let states = &unsafe { state_memory.data(&store).align_to::<i32>().1 }[0..new_set_len];
            assert_eq!(states, expected_states, "for state [{state_id}]");
        };

        test(0, &[0, 1, 2, 3, 4, 5, 11]);
        test(3, &[3, 4, 5, 11]);
        test(4, &[4, 5]);
        test(5, &[5]);
    }
}
