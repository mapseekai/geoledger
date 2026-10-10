//! Streaming FeatureCollection reader; at most one feature plus one bounded batch is resident.
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use sha2::Digest;
use std::{fmt, io::Read};

pub fn read<R: Read, F: FnMut(Value) -> Result<(), String>>(
    input: R,
    callback: &mut F,
) -> Result<(), serde_json::Error> {
    let mut decoder = serde_json::Deserializer::from_reader(input);
    Collection(callback).deserialize(&mut decoder)?;
    decoder.end()
}
struct Collection<'a, F>(&'a mut F);
impl<'de, F: FnMut(Value) -> Result<(), String>> DeserializeSeed<'de> for Collection<'_, F> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de, F: FnMut(Value) -> Result<(), String>> Visitor<'de> for Collection<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("GeoJSON FeatureCollection")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut kind = None;
        let mut features = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" if kind.is_none() => kind = Some(map.next_value::<String>()?),
                "features" if !features => {
                    features = true;
                    map.next_value_seed(Features(self.0))?;
                }
                "features" | "type" => return Err(serde::de::Error::custom("duplicate field")),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if kind.as_deref() != Some("FeatureCollection") || !features {
            return Err(serde::de::Error::custom(
                "expected FeatureCollection with features",
            ));
        }
        Ok(())
    }
}
struct Features<'a, F>(&'a mut F);
impl<'de, F: FnMut(Value) -> Result<(), String>> DeserializeSeed<'de> for Features<'_, F> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de, F: FnMut(Value) -> Result<(), String>> Visitor<'de> for Features<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("feature array")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        while let Some(feature) = seq.next_element::<Value>()? {
            (self.0)(feature).map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

pub fn feature(mut value: Value, index: u64) -> Result<Value, String> {
    if value["type"] != "Feature" || !value["properties"].is_object() {
        return Err(format!(
            "feature {index}: expected Feature with object properties"
        ));
    }
    // Benchmark IDs sort in input order, allowing a bounded-memory full readback digest.
    value["id"] = Value::String(format!("{index:012}"));
    let object = value.as_object_mut().ok_or("feature object")?;
    object.retain(|key, _| ["type", "id", "properties", "geometry"].contains(&key.as_str()));
    Ok(value)
}

pub fn shape(value: &Value) -> Result<(&'static str, u32), String> {
    let geometry = &value["geometry"];
    let family = match geometry["type"].as_str() {
        Some("Point" | "MultiPoint") => "point",
        Some("LineString" | "MultiLineString") => "line",
        Some("Polygon" | "MultiPolygon") => "polygon",
        _ => return Err("benchmark expects a typed point, line or polygon geometry".into()),
    };
    let mut coords = &geometry["coordinates"];
    while coords
        .as_array()
        .is_some_and(|a| a.first().is_some_and(Value::is_array))
    {
        coords = &coords[0];
    }
    let n = coords.as_array().ok_or("coordinates")?.len();
    if ![2, 3].contains(&n) {
        return Err("expected XY or XYZ coordinates".into());
    }
    Ok((family, n as u32))
}

pub fn digest(hasher: &mut sha2::Sha256, value: &Value) -> Result<(), serde_json::Error> {
    fn coordinates(value: &mut Value) {
        match value {
            Value::Number(n) => {
                if let Some(n) = n.as_f64().and_then(serde_json::Number::from_f64) {
                    *value = Value::Number(n);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(coordinates),
            Value::Object(o) => o.values_mut().for_each(coordinates),
            _ => {}
        }
    }
    fn property_numbers(value: &mut Value) {
        match value {
            Value::Number(n) if n.as_i64().is_none() && n.as_u64().is_none() => {
                if let Some(f) = n.as_f64() {
                    if f.fract() == 0.0 && f.abs() < 9007199254740992.0 {
                        *value = Value::from(f as i64);
                    } else if let Some(n) = serde_json::Number::from_f64(f) {
                        *value = Value::Number(n);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(property_numbers),
            Value::Object(o) => o.values_mut().for_each(property_numbers),
            _ => {}
        }
    }
    let mut canonical = value.clone();
    coordinates(&mut canonical["geometry"]);
    property_numbers(&mut canonical["properties"]);
    let bytes = serde_json::to_vec(&canonical)?;
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_pretty_json_and_rejects_truncation_duplicate_and_trailing_input() {
        let source = r#"{"features":[{"type":"Feature","properties":{"n":18446744073709551615},"geometry":{"type":"Point","coordinates":[1,2]}}],"type":"FeatureCollection"}"#;
        let mut rows = Vec::new();
        assert!(
            read(source.as_bytes(), &mut |v| {
                rows.push(v);
                Ok(())
            })
            .is_ok()
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["properties"]["n"].as_u64(), Some(u64::MAX));
        for bad in [
            &source[..source.len() - 1],
            &format!("{source} false"),
            r#"{"type":"FeatureCollection","features":[],"features":[]}"#,
        ] {
            assert!(read(bad.as_bytes(), &mut |_| Ok(())).is_err());
        }
        assert!(read(source.as_bytes(), &mut |_| Err("consumer closed".into())).is_err());
    }
    #[test]
    fn digest_treats_geometry_numbers_equally_but_preserves_property_integers()
    -> Result<(), Box<dyn std::error::Error>> {
        let a = serde_json::json!({"properties":{"n":18446744073709551615u64},"geometry":{"coordinates":[1,2]}});
        let mut b = a.clone();
        b["geometry"]["coordinates"] = serde_json::json!([1.0, 2.0]);
        let mut left = sha2::Sha256::new();
        let mut right = sha2::Sha256::new();
        digest(&mut left, &a)?;
        digest(&mut right, &b)?;
        assert_eq!(left.clone().finalize(), right.clone().finalize());
        b["properties"]["n"] = serde_json::json!(18446744073709551614u64);
        let mut changed = sha2::Sha256::new();
        digest(&mut changed, &b)?;
        assert_ne!(left.clone().finalize(), changed.clone().finalize());
        Ok(())
    }
}
