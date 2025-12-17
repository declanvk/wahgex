//! Utilities used to run [`wahgex`][crate] compiled regular expressions
//! using [`wasmi`].

use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::{RegexBytecode, common_input_validation, input::InputOpts};

#[derive(Debug)]
pub(crate) struct Executor {
    _engine: Engine,
    _module: Module,
    store: Store<()>,
    instance: Instance,
}

impl Executor {
    /// TODO: Write docs for this item
    pub fn with_engine(engine: Engine, bytecode: &RegexBytecode) -> Result<Self, wasmi::Error> {
        let module = Module::new(&engine, bytecode)?;
        let mut store = Store::new(&engine, ());
        let linker = Linker::<()>::new(&engine);
        let instance = linker
            .instantiate(&mut store, &module)
            .unwrap()
            .start(&mut store)
            .unwrap();

        Ok(Self {
            _engine: engine,
            _module: module,
            store,
            instance,
        })
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub(crate) fn instance(&self) -> &Instance {
        &self.instance
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub(crate) fn store(&self) -> &Store<()> {
        &self.store
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub(crate) fn store_mut(&mut self) -> &mut Store<()> {
        &mut self.store
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct Regex {
    executor: Executor,
    prepare_input: TypedFunc<i64, i32>,
    is_match: TypedFunc<(i32, i32, i64, i64, i64), i32>,
    haystack: Memory,
}

impl Regex {
    /// TODO: Write docs for this item
    pub fn new(bytecode: &RegexBytecode) -> Result<Self, wasmi::Error> {
        Self::with_engine(Engine::default(), bytecode)
    }

    /// TODO: Write docs for this item
    ///
    /// # Panics
    ///
    /// TODO: Write docs for this item
    pub fn with_engine(engine: Engine, bytecode: &RegexBytecode) -> Result<Self, wasmi::Error> {
        let executor = Executor::with_engine(engine, bytecode)?;

        let prepare_input = executor
            .instance
            .get_typed_func::<i64, i32>(&executor.store, "prepare_input")
            .expect(
                "If the `RegexBytecode` passed is well-formed, then there must be a \
                 `prepare_input` function",
            );
        let is_match = executor
            .instance
            // [anchored, anchored_pattern, span_start, span_end, haystack_len]
            .get_typed_func::<(i32, i32, i64, i64, i64), i32>(&executor.store, "is_match")
            .expect(
                "If the `RegexBytecode` passed is well-formed, then there must be a `is_match` \
                 function",
            );
        let haystack: Memory = executor
            .instance
            .get_memory(&executor.store, "haystack")
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

    /// TODO: Write docs for this item
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
