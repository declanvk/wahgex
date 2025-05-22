use std::{collections::HashMap, fmt::Write};

use common::configure_pikevm_builder;
use rayon::iter::{ParallelBridge, ParallelIterator};
use regex_test::{
    anyhow::{self, Context},
    RegexTest,
};
use wahgex_core::{Builder, PikeVM};

mod common;

#[derive(Debug)]
struct WasmSizeResult {
    group: String,
    size: usize,
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

            let reg = match compile(PikeVM::builder(), test, test.regexes())
                .context(format!("compiling regex [{}]", test.full_name()))
                .unwrap()
            {
                CompileOutput::Skip => {
                    return None;
                },
                CompileOutput::Compiled(reg) => reg,
            };

            Some(WasmSizeResult {
                group: test.group().into(),
                size: reg.get_wasm().len(),
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
                "{name:<width$} = {size}",
                name = res.name,
                width = max_name_len,
                size = res.size
            )?;
        }
        writeln!(&mut formatted)?;
    }
    Ok(formatted)
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

    let re = match builder.build_many(regexes) {
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
