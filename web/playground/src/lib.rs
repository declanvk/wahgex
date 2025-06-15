use wahgex_core::Builder;
use wasm_bindgen::prelude::*;
use wasmprinter::print_bytes;

#[wasm_bindgen]
pub struct CompileResult {
    wasm_bytes: Box<[u8]>,
    module_size: usize,
    states: usize,
    pattern_len: usize,
    has_capture: bool,
    has_empty: bool,
    is_utf8: bool,
    is_reverse: bool,
    lookset_any: String,
    lookset_prefix_any: String,
    wat_string: String,
}

#[wasm_bindgen]
pub fn compile(regex: String) -> Result<CompileResult, String> {
    let regex_vm = Builder::new()
        .build(&regex)
        .map_err(|err| err.to_string())?;

    let wasm_bytes = regex_vm.get_wasm();
    let wat_string = print_bytes(wasm_bytes).map_err(|err| err.to_string())?;

    let nfa = regex_vm.get_nfa();

    let result = CompileResult {
        wasm_bytes: wasm_bytes.into(),
        module_size: wasm_bytes.len(),
        states: nfa.states().len(),
        pattern_len: nfa.pattern_len(),
        has_capture: nfa.has_capture(),
        has_empty: nfa.has_empty(),
        is_utf8: nfa.is_utf8(),
        is_reverse: nfa.is_reverse(),
        lookset_any: format!("{:?}", nfa.look_set_any()),
        lookset_prefix_any: format!("{:?}", nfa.look_set_prefix_any()),
        wat_string,
    };

    Ok(result)
}

#[wasm_bindgen]
impl CompileResult {
    #[wasm_bindgen(getter)]
    pub fn wasm_bytes(&self) -> Box<[u8]> {
        self.wasm_bytes.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn module_size(&self) -> usize {
        self.module_size
    }

    #[wasm_bindgen(getter)]
    pub fn states(&self) -> usize {
        self.states
    }

    #[wasm_bindgen(getter)]
    pub fn pattern_len(&self) -> usize {
        self.pattern_len
    }

    #[wasm_bindgen(getter)]
    pub fn has_capture(&self) -> bool {
        self.has_capture
    }

    #[wasm_bindgen(getter)]
    pub fn has_empty(&self) -> bool {
        self.has_empty
    }

    #[wasm_bindgen(getter)]
    pub fn is_utf8(&self) -> bool {
        self.is_utf8
    }

    #[wasm_bindgen(getter)]
    pub fn is_reverse(&self) -> bool {
        self.is_reverse
    }

    #[wasm_bindgen(getter)]
    pub fn lookset_any(&self) -> String {
        self.lookset_any.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn lookset_prefix_any(&self) -> String {
        self.lookset_prefix_any.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn wat_string(&self) -> String {
        self.wat_string.clone()
    }
}
