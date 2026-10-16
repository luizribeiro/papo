//! Runs the `papo-core` browser probes in a dedicated worker.

use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

include!("support/suite.rs");
