//! Utilities used to run [`wahgex`][crate] compiled regular expressions
//! using [`wasmtime`].

use std::fmt;

pub use wasmtime;
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::{RegexBytecode, common_input_validation, engines::IsMatchArgs, input::InputOpts};

#[derive(Debug)]
pub(crate) struct Executor {
    _engine: Engine,
    _module: Module,
    store: Store<()>,
    instance: Instance,
}

impl Executor {
    /// Creates a new `Executor` with the given `wasmtime` engine and
    /// `RegexBytecode`.
    pub fn with_engine(engine: Engine, bytecode: &RegexBytecode) -> Result<Self, wasmtime::Error> {
        let module = Module::new(&engine, bytecode)?;
        let mut store = Store::new(&engine, ());
        let linker = Linker::<()>::new(&engine);
        let instance = linker.instantiate(&mut store, &module).unwrap();

        Ok(Self {
            _engine: engine,
            _module: module,
            store,
            instance,
        })
    }
}

/// The main entry point for executing a compiled regular expression with the
/// [`wasmtime`] engine.
pub struct Regex {
    executor: Executor,
    prepare_input: TypedFunc<i64, i32>,
    is_match: TypedFunc<IsMatchArgs, i32>,
    haystack: Memory,
}

impl fmt::Debug for Regex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            executor,
            prepare_input: _,
            is_match: _,
            haystack,
        } = self;
        f.debug_struct("Regex")
            .field("executor", &executor)
            .field("prepare_input", &"[...]")
            .field("is_match", &"[...]")
            .field("haystack", &haystack)
            .finish()
    }
}

impl Regex {
    /// Creates a new `Regex` instance with the default `wasmtime` engine.
    ///
    /// This is a convenience function that uses the default [`Engine`]
    /// configuration. For more control over the engine, use
    /// [`with_engine`][Self::with_engine].
    pub fn new(bytecode: &RegexBytecode) -> Result<Self, wasmtime::Error> {
        Self::with_engine(Engine::default(), bytecode)
    }

    /// Creates a new `Regex` instance with the given `wasmtime` engine.
    ///
    /// # Panics
    ///
    /// This function will panic if the provided `RegexBytecode` is not
    /// well-formed and is missing any of the expected functions or memory.
    pub fn with_engine(engine: Engine, bytecode: &RegexBytecode) -> Result<Self, wasmtime::Error> {
        let mut executor = Executor::with_engine(engine, bytecode)?;

        let prepare_input = executor
            .instance
            .get_typed_func::<i64, i32>(&mut executor.store, "prepare_input")
            .expect(
                "If the `RegexBytecode` passed is well-formed, then there must be a \
                 `prepare_input` function",
            );
        let is_match = executor
            .instance
            // [anchored, anchored_pattern, span_start, span_end, haystack_len]
            .get_typed_func::<IsMatchArgs, i32>(&mut executor.store, "is_match")
            .expect(
                "If the `RegexBytecode` passed is well-formed, then there must be a `is_match` \
                 function",
            );
        let haystack: Memory = executor
            .instance
            .get_memory(&mut executor.store, "haystack")
            .expect(
                "If the `RegexBytecode` passed is well-formed, then there must be a `haystack` \
                 memory",
            );

        Ok(Self {
            executor,
            prepare_input,
            is_match,
            haystack,
        })
    }

    /// Checks if the given input matches the regular expression.
    pub fn is_match(&mut self, input: regex_automata::Input<'_>) -> bool {
        common_input_validation(&input);

        let haystack = input.haystack();
        let _success = self
            .prepare_input
            .call(&mut self.executor.store, haystack.len().try_into().unwrap())
            .expect("execution should not trap");

        self.haystack.data_mut(&mut self.executor.store)[0..haystack.len()]
            .copy_from_slice(haystack);

        let input_opts = InputOpts::new(&input);

        let is_match_result = self
            .is_match
            .call(
                &mut self.executor.store,
                (
                    input_opts.anchored,
                    input_opts.anchored_pattern,
                    i64::from_ne_bytes(
                        u64::try_from(input.get_span().start).unwrap().to_ne_bytes(),
                    ),
                    i64::from_ne_bytes(u64::try_from(input.get_span().end).unwrap().to_ne_bytes()),
                    i64::from_ne_bytes(u64::try_from(haystack.len()).unwrap().to_ne_bytes()),
                ),
            )
            .expect("execution should not trap");

        if is_match_result == (true as i32) {
            true
        } else if is_match_result == (false as i32) {
            false
        } else {
            panic!("unexpected value from is_match: {is_match_result}");
        }
    }
}

#[cfg(test)]
mod tests {
    use regex_automata::Input;

    use crate::Builder;

    use super::*;

    #[test]
    fn empty_pattern_empty_haystack() {
        let (bytecode, _) = Builder::new().build_many::<&str>(&[]).unwrap();
        let mut regex = Regex::new(&bytecode).unwrap();
        assert!(!regex.is_match(Input::new("")));
    }
}
