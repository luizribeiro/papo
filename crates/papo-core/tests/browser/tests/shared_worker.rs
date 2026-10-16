//! Runs the `papo-core` browser probes in a shared worker.

use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_shared_worker);

include!("support/suite.rs");
