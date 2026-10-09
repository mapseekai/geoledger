//! Identical validation and GeoJSON preservation on both storage backends.
#[cfg(any(test, feature = "fuzzing"))]
use crate::Error;
use crate::{Result, bad, geometry_shape};
use geo::Validation;
#[cfg(any(test, feature = "fuzzing"))]
use geo::{BoundingRect, Intersects};
use serde_json::Value;

pub(crate) fn geometry(value: &Value) -> Result<geo::Geometry<f64>> {
    let json: geojson::Geometry = serde_json::from_value(value.clone()).map_err(|_| bad())?;
    geo::Geometry::try_from(json.value).map_err(|_| bad())
}
pub(crate) fn normalize(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    geometry_shape(value)?;
    fn positions(value: &Value, dimension: &mut Option<usize>) -> Result<()> {
        if let Some(a) = value.as_array() {
            if a.first().is_some_and(Value::is_number) {
                if dimension.is_some_and(|d| d != a.len()) {
                    return Err(crate::Error::new(
                        400,
                        "all coordinates within a feature must use the same dimension (XY or XYZ)",
                    ));
                }
                *dimension = Some(a.len());
                let x = a[0].as_f64().ok_or_else(bad)?;
                let y = a[1].as_f64().ok_or_else(bad)?;
                if !(-180.0..=180.0).contains(&x) || !(-90.0..=90.0).contains(&y) {
                    return Err(bad());
                }
            } else {
                for v in a {
                    positions(v, dimension)?;
                }
            }
        } else if let Some(o) = value.as_object() {
            for key in ["coordinates", "geometries"] {
                if let Some(v) = o.get(key) {
                    positions(v, dimension)?;
                }
            }
        }
        Ok(())
    }
    structure(value)?;
    positions(value, &mut None)?;
    Ok(Some(value.to_string()))
}
/// SpatiaLite's JSON parser cannot read nested/empty collection members.
/// Flatten only the derived spatial representation; version snapshots stay exact.
pub(crate) fn spatial_collection(source: &str) -> Result<Option<String>> {
    let value: Value = serde_json::from_str(source).map_err(crate::Error::stored_json)?;
    fn collect(value: Value, out: &mut Vec<Value>) {
        if value["type"] == "GeometryCollection" {
            if let Some(parts) = value["geometries"].as_array() {
                for part in parts {
                    collect(part.clone(), out);
                }
            }
        } else if value["coordinates"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            out.push(value);
        }
    }
    let mut parts = Vec::new();
    collect(value, &mut parts);
    Ok(match parts.len() {
        0 => None,
        1 => parts.pop().map(|v| v.to_string()),
        _ => Some(serde_json::json!({"type":"GeometryCollection","geometries":parts}).to_string()),
    })
}
/// Topology is diagnostic: preserve source coordinates rather than repairing boundaries.
pub(crate) fn topology_warning(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(geometry(value)?
        .check_validation()
        .err()
        .map(|e| e.to_string()))
}
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn bounds(source: &str) -> Result<Option<geo::Rect<f64>>> {
    let v: Value = serde_json::from_str(source).map_err(Error::stored_json)?;
    Ok(geometry(&v)?.bounding_rect())
}
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn intersects(source: &str, bbox: [f64; 4]) -> Result<bool> {
    let v: Value = serde_json::from_str(source).map_err(Error::stored_json)?;
    let rect = geo::Rect::new(
        geo::coord! { x: bbox[0], y: bbox[1] },
        geo::coord! { x: bbox[2], y: bbox[3] },
    );
    Ok(geometry(&v)?.intersects(&rect))
}

// Validate the original coordinate shape before geo-types conversion, which can
// close an unclosed polygon ring. Storage must never silently repair geometry.
fn structure(v: &Value) -> Result<()> {
    let kind = v["type"].as_str().ok_or_else(bad)?;
    fn position(v: &Value) -> Result<()> {
        let a = v.as_array().ok_or_else(bad)?;
        if !(2..=3).contains(&a.len()) || a.iter().any(|n| !n.is_number()) {
            return Err(bad());
        }
        Ok(())
    }
    fn line(v: &Value, ring: bool) -> Result<()> {
        let a = v.as_array().ok_or_else(bad)?;
        if a.len() < if ring { 4 } else { 2 } {
            return Err(bad());
        }
        let dim = a[0].as_array().ok_or_else(bad)?.len();
        for p in a {
            position(p)?;
            if p.as_array().ok_or_else(bad)?.len() != dim {
                return Err(bad());
            }
        }
        if ring && a.first() != a.last() {
            return Err(bad());
        }
        Ok(())
    }
    fn polygon(v: &Value) -> Result<()> {
        let a = v.as_array().ok_or_else(bad)?;
        if a.is_empty() {
            return Err(bad());
        }
        for r in a {
            line(r, true)?;
        }
        Ok(())
    }
    let c = &v["coordinates"];
    match kind {
        "Point" => position(c),
        "LineString" => line(c, false),
        "Polygon" => polygon(c),
        "MultiPoint" | "MultiLineString" | "MultiPolygon" => {
            for part in c.as_array().ok_or_else(bad)? {
                match kind {
                    "MultiPoint" => position(part)?,
                    "MultiLineString" => line(part, false)?,
                    _ => polygon(part)?,
                }
            }
            Ok(())
        }
        "GeometryCollection" => {
            for g in v["geometries"].as_array().ok_or_else(bad)? {
                structure(g)?;
            }
            Ok(())
        }
        _ => Err(bad()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn geometry_is_validated_without_silent_repair() {
        for g in [
            json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}),
            json!({"type":"LineString","coordinates":[[0,0],[1,1,2]]}),
            json!({"type":"Point","coordinates":[181,0]}),
        ] {
            assert!(normalize(&g).is_err());
        }
        let crossing = json!({"type":"Polygon","coordinates":[[[0,0],[1,1],[0,1],[1,0],[0,0]]]});
        assert_eq!(
            normalize(&crossing).ok().flatten(),
            Some(crossing.to_string())
        );
        assert!(topology_warning(&crossing).ok().flatten().is_some());
        let xyz = json!({"type":"LineString","coordinates":[[0,0,3],[1,1,4]]});
        assert_eq!(normalize(&xyz).ok().flatten(), Some(xyz.to_string()));
    }
}
