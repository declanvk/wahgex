use wahgex::{Builder, Input, engines::wasmtime::Regex};

#[test]
fn test_wasmtime_match_sherlock() {
    let haystack = include_str!("../../testdata/haystacks/sherlock.txt");
    let (bytecode, _) = Builder::new().build("Irene Adler").unwrap();
    let mut regex = Regex::new(&bytecode).unwrap();
    let input = Input::new(haystack);
    assert!(regex.is_match(input));
}
