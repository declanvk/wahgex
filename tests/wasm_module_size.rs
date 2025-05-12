use std::collections::HashMap;

use integration::configure_pikevm_builder;
use regex_test::{
    anyhow::{self, Context},
    RegexTest,
};
use wahgex::{Builder, PikeVM};

mod integration;

#[test]
fn wasm_module_size_of() {
    let suite = integration::suite().unwrap();

    let mut lines = Vec::new();
    let max_lens = test_group_max_name_length(&suite);
    let builder = PikeVM::builder();

    for test in suite.iter() {
        if !test.compiles() {
            continue;
        }

        let max_len_test_name = max_lens.get(test.group()).unwrap();
        let name = test.full_name();
        let pad = " ".repeat(max_len_test_name - name.len());
        let reg = match compile(builder.clone(), test, test.regexes())
            .context(format!("compiling regex [{}]", name))
            .unwrap()
        {
            CompileOutput::Skip => continue,
            CompileOutput::Compiled(reg) => reg,
        };

        lines.push((name, format!("{name}{pad}:{}", reg.get_wasm().len())))
    }

    lines.sort_by(|a, b| a.0.cmp(b.0));
    let lines = lines
        .into_iter()
        .map(|(_, content)| content)
        .collect::<Vec<_>>();

    insta::assert_snapshot!(lines.join("\n"));
}

#[derive(Debug)]
enum CompileOutput {
    Skip,
    Compiled(PikeVM),
}

fn compile(
    mut builder: Builder,
    test: &RegexTest,
    regexes: &[String],
) -> anyhow::Result<CompileOutput> {
    if !configure_pikevm_builder(test, &mut builder) {
        return Ok(CompileOutput::Skip);
    }

    let re = match builder.build_many(&regexes) {
        Ok(re) => re,
        Err(err) if err.is_unsupported() => {
            return Ok(CompileOutput::Skip);
        },
        Err(err) => {
            return Err(err.into());
        },
    };

    Ok(CompileOutput::Compiled(re))
}

fn test_group_max_name_length(suite: &regex_test::RegexTests) -> HashMap<&str, usize> {
    let mut max_lens: HashMap<&str, usize> = HashMap::new();
    for test in suite.iter() {
        if !test.compiles() {
            continue;
        }

        max_lens
            .entry(test.group())
            .and_modify(|len| {
                *len = (*len).max(test.full_name().len());
            })
            .or_insert(test.full_name().len());
    }
    max_lens
}
