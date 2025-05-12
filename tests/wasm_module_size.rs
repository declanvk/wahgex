use std::{collections::HashMap, fmt::Write};

use integration::configure_pikevm_builder;
use regex_test::{
    anyhow::{self, Context},
    RegexTest,
};
use wahgex::{Builder, PikeVM};

mod integration;

#[derive(Debug)]
struct WasmSizeResult {
    size: usize,
    name: String,
}

#[test]
fn wasm_module_size_of() {
    let suite = integration::suite().unwrap();

    let mut groups: HashMap<&str, Vec<WasmSizeResult>> = HashMap::new();
    let builder = PikeVM::builder();

    for test in suite.iter() {
        if !test.compiles() {
            continue;
        }

        let reg = match compile(builder.clone(), test, test.regexes())
            .context(format!("compiling regex [{}]", test.full_name()))
            .unwrap()
        {
            CompileOutput::Skip => continue,
            CompileOutput::Compiled(reg) => reg,
        };

        groups
            .entry(test.group())
            .or_default()
            .push(WasmSizeResult {
                size: reg.get_wasm().len(),
                name: test.name().into(),
            });
    }

    let formatted = format_grouped_results(groups).unwrap();

    insta::assert_snapshot!(formatted);
}

fn format_grouped_results(
    mut groups: HashMap<&str, Vec<WasmSizeResult>>,
) -> anyhow::Result<String> {
    let mut formatted = String::new();
    let mut group_names: Vec<_> = groups.keys().copied().collect();
    group_names.sort();

    for group in group_names {
        let mut results = groups.remove(group).unwrap();
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
