use common::configure_pikevm_builder;
use regex_automata::Input;
use regex_test::{
    anyhow::{self, bail, Context},
    CompiledRegex, RegexTest, TestResult, TestRunner,
};
use wahgex_core::{Builder, InputOpts, PikeVM, PrepareInputResult};

mod common;

/// Tests the default configuration of the hybrid NFA/DFA.
#[test]
fn default() -> anyhow::Result<()> {
    const TEST_DENYLIST: &[&str] = &[];

    let builder = PikeVM::builder();
    let mut runner = TestRunner::new()?;
    runner.expand(&["is_match"], |test| test.compiles());
    runner
        .test_iter(
            common::suite()?
                .iter()
                .filter(|test| !TEST_DENYLIST.contains(&test.name())),
            compiler(builder),
        )
        .assert();
    Ok(())
}

/// Configure a regex_automata::Input with the given test configuration.
fn create_input(test: &regex_test::RegexTest) -> regex_automata::Input<'_> {
    use regex_automata::Anchored;

    let bounds = test.bounds();
    let anchored = if test.anchored() {
        Anchored::Yes
    } else {
        Anchored::No
    };
    regex_automata::Input::new(test.haystack())
        .range(bounds.start..bounds.end)
        .anchored(anchored)
}

fn compiler(
    mut builder: Builder,
) -> impl FnMut(&RegexTest, &[String]) -> anyhow::Result<CompiledRegex> {
    move |test, regexes| {
        if !configure_pikevm_builder(test, &mut builder) {
            return Ok(CompiledRegex::skip());
        }

        let re = match builder.build_many(regexes) {
            Ok(re) => re,
            Err(err) if err.is_unsupported() => {
                return Ok(CompiledRegex::skip());
            },
            Err(err) => {
                return Err(err.into());
            },
        };

        Ok(CompiledRegex::compiled(move |test| -> TestResult {
            run_test(&re, test)
        }))
    }
}

fn run_test(re: &PikeVM, test: &RegexTest) -> TestResult {
    let input = create_input(test);
    match test.additional_name() {
        "is_match" => run_is_match(re, input)
            .unwrap_or_else(|err| TestResult::fail(format!("{err:?}").as_str())),
        name => TestResult::fail(&format!("unrecognized test name: {name}")),
    }
}

fn run_is_match(re: &PikeVM, input: Input<'_>) -> anyhow::Result<TestResult> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, re.get_wasm()).context("compile module")?;
    let mut store = wasmi::Store::new(&engine, ());
    let linker = wasmi::Linker::<()>::new(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .context("instantiate module")?
        .start(&mut store)
        .context("run module start")?;
    let haystack_memory = instance.get_memory(&store, "haystack").unwrap();

    let prepare_input = instance
        .get_typed_func::<i64, i32>(&store, "prepare_input")
        .context("get prepare_input fn")?;

    let is_match = instance
        // [anchored, anchored_pattern, span_start, span_end, haystack_len]
        .get_typed_func::<(i32, i32, i64, i64, i64), i32>(&store, "is_match")
        .context("get is_match fn")?;

    let success = prepare_input
        .call(&mut store, input.haystack().len().try_into().unwrap())
        .context("call prepare_input")?;
    if success == (PrepareInputResult::Failure as i32) {
        bail!("prepare_input failed")
    }

    haystack_memory.data_mut(&mut store)[0..input.haystack().len()]
        .copy_from_slice(input.haystack());

    let input_opts = InputOpts::new(&input);
    let is_match_result = is_match.call(
        &mut store,
        (
            input_opts.anchored,
            input_opts.anchored_pattern,
            i64::from_ne_bytes(u64::try_from(input.get_span().start).unwrap().to_ne_bytes()),
            i64::from_ne_bytes(u64::try_from(input.get_span().end).unwrap().to_ne_bytes()),
            i64::from_ne_bytes(u64::try_from(input.haystack().len()).unwrap().to_ne_bytes()),
        ),
    )?;

    let is_match_result = if is_match_result == (true as i32) {
        true
    } else if is_match_result == (false as i32) {
        false
    } else {
        bail!("unexpected value from is_match: {is_match_result}")
    };

    Ok(TestResult::matched(is_match_result))
}
