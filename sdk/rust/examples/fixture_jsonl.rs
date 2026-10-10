//! Prepare bounded-memory, disposable load-test fixtures; never opens a database.
#[path = "large_data/input.rs"]
mod input;
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
};

fn geometry_numbers(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(n) = n.as_f64().and_then(serde_json::Number::from_f64) {
                *value = Value::Number(n);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(geometry_numbers),
        Value::Object(items) => items.values_mut().for_each(geometry_numbers),
        _ => {}
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let source = args.get(1).ok_or("usage: fixture_jsonl SOURCE OUTPUT")?;
    let output = args.get(2).ok_or("output required")?;
    let mut output = BufWriter::new(File::options().write(true).create_new(true).open(output)?);
    let mut count = 0u64;
    let mut shape = None;
    let mut hash = sha2::Sha256::new();
    input::read(BufReader::new(File::open(source)?), &mut |value| {
        count += 1;
        let mut feature = input::feature(value, count)?;
        if shape.is_none() {
            shape = Some(input::shape(&feature)?);
        }
        geometry_numbers(&mut feature["geometry"]);
        feature["properties"] = json!({"attributes":feature["properties"].take()});
        input::digest(&mut hash, &feature).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut output, &feature).map_err(|e| e.to_string())?;
        output.write_all(b"\n").map_err(|e| e.to_string())
    })?;
    output.flush()?;
    let (family, dimension) = shape.ok_or("empty fixture")?;
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!(
        "{}",
        json!({"features":count,"geometry_type":family,"dimension":dimension,"sha256":digest})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_geometry_without_touching_properties() {
        let mut f =
            json!({"geometry":{"coordinates":[1,2,3]},"properties":{"n":18446744073709551615u64}});
        geometry_numbers(&mut f["geometry"]);
        assert_eq!(f["geometry"]["coordinates"], json!([1.0, 2.0, 3.0]));
        assert_eq!(f["properties"]["n"].as_u64(), Some(u64::MAX));
    }
}
