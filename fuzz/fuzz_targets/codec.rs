//! Exact-number JSON codec used for every request body and stored value.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| geoledger_engine::fuzzing::codec(data));
