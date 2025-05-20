use std::{env, iter};

use wahgex_core::PikeVM;

fn main() {
    let input = env::args().nth(1).unwrap();

    let regex = wahgex_core::PikeVM::new(&input).unwrap();

    eprint_input_info(&input, &regex);

    let pretty_wasm = wasm_print_module(&regex.get_wasm());

    println!("{}", pretty_wasm);
}

fn eprint_input_info(input: &str, regex: &PikeVM) {
    eprint_fields(&[
        ("input", input.into()),
        ("module size", regex.get_wasm().len().to_string()),
        ("states", regex.get_nfa().states().len().to_string()),
        ("pattern len", regex.get_nfa().pattern_len().to_string()),
        ("has capture?", regex.get_nfa().has_capture().to_string()),
        ("has empty?", regex.get_nfa().has_empty().to_string()),
        ("is utf8?", regex.get_nfa().is_utf8().to_string()),
        ("is reverse?", regex.get_nfa().is_reverse().to_string()),
        (
            "lookset any",
            format!("{:?}", regex.get_nfa().look_set_any()),
        ),
        (
            "lookset prefix any",
            format!("{:?}", regex.get_nfa().look_set_prefix_any()),
        ),
        (
            "lookset prefix any",
            format!("{:?}", regex.get_nfa().look_set_prefix_any()),
        ),
    ]);
}

fn eprint_fields(fields: &[(&str, String)]) {
    let max_name_len = fields.iter().map(|(name, _)| name.len()).max().unwrap_or(0);

    for (name, value) in fields {
        let pad: String = iter::repeat_n(' ', max_name_len - name.len() + 1).collect();
        eprintln!("{pad}{name}:  {value}");
    }
}

#[track_caller]
pub fn wasm_print_module(module_bytes: impl AsRef<[u8]>) -> String {
    let module_bytes = module_bytes.as_ref();
    let wasm_text = wasmprinter::print_bytes(module_bytes);
    if let Err(err) = wasmparser::validate(module_bytes) {
        let mut wasm_text_with_offsets = String::new();
        let print = wasmprinter::Config::new().print_offsets(true).print(
            module_bytes,
            &mut wasmprinter::PrintFmtWrite(&mut wasm_text_with_offsets),
        );

        match print {
            Ok(()) => {
                panic!("{err}:\n{wasm_text_with_offsets}")
            },
            Err(print_err) => panic!("{err}:\nUnable to print WAT: {print_err}"),
        }
    }
    wasm_text.expect("should be able to print WASM module in WAT format")
}
