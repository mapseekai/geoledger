//! GeoJSON geometry validation, normalization, bounds and bbox tests.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| geoledger_engine::fuzzing::geometry(data));
