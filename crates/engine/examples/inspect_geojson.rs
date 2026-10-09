//! Read-only topology diagnostics for user-supplied GeoJSON; never repairs coordinates.
use geo::Validation;
use serde_json::{Value, json};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect_geojson FILE")?;
    let source = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&source)?;
    let features = root["features"]
        .as_array()
        .ok_or("expected FeatureCollection")?;
    let mut invalid = 0;
    for (index, feature) in features.iter().enumerate() {
        let geometry: geojson::Geometry = serde_json::from_value(feature["geometry"].clone())?;
        let geometry: geo::Geometry<f64> = geometry.value.try_into()?;
        if let Err(error) = geometry.check_validation() {
            invalid += 1;
            println!(
                "{}",
                json!({"feature":index+1,"id":feature.get("id"),"reason":error.to_string()})
            );
        }
    }
    println!("{}", json!({"features":features.len(),"invalid":invalid}));
    Ok(())
}
