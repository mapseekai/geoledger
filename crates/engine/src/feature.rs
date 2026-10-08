use super::*;

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
    if f.kind != "Feature"
        || f.id != key
        || f.properties.len() > 256
        || serde_json::to_vec(&f).map_err(|_| bad())?.len() > 16384
    {
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

pub(super) fn validate_candidate(key: &str, value: &Stored) -> Result<()> {
    // Merging individually valid property maps can exceed a Feature's bounds.
    if value.properties.len() > 256
        || serde_json::to_vec(&geojson(key, Some(value))?)
            .map_err(|_| bad())?
            .len()
            > 16384
    {
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
    t: &mut Transaction,
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
pub(super) fn dataset(t: &mut Transaction, p: &str, d: &str) -> Result<()> {
    id(d)?;
    t.dataset_exists(p, d)?.ok_or_else(missing)?;
    Ok(())
}
