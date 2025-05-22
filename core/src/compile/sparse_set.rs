/*!
This module defines a sparse set data structure. Its most interesting
properties are:

* They preserve insertion order.
* Set membership testing is done in constant time.
* Set insertion is done in constant time.
* Clearing the set is done in constant time.

The cost for doing this is that the capacity of the set needs to be known up
front, and the elements in the set are limited to state identifiers.

These sets are principally used when traversing an NFA state graph. This
happens at search time, for example, in the PikeVM. It also happens during DFA
determinization.

Copied above documentation from https://github.com/rust-lang/regex/blob/master/regex-automata/src/util/sparse_set.rs
and based my implementation off the same file.
*/

use std::alloc::{Layout, LayoutError};

use wasm_encoder::{BlockType, NameMap, ValType};

use crate::util::repeat;

use super::{
    context::{Function, FunctionDefinition, FunctionIdx, FunctionSignature},
    instructions::InstructionSinkExt,
    CompileContext,
};

/// This struct describes the layout of a "sparse set", which is used to
/// track NFA state ID membership.
///
/// This type has methods which will generate WASM functions that operate on the
/// set in WASM memory.
#[derive(Debug)]
pub struct SparseSetLayout {
    #[cfg_attr(not(test), expect(dead_code))]
    dense_layout: Layout,
    #[cfg_attr(not(test), expect(dead_code))]
    dense_stride: usize,
    #[cfg_attr(not(test), expect(dead_code))]
    sparse_layout: Layout,
    #[cfg_attr(not(test), expect(dead_code))]
    sparse_stride: usize,

    #[cfg_attr(not(test), expect(dead_code))]
    pub set_overall: Layout,
    pub set_start_pos: usize,
    sparse_array_offset: usize,

    state_id_layout: Layout,
}

impl SparseSetLayout {
    /// Create a new sparse set layout for the given [`NFA`].
    ///
    /// The sparse set will be scaled to the number of states in the NFA.
    pub fn new(ctx: &mut CompileContext, overall: Layout) -> Result<(Layout, Self), LayoutError> {
        let num_states = ctx.nfa.states().len();

        Self::with_num_states(num_states, overall, ctx.state_id_layout())
    }

    fn with_num_states(
        num_states: usize,
        overall: Layout,
        state_id_layout: &Layout,
    ) -> Result<(Layout, Self), LayoutError> {
        // First field: `dense` - an array of length `num_state`, that contains the
        // state IDs in the order they were inserted
        let (dense_layout, dense_stride) = repeat(state_id_layout, num_states)?;

        // Second field: `sparse` - an array of length `num_state`, that contains the
        // state IDs in the order they were inserted
        let (sparse_layout, sparse_stride) = repeat(state_id_layout, num_states)?;

        let (set_overall, sparse_array_offset) = dense_layout.extend(sparse_layout)?;
        let (overall, set_start_pos) = overall.extend(set_overall)?;

        // The `len` field, which would normally be first, is going to be passed around
        // by function parameter

        let state_id_layout = *state_id_layout;

        Ok((
            overall,
            Self {
                set_overall,
                dense_layout,
                dense_stride,
                set_start_pos,
                sparse_layout,
                sparse_stride,
                sparse_array_offset,
                state_id_layout,
            },
        ))
    }
}

/// This struct contains the sparse set functions
#[derive(Debug)]
pub struct SparseSetFunctions {
    #[expect(dead_code)]
    pub contains: FunctionIdx,
    pub insert: FunctionIdx,
}

impl SparseSetFunctions {
    /// Register all the sparse set functions and save their
    /// [`FunctionIdx`]s.
    pub fn new(ctx: &mut CompileContext, layout: &SparseSetLayout) -> Self {
        let contains = ctx.add_function(Self::contains_fn(layout));
        let insert = ctx.add_function(Self::insert_fn(layout, contains));

        Self { contains, insert }
    }

