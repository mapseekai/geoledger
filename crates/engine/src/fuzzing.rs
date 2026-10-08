//! Fuzz entry points for the untrusted-input decoders (`fuzz/`, cargo-fuzz) and a
//! deterministic smoke run in the unit tests. Each function accepts any bytes and
//! panics only when an invariant breaks, which the fuzzer reports as a finding.
use crate::{codec, geometry};

/// Exact-number JSON codec: accepted input re-encodes to JSON that the codec
/// accepts again unchanged, and stored values decode the same way.
pub fn codec(data: &[u8]) {
    let Ok(value) = codec::parse(data) else {
        return;
    };
    let Ok(encoded) = codec::encode(&value) else {
        return; // larger than the response limit
    };
    match codec::parse(&encoded) {
        Ok(again) => assert_eq!(again, value, "codec re-encoding changed the value"),
        Err(e) => panic!("codec rejected its own encoding: {e}"),
    }
    let Ok(text) = std::str::from_utf8(&encoded) else {
        panic!("codec produced non-UTF-8 JSON");
    };
    match codec::stored::<serde_json::Value>(text) {
        Ok(stored) => assert_eq!(stored, value, "stored decoding differs"),
        Err(e) => panic!("stored decoding rejected codec output: {e}"),
    }
}

/// GeoJSON geometry validation: accepted geometry is stored verbatim, is stable
/// under re-validation, and its stored form always yields bounds and bbox tests.
pub fn geometry(data: &[u8]) {
    let Ok(value) = codec::parse(data) else {
        return;
    };
    let Ok(Some(normalized)) = geometry::normalize(&value) else {
        return;
    };
    match codec::parse(normalized.as_bytes()) {
        Ok(again) => match geometry::normalize(&again) {
            Ok(Some(twice)) => assert_eq!(twice, normalized, "normalization is not stable"),
            other => panic!("accepted geometry rejected on re-validation: {other:?}"),
        },
        Err(e) => panic!("normalized geometry is not codec JSON: {e}"),
    }
    if let Err(e) = geometry::bounds(&normalized) {
        panic!("bounds failed for stored geometry: {e}");
    }
    for bbox in [[-180.0, -90.0, 180.0, 90.0], [0.0, 0.0, 0.0, 0.0]] {
        if let Err(e) = geometry::intersects(&normalized, bbox) {
            panic!("bbox test failed for stored geometry: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// Deterministic xorshift so the smoke run is reproducible without a dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }
    const SEEDS: &[&str] = &[
        r#"{"a":18446744073709551615,"b":-9223372036854775808,"c":0.1,"d":1e2,"e":"x"}"#,
        r#"[9007199254740993,90071992547409930e-1,7.93377316861183e-76,-0,1.5,[{"k":null}]]"#,
        r#"{"type":"Point","coordinates":[120.5,30.25,12]}"#,
        r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,0.5]]}"#,
        r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]],[[0.2,0.2],[0.2,0.4],[0.4,0.4],[0.2,0.2]]]}"#,
        r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[2,2],[3,2],[3,3],[2,2]]]]}"#,
        r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}]}"#,
        r#"{"type":"MultiLineString","coordinates":[[[179.9,-89.9],[-179.9,89.9]]]}"#,
    ];
    const TOKENS: &[&[u8]] = &[
        b"0",
        b"-",
        b".",
        b"e",
        b"E+",
        b"9",
        b"[",
        b"]",
        b"{",
        b"}",
        b",",
        b":",
        b"\"",
        b"\\u0000",
        b"1e400",
        b"18446744073709551616",
        b"null",
        b"\xff",
        b" ",
        b"\"type\":\"Point\"",
        b"[[0,0],[1,1]]",
    ];
    #[test]
    fn fuzz_harnesses_hold_on_mutated_seeds() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for seed in SEEDS {
            super::codec(seed.as_bytes());
            super::geometry(seed.as_bytes());
        }
        for _ in 0..20_000 {
            let mut input = SEEDS[rng.below(SEEDS.len())].as_bytes().to_vec();
            for _ in 0..=rng.below(4) {
                let at = rng.below(input.len() + 1);
                match rng.below(4) {
                    0 if at < input.len() => input[at] = (rng.next() & 0xff) as u8,
                    1 => {
                        let token = TOKENS[rng.below(TOKENS.len())];
                        input.splice(at..at, token.iter().copied());
                    }
                    2 if at < input.len() => {
                        let end = (at + 1 + rng.below(8)).min(input.len());
                        input.drain(at..end);
                    }
                    _ => input.truncate(at),
                }
            }
            super::codec(&input);
            super::geometry(&input);
        }
    }
}
