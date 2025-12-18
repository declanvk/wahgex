use std::{env::current_dir, fs::OpenOptions, io::Read, sync::LazyLock};

use gungraun::{library_benchmark, library_benchmark_group, main};
use wahgex::{Builder, Input, engines::wasmi::Regex};

fn read_haystack_from_path(path: &str) -> String {
    eprintln!(
        "Reading haystack at [{path}], current working directory is [{}]...",
        current_dir().unwrap().display()
    );
    let mut haystack_file = OpenOptions::new().read(true).open(path).unwrap();
    let mut buf = String::new();
    let _haystack = haystack_file.read_to_string(&mut buf).unwrap();
    buf
}

fn cache_read_haystack(name: &'static str) -> &'static str {
    match name {
        "sherlock" => {
            static SHERLOCK: LazyLock<String> =
                LazyLock::new(|| read_haystack_from_path("benches/haystacks/sherlock.txt"));
            &SHERLOCK
        },
        unknown => panic!("Unknown haystack [{unknown}]"),
    }
}

fn compile_passthrough_haystack(
    pattern: &'static str,
    haystack_name: &'static str,
) -> (Regex, Input<'static>) {
    let haystack = cache_read_haystack(haystack_name);
    let bytecode = Builder::new().build(pattern).unwrap().0;
    let regex = Regex::new(&bytecode).unwrap();
    (regex, Input::new(haystack))
}

#[library_benchmark(setup = compile_passthrough_haystack)]
#[bench::literal(r"Sherlock Holmes", "sherlock")]
#[bench::literal_prefix(r"Sherlock\s+\w+", "sherlock")]
#[bench::literal_suffix(r"\w+\s+Holmes", "sherlock")]
#[bench::unicode_boundary(r"\bSherlock\b", "sherlock")]
#[bench::ascii_boundary(r"(?-u)\bSherlock\b", "sherlock")]
fn bench_is_match((mut regex, haystack): (Regex, Input<'static>)) -> bool {
    regex.is_match(haystack)
}

library_benchmark_group!(
    name = bench_wasmi_group;
    benchmarks = bench_is_match
);

main!(library_benchmark_groups = bench_wasmi_group);