    /// Returns a WASM function that will check whether a given state ID is
    /// present in the set or not.
    ///
    /// If it is present, it returns `1`, else it returns `0`
    fn contains_fn(layout: &SparseSetLayout) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "set_ptr");
        locals_name_map.append(1, "set_len");
        locals_name_map.append(2, "state_id");
        // Locals
        locals_name_map.append(3, "index");

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);
        body.instructions()
            // let index = self.sparse[id];
            .local_get(2)
            .i64_extend_i32_u()
            .i64_const(layout.state_id_layout.size().try_into().unwrap())
            .i64_mul() // need to scale the `state_id` index by the size of the elements of the array
            .local_get(0)
            .i64_add()
            .state_id_load(
                // sparse array is after dense
                layout.sparse_array_offset.try_into().unwrap(),
                &layout.state_id_layout,
            )
            .local_tee(3)
            // index.as_usize() < self.len()
            .local_get(1)
            .i32_ge_u()
            // returns `1` if `index.as_usize() >= self.len()`, meaning we should early exit
            .if_(BlockType::Empty)
            // `false` as an i32
            .i32_const(0)
            .return_()
            .end()
            // && self.dense[index] == id
            .local_get(3)
            .i64_extend_i32_u()
            .i64_const(layout.state_id_layout.size().try_into().unwrap())
            .i64_mul() // need to scale the `state_id` index by the size of the elements of the array
            .local_get(0)
            .i64_add()
            .state_id_load(
                // dense array is at offset 0
                0,
                &layout.state_id_layout,
            )
            .local_get(2)
            .i32_eq()
            .end();

        Function {
            sig: FunctionSignature {
                name: "sparse_set_contains".into(),
                params_ty: &[ValType::I64, ValType::I32, ValType::I32],
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

    /// Returns a WASM function that will insert the state ID value into this
    /// set and return `1` if the given state ID was not previously in this
    /// set.
    ///
    /// This operation is idempotent. If the given value is already in this
    /// set, then this is a no-op.
    fn insert_fn(layout: &SparseSetLayout, contains: FunctionIdx) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "set_len");
        locals_name_map.append(1, "state_id");
        locals_name_map.append(2, "set_ptr");

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // if self.contains(id) {
            //     return set_len;
            // }
            .local_get(2) // set_ptr for contains
            .local_get(0) // set_len for contains
            .local_get(1) // state_id for contains
            .call(contains.into())
            .i32_const(true as i32)
            .i32_eq()
            .if_(BlockType::Empty)
            .local_get(0) // return current set_len
            .return_()
            .end()
            // self.dense[index] = id;
            .local_get(0) // set_len as index
            .i64_extend_i32_u()
            .i64_const(layout.state_id_layout.size().try_into().unwrap())
            .i64_mul() // need to scale the `state_id` index by the size of the elements of the array
            .local_get(2) // set_ptr
            .i64_add()
            .local_get(1) // state_id
            .state_id_store(
                // dense is at offset 0
                0,
                &layout.state_id_layout,
            )
            // self.sparse[id] = index;
            .local_get(1) // state_id
            .i64_extend_i32_u()
            .i64_const(layout.state_id_layout.size().try_into().unwrap())
            .i64_mul() // need to scale the `state_id` index by the size of the elements of the array
            .local_get(2) // set_ptr
            .i64_add()
            .local_get(0) // set_len as index
            .state_id_store(
                // sparse is after dense
                layout.sparse_array_offset.try_into().unwrap(),
                &layout.state_id_layout,
            )
            .local_get(0) // current set_len
            .i32_const(1)
            .i32_add()
            .end();

        Function {
            sig: FunctionSignature {
                name: "sparse_set_insert".into(),
                // [set_len, state_id, set_ptr]
                params_ty: &[ValType::I32, ValType::I32, ValType::I64],
                // [new_set_len]
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
}

#[cfg(test)]
pub mod tests {
    use regex_automata::nfa::thompson::NFA;

    use crate::compile::tests::setup_interpreter;

    use super::*;

    fn compile_test_module(layout: &SparseSetLayout) -> Vec<u8> {
        let mut ctx = CompileContext::new(
            NFA::never_match(),
            crate::Config::new()
                .export_all_functions(true)
                .export_state(true),
        );

        let _funcs = SparseSetFunctions::new(&mut ctx, layout);

        let module = ctx.compile(&layout.set_overall);
        module.finish()
    }

    pub fn get_sparse_set_fns(
        instance: &wasmi::Instance,
        store: &wasmi::Store<()>,
    ) -> (
        wasmi::TypedFunc<(i64, i32, i32), i32>, // contains: (ptr, len, id) -> bool
        wasmi::TypedFunc<(i32, i32, i64), i32>, // insert: (len, id, ptr) -> new_len
    ) {
        let sparse_set_contains = instance
            .get_typed_func::<(i64, i32, i32), i32>(&store, "sparse_set_contains")
            .unwrap();

        let sparse_set_insert = instance
            .get_typed_func::<(i32, i32, i64), i32>(&store, "sparse_set_insert")
            .unwrap();

        (sparse_set_contains, sparse_set_insert)
    }

    #[test]
    fn test_sparse_set_layout() {
        // Lets do a non-standard layout to start
        let overall = Layout::new::<u16>();
        let state_id_layout = Layout::new::<u32>();

        // Layout one sparse array at offset 0 with 5 states capacity.
        let (overall, sparse_set_layout) =
            SparseSetLayout::with_num_states(7, overall, &state_id_layout).unwrap();

        assert_eq!(overall, Layout::from_size_align(60, 4).unwrap());

        assert_eq!(
            sparse_set_layout.dense_layout,
            Layout::from_size_align(28, 4).unwrap()
        );
        assert_eq!(sparse_set_layout.dense_stride, 4);
        assert_eq!(
            sparse_set_layout.sparse_layout,
            Layout::from_size_align(28, 4).unwrap()
        );
        assert_eq!(sparse_set_layout.sparse_stride, 4);

        assert_eq!(
            sparse_set_layout.set_overall,
            Layout::from_size_align(56, 4).unwrap()
        );
        assert_eq!(sparse_set_layout.set_start_pos, 4);
        assert_eq!(sparse_set_layout.sparse_array_offset, 28);
    }

    #[test]
    fn test_init_insert_contains() {
        let overall = Layout::new::<()>();
        let state_id_layout = Layout::new::<u32>();

        // Layout one sparse array at offset 0 with 5 states capacity.
        let (_overall, sparse_set_layout) =
            SparseSetLayout::with_num_states(5, overall, &state_id_layout).unwrap();
        let module_bytes = compile_test_module(&sparse_set_layout);
        let (_engine, _module, mut store, instance) = setup_interpreter(module_bytes);
        let (contains, insert) = get_sparse_set_fns(&instance, &store);

        let state_memory = instance.get_memory(&store, "state").unwrap();

        let set_ptr = i64::from_ne_bytes(
            u64::try_from(sparse_set_layout.set_start_pos)
                .unwrap()
                .to_ne_bytes(),
        );
        let set_len = 0;

        let res = contains.call(&mut store, (set_ptr, set_len, 0)).unwrap();
        // true because 0 was not present in the set
        assert_eq!(res, false as i32);

        let set_len = insert.call(&mut store, (set_len, 0, set_ptr)).unwrap();
        assert_eq!(set_len, 1);

        let res = contains.call(&mut store, (set_ptr, set_len, 0)).unwrap();
        // true because 0 is already present in the set
        assert_eq!(res, true as i32);

        let set_len = insert.call(&mut store, (set_len, 0, set_ptr)).unwrap();
        assert_eq!(set_len, 1);

        let res = contains.call(&mut store, (set_ptr, set_len, 0)).unwrap();
        // true because 0 is already present in the set
        assert_eq!(res, true as i32);

        let mut set_len = set_len;
        for state_id in 1..5 {
            let new_set_len = insert
                .call(&mut store, (set_len, state_id, set_ptr))
                .unwrap();
            assert_eq!(new_set_len, set_len + 1);
            set_len = new_set_len;
        }

        for state_id in 0..5 {
            let res = contains
                .call(&mut store, (set_ptr, set_len, state_id))
                .unwrap();
            // true because state is already present in the set
            assert_eq!(res, true as i32, "{state_id} should be present");
        }

        #[rustfmt::skip]
        assert_eq!(
            &state_memory.data(&store)[..(state_id_layout.size() * 5 * 2)],
            &[
                0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
                0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
            ]
        );

        // Reset length, now set is empty
        for state_id in 0..5 {
            let res = contains.call(&mut store, (set_ptr, 0, state_id)).unwrap();
            // true because state is already present in the set
            assert_eq!(res, false as i32, "{state_id} should not be present");
        }
    }

    #[test]
    fn test_init_insert_reverse_contains() {
        let overall = Layout::new::<()>();
        let state_id_layout = Layout::new::<u32>();

        // Layout one sparse array at offset 0 with 5 states capacity.
        let (_overall, sparse_set_layout) =
            SparseSetLayout::with_num_states(5, overall, &state_id_layout).unwrap();
        let module_bytes = compile_test_module(&sparse_set_layout);
        let (_engine, _module, mut store, instance) = setup_interpreter(module_bytes);
        let (contains, insert) = get_sparse_set_fns(&instance, &store);

        let state_memory = instance.get_memory(&store, "state").unwrap();

        let set_ptr = 0;
        let mut set_len = 0;

        for state_id in [4, 1, 0, 2, 3] {
            let res = contains
                .call(&mut store, (set_ptr, set_len, state_id))
                .unwrap();
            // true because state is not present in the set
            assert_eq!(res, false as i32, "{state_id} should not be present");
        }

        // inserting in weird order doesn't affect function

        for state_id in [4, 1, 0, 2, 3] {
            let new_set_len = insert
                .call(&mut store, (set_len, state_id, set_ptr))
                .unwrap();
            // true because state_id was not present in the set
            assert_eq!(new_set_len, set_len + 1);
            set_len = new_set_len;
        }

        for state_id_check in [4, 1, 0, 2, 3] {
            let res = contains
                .call(&mut store, (set_ptr, set_len, state_id_check))
                .unwrap();
            // true because state is already present in the set
            assert_eq!(res, true as i32, "{state_id_check} should be present");
        }

        #[rustfmt::skip]
        assert_eq!(
            &state_memory.data(&store)[..(state_id_layout.size() * 5 * 2)],
            &[
                4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0,
                2, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn test_state_id_larger_than_one_byte() {
        let overall = Layout::new::<()>();
        let state_id_layout = Layout::new::<u32>();

        // Layout one sparse array at offset 0 with 512 states capacity.
        let (_overall, sparse_set_layout) =
            SparseSetLayout::with_num_states(512, overall, &state_id_layout).unwrap();
        let module_bytes = compile_test_module(&sparse_set_layout);
        let (_engine, _module, mut store, instance) = setup_interpreter(module_bytes);
        let (contains, insert) = get_sparse_set_fns(&instance, &store);

        let state_memory = instance.get_memory(&store, "state").unwrap();

        let set_ptr = 0;
        let set_len = 0;

        let res = contains.call(&mut store, (set_ptr, set_len, 511)).unwrap();
        assert_eq!(res, false as i32);

        let set_len = insert.call(&mut store, (set_len, 256, set_ptr)).unwrap();
        assert_eq!(set_len, 1);

        let set_len = insert.call(&mut store, (set_len, 511, set_ptr)).unwrap();
        assert_eq!(set_len, 2);

        let res = contains.call(&mut store, (set_ptr, set_len, 511)).unwrap();
        assert_eq!(res, true as i32);
        let res = contains.call(&mut store, (set_ptr, set_len, 256)).unwrap();
        assert_eq!(res, true as i32);

        // dense entries
        assert_eq!(
            &state_memory.data(&store)[0..state_id_layout.size()],
            &[0, 1, 0, 0]
        );
        assert_eq!(
            &state_memory.data(&store)[state_id_layout.size()..(2 * state_id_layout.size())],
            &[255, 1, 0, 0]
        );

        // sparse entries
        assert_eq!(
            &state_memory.data(&store)[(sparse_set_layout.sparse_array_offset
                + 256 * state_id_layout.size())
                ..(sparse_set_layout.sparse_array_offset + 257 * state_id_layout.size())],
            &[0, 0, 0, 0]
        );
        assert_eq!(
            &state_memory.data(&store)[(sparse_set_layout.sparse_array_offset
                + 511 * state_id_layout.size())
                ..(sparse_set_layout.sparse_array_offset + 512 * state_id_layout.size())],
            &[1, 0, 0, 0]
        );
    }
}
