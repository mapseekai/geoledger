use super::*;
use crate::repository::{DatasetRepository, FeatureRepository};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub properties: Map<String, Value>,
    pub geometry: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct Stored {
    pub(super) properties: Map<String, Value>,
    pub(super) geometry: Option<String>,
}
impl Stored {
    pub(super) fn record(&self, key: &str) -> Record {
        let mut fields: BTreeMap<String, Cell> = self
            .properties
            .iter()
            .map(|(k, v)| {
                (
                    format!("/properties/{}", k.replace('~', "~0").replace('/', "~1")),
                    Cell::Text(v.to_string()),
                )
            })
            .collect();
        fields.insert(
            "/geometry".into(),
            self.geometry
                .clone()
                .map(Cell::Geometry)
                .unwrap_or(Cell::Null),
        );
        Record {
            key: key.into(),
            fields,
        }
    }
    pub(super) fn from_record(r: Record) -> Result<Self> {
        let mut properties = Map::new();
        let mut geometry = None;
        for (k, v) in r.fields {
            if k == "/geometry" {
                geometry = match v {
                    Cell::Null => None,
                    Cell::Geometry(s) => Some(s),
                    _ => return Err(bad()),
                };
            } else {
                let key = k
                    .strip_prefix("/properties/")
                    .ok_or_else(bad)?
                    .replace("~1", "/")
                    .replace("~0", "~");
                let Cell::Text(s) = v else { return Err(bad()) };
                properties.insert(key, codec::stored(&s)?);
            }
        }
        Ok(Self {
            properties,
            geometry,
        })
    }
}
pub(super) fn geometry_shape(v: &Value) -> Result<()> {
    let obj = v.as_object().ok_or_else(bad)?;
    let kind = obj.get("type").and_then(Value::as_str).ok_or_else(bad)?;
    if kind == "GeometryCollection" {
        if obj.len() != 2 {
            return Err(bad());
        }
        for g in obj
            .get("geometries")
            .and_then(Value::as_array)
            .ok_or_else(bad)?
        {
            geometry_shape(g)?;
        }
    } else {
        if ![
            "Point",
            "MultiPoint",
            "LineString",
            "MultiLineString",
            "Polygon",
            "MultiPolygon",
        ]
        .contains(&kind)
            || obj.len() != 2
        {
            return Err(bad());
        }
        coordinates(obj.get("coordinates").ok_or_else(bad)?)?;
    }
    Ok(())
}
pub(super) fn coordinates(v: &Value) -> Result<()> {
    let a = v.as_array().ok_or_else(bad)?;
    if a.first().is_some_and(Value::is_number) {
        if !(2..=3).contains(&a.len()) || a.iter().any(|x| !x.as_f64().is_some_and(f64::is_finite))
        {
            return Err(bad());
        }
    } else {
        for x in a {
            coordinates(x)?;
        }
    }
    Ok(())
}
pub(super) fn normalize(f: Feature, key: &str) -> Result<Stored> {
    text(key, 256)?;
    if f.kind != "Feature" || f.id != key || f.properties.len() > 256 {
        return Err(bad());
    }
    for (k, value) in &f.properties {
        text(k, 256)?;
        portable_property(value)?;
    }
    let geometry = crate::geometry::normalize(&f.geometry)?;
    Ok(Stored {
        properties: f.properties,
        geometry,
    })
}

