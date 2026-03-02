//! This module contains other modules related to running [`wahgex`][crate]
//! on specific WASM engines.

type IsMatchArgs = (i32, i32, i64, i64, i64);

#[cfg(feature = "wasmi")]
pub mod wasmi;
#[cfg(feature = "wasmtime")]
pub mod wasmtime;
