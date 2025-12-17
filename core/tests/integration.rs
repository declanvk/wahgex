use common::configure_pikevm_builder;
use regex_automata::Input;
use regex_test::{
    CompiledRegex, RegexTest, TestResult, TestRunner,
    anyhow::{self, Context},
};
use wahgex::{Builder, RegexBytecode, RegexContext, engines::wasmi::Regex};

mod common;

/// Tests the default configuration of the hybrid NFA/DFA.
#[test]
fn default() -> anyhow::Result<()> {
    const TEST_DENYLIST: &[&str] = &[];

    let builder = RegexContext::builder();
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

        let (bytecode, _context) = match builder.build_many(regexes) {
            Ok(re) => re,
            Err(err) => {
                return Err(err.into());
            },
        };

        Ok(CompiledRegex::compiled(move |test| -> TestResult {
            run_test(&bytecode, test)
        }))
    }
}

fn run_test(bytecode: &RegexBytecode, test: &RegexTest) -> TestResult {
    let input = create_input(test);
    match test.additional_name() {
        "is_match" => run_is_match(bytecode, input)
            .unwrap_or_else(|err| TestResult::fail(format!("{err:?}").as_str())),
        name => TestResult::fail(&format!("unrecognized test name: {name}")),
    }
}

fn run_is_match(bytecode: &RegexBytecode, input: Input<'_>) -> anyhow::Result<TestResult> {
    let mut regex = Regex::new(bytecode).context("compile module")?;
    let is_match_result = regex.is_match(input);

    Ok(TestResult::matched(is_match_result))
}
