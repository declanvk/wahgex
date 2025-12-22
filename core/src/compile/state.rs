//! This module contains type and functions related to the entire runtime state
//! of the engine.

use std::alloc::{Layout, LayoutError};

use super::{
    BuildError, CompileContext,
    epsilon_closure::EpsilonClosureFunctions,
    lookaround::{LookFunctions, LookLayout},
    pattern::{PatternFunctions, PatternLayout},
    sparse_set::{SparseSetFunctions, SparseSetLayout},
    transition::{TransitionFunctions, TransitionLayout},
};

/// This type will be used to plan the WASM memory layout and precompute the
/// ptr/offsets of various data structures.
#[derive(Debug)]
pub struct StateLayout {
    /// The overall memory layout encompassing all state-related data
    /// structures.
    pub overall: Layout,
    transition: TransitionLayout,
    pub first_sparse_set: SparseSetLayout,
    pub second_sparse_set: SparseSetLayout,
    pattern: PatternLayout,
    look: LookLayout,
}

impl StateLayout {
    /// Creates a new `StateLayout` by sequentially arranging layouts for
    /// various components.
    pub fn new(ctx: &mut CompileContext) -> Result<Self, LayoutError> {
        // Using a ZST to start the layout so that we have minimal alignment
        // requirements
        let overall = Layout::new::<()>();
        let (overall, pattern) = PatternLayout::new(ctx, overall)?;
        let (overall, transition) = TransitionLayout::new(ctx, overall)?;
        let (overall, look) = LookLayout::new(ctx, overall)?;
        let (overall, first_sparse_set) = SparseSetLayout::new(ctx, overall)?;
        let (overall, second_sparse_set) = SparseSetLayout::new(ctx, overall)?;

        let overall = overall.pad_to_align();

        Ok(Self {
            overall,
            transition,
            first_sparse_set,
            second_sparse_set,
            pattern,
            look,
        })
    }
}

/// This struct contains all the functions for manipulating the built-in
/// data structures.
#[derive(Debug)]
pub struct StateFunctions {
    #[expect(dead_code)]
    sparse_set: SparseSetFunctions,
    pub epsilon_closure: EpsilonClosureFunctions,
    pub transition: TransitionFunctions,
    pub pattern: PatternFunctions,
}

impl StateFunctions {
    /// Creates and registers all WebAssembly functions required for managing
    /// the NFA runtime state.
    pub fn new(ctx: &mut CompileContext, layout: &StateLayout) -> Result<Self, BuildError> {
        // It shouldn't matter if we pass the first or the second sparse set, since they
        // have the same
        let sparse_set = SparseSetFunctions::new(ctx, &layout.first_sparse_set);
        let look_funcs = LookFunctions::new(ctx, &layout.look);
        let epsilon_closure = EpsilonClosureFunctions::new(ctx, sparse_set.insert, &look_funcs)?;
        let transition = TransitionFunctions::new(ctx, &epsilon_closure, &layout.transition);
        let pattern = PatternFunctions::new(ctx, &layout.pattern);

        Ok(Self {
            sparse_set,
            epsilon_closure,
            transition,
            pattern,
        })
    }
}
