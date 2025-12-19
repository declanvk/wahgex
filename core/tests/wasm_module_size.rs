use std::{collections::HashMap, fmt::Write};

use common::configure_pikevm_builder;
use rayon::iter::{ParallelBridge, ParallelIterator};
use regex_test::{
    RegexTest,
    anyhow::{self, Context},
};
use wahgex::{Builder, RegexBytecode, RegexContext};

mod common;

#[derive(Debug)]
struct WasmSizeResult {
    group: String,
    size_with_names: usize,
    size_without_names: usize,
    name: String,
}

#[test]
fn wasm_module_size_of() {
    let suite = common::suite().unwrap();

    let all_regexes = suite
        .iter()
        .par_bridge()
        .filter_map(|test| {
            if !test.compiles() {
                return None;
            }

            let (with_names, without_names) =
                match compile(RegexContext::builder(), test, test.regexes())
                    .context(format!("compiling regex [{}]", test.full_name()))
                    .unwrap()
                {
                    CompileOutput::Skip => {
                        return None;
                    },
                    CompileOutput::Compiled {
                        with_names,
                        without_names,
                    } => (with_names, without_names),
                };

            Some(WasmSizeResult {
                group: test.group().into(),
                size_with_names: with_names.as_ref().len(),
                size_without_names: without_names.as_ref().len(),
                name: test.name().into(),
            })
        })
        .collect::<Vec<_>>();

    let mut groups: HashMap<_, Vec<WasmSizeResult>> = HashMap::new();

    for result in all_regexes {
        groups.entry(result.group.clone()).or_default().push(result);
    }

    let formatted = format_grouped_results(groups).unwrap();

    insta::assert_snapshot!(formatted);
}

fn format_grouped_results(
    mut groups: HashMap<String, Vec<WasmSizeResult>>,
) -> anyhow::Result<String> {
    let mut formatted = String::new();
    let mut group_names: Vec<_> = groups.keys().cloned().collect();
    group_names.sort();

    for group in group_names {
        let mut results = groups.remove(&group).unwrap();
        writeln!(&mut formatted, "[{group}]")?;
        results.sort_by(|a, b| a.name.cmp(&b.name));
        let max_name_len = results.iter().map(|res| res.name.len()).max().unwrap_or(0);
        for res in results {
            writeln!(
                &mut formatted,
                "{name:<width$} = {size_with_names}/{size_without_names}",
                name = res.name,
                width = max_name_len,
                size_with_names = res.size_with_names,
                size_without_names = res.size_without_names,
            )?;
        }
        writeln!(&mut formatted)?;
    }
    Ok(formatted)
}

#[derive(Debug)]
enum CompileOutput {
    Skip,
    Compiled {
        with_names: RegexBytecode,
        without_names: RegexBytecode,
    },
}

fn compile(
    mut builder: Builder,
    test: &RegexTest,
    regexes: &[String],
) -> anyhow::Result<CompileOutput> {
    if !configure_pikevm_builder(test, &mut builder) {
        return Ok(CompileOutput::Skip);
    }

    let (with_names, _) = match builder
        .configure(builder.get_config().clone().include_names(true))
        .build_many(regexes)
    {
        Ok(re) => re,
        Err(err) => {
            return Err(err.into());
        },
    };

    let (without_names, _) = match builder
        .configure(builder.get_config().clone().include_names(false))
        .build_many(regexes)
    {
        Ok(re) => re,
        Err(err) => {
            return Err(err.into());
        },
    };

    Ok(CompileOutput::Compiled {
        with_names,
        without_names,
    })
}
