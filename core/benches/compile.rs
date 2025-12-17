use gungraun::{library_benchmark, library_benchmark_group, main};
use wahgex::{Builder, RegexBytecode};

#[library_benchmark]
#[bench::empty_regex("")]
#[bench::simple_repetition("(?:abc)+")]
#[bench::sparse_transitions("a|b|d|e|g")]
#[bench::simple_lookaround("^hell worm$")]
#[bench::repeated_lookaround("(?:^|$)+")]
#[bench::lookaround_crlf("(?mR)^[a-z]+$")]
#[bench::lookaround_is_word_ascii(r"(?-u)hello\b")]
#[bench::lookaround_is_word_unicode(r"(?u)hello\b")]
fn bench_compile(pattern: &'static str) -> RegexBytecode {
    Builder::new().build(pattern).unwrap().0
}

library_benchmark_group!(
    name = bench_compile_group;
    benchmarks = bench_compile
);

main!(library_benchmark_groups = bench_compile_group);
