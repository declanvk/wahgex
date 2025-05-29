use wahgex_core::Builder;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn compile(regex: String) -> Result<Box<[u8]>, String> {
    let regex = Builder::new()
        .build(&regex)
        .map_err(|err| err.to_string())?;

    Ok(Box::from(regex.get_wasm()))
}
