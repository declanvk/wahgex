use wahgex::{Builder, Input, engines::wasmi::Regex};

#[test]
fn test_wasmi_match_sherlock() {
    let haystack = include_str!("../../testdata/haystacks/sherlock.txt");
    let (bytecode, _) = Builder::new().build("Irene Adler").unwrap();
    let mut regex = Regex::new(&bytecode).unwrap();
    let input = Input::new(haystack);
    assert!(regex.is_match(input));
}