// PostgreSQL JSONB cannot represent U+0000. Apply the same property contract
// before either backend writes, including nested object keys and array values.
// Inspect decoded strings so a literal backslash followed by "u0000" is valid.
fn portable_property(value: &Value) -> Result<()> {
    match value {
        Value::String(value) if value.contains('\0') => {
            return Err(Error::new(400, "feature properties cannot contain U+0000"));
        }
        Value::Array(values) => {
            for value in values {
                portable_property(value)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.contains('\0') {
                    return Err(Error::new(400, "feature properties cannot contain U+0000"));
                }
                portable_property(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_candidate(value: &Stored) -> Result<()> {
    // Merging individually valid property maps can exceed a Feature's bounds.
    if value.properties.len() > 256 {
        return Err(Error::new(
            422,
            "merged feature exceeds feature limits; revise the draft",
        ));
    }
    Ok(())
}

pub(super) fn geojson(key: &str, value: Option<&Stored>) -> Result<Value> {
    let Some(v) = value else {
        return Ok(Value::Null);
    };
    let geometry = match &v.geometry {
        None => Value::Null,
        Some(g) => serde_json::from_str::<Value>(g).map_err(Error::stored_json)?,
    };
    Ok(json!({"type":"Feature","id":key,"properties":v.properties,"geometry":geometry}))
}
pub(super) fn stored_row(r: &Row, properties: usize, geom: usize) -> Result<Option<Stored>> {
    let p: Option<String> = r.get(properties)?;
    p.map(|p| {
        Ok(Stored {
            properties: codec::stored(&p)?,
            geometry: r.get(geom)?,
        })
    })
    .transpose()
}
pub(super) fn at_revision(
    t: &mut (impl FeatureRepository + ?Sized),
    p: &str,
    d: &str,
    key: &str,
    revision: i64,
) -> Result<Option<Stored>> {
    t.feature_at(p, d, key, revision)?
        .map(|r| stored_row(&r, 0, 1))
        .transpose()
        .map(Option::flatten)
}
pub(super) fn dataset(t: &mut (impl DatasetRepository + ?Sized), p: &str, d: &str) -> Result<()> {
    id(d)?;
    t.dataset_exists(p, d)?.ok_or_else(missing)?;
    Ok(())
}

pub(super) fn validate_dataset_geometry(
    t: &mut (impl DatasetRepository + ?Sized),
    p: &str,
    d: &str,
    v: &Stored,
) -> Result<()> {
    let dataset = t.dataset_exists(p, d)?.ok_or_else(missing)?;
    let family: String = dataset.get(0usize)?;
    let dimension: i32 = dataset.get(2usize)?;
    fn matching_dimension(value: &Value, dimension: i32) -> bool {
        match value {
            Value::Array(a) if a.first().is_some_and(Value::is_number) => {
                a.len() == dimension as usize
            }
            Value::Array(a) => a.iter().all(|v| matching_dimension(v, dimension)),
            Value::Object(o) => ["coordinates", "geometries"]
                .iter()
                .all(|key| o.get(*key).is_none_or(|v| matching_dimension(v, dimension))),
            _ => true,
        }
    }
    fn matches(v: &Value, family: &str) -> bool {
        match v.get("type").and_then(Value::as_str) {
            Some("Point" | "MultiPoint") => family == "point",
            Some("LineString" | "MultiLineString") => family == "line",
            Some("Polygon" | "MultiPolygon") => family == "polygon",
            Some("GeometryCollection") => v
                .get("geometries")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().all(|v| matches(v, family))),
            _ => false,
        }
    }
    if let Some(g) = &v.geometry {
        let value: Value = codec::stored(g)?;
        if !matching_dimension(&value, dimension) {
            return Err(Error::new(
                400,
                &format!("dataset {d} requires {dimension}-dimensional coordinates"),
            ));
        }
        if !matches(&value, &family) {
            return Err(Error::new(
                400,
                &format!("dataset {d} only accepts {family} geometries"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{Cell, FeatureQuery};

    struct FeatureOnly;
    impl FeatureRepository for FeatureOnly {
        fn feature_page(&mut self, _: &FeatureQuery) -> Result<Vec<Row>> {
            Err(Error::new(500, "unexpected page read"))
        }
        fn feature_at(
            &mut self,
            p: &str,
            d: &str,
            key: &str,
            revision: i64,
        ) -> Result<Option<Row>> {
            assert_eq!((p, d, key, revision), ("p", "d", "one", 7));
            Ok(Some(Row::new(vec![
                Cell::Text(r#"{"value":9007199254740993}"#.into()),
                Cell::Null,
            ])))
        }
    }

    #[test]
    fn historical_feature_helper_accepts_only_the_feature_capability() -> Result<()> {
        // This substitute implements no credentials, writes, transaction ownership or commit.
        let features: &mut dyn FeatureRepository = &mut FeatureOnly;
        let stored = at_revision(features, "p", "d", "one", 7)?.ok_or_else(missing)?;
        assert_eq!(stored.properties["value"], json!(9007199254740993_u64));
        assert!(stored.geometry.is_none());
        Ok(())
    }
}
