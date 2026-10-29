//! Backend-independent values and fixed-projection rows.
use crate::{Error, Result};

#[derive(Clone)]
pub enum Cell {
    Null,
    Text(String),
    Integer(i64),
    Real(f64),
    Bool(bool),
}
pub trait Decode: Sized {
    fn decode(cell: &Cell) -> Option<Self>;
}
impl Decode for String {
    fn decode(c: &Cell) -> Option<Self> {
        if let Cell::Text(v) = c {
            Some(v.clone())
        } else {
            None
        }
    }
}
impl Decode for i64 {
    fn decode(c: &Cell) -> Option<Self> {
        if let Cell::Integer(v) = c {
            Some(*v)
        } else {
            None
        }
    }
}
impl Decode for i32 {
    fn decode(c: &Cell) -> Option<Self> {
        i64::decode(c).and_then(|v| i32::try_from(v).ok())
    }
}
impl Decode for bool {
    fn decode(c: &Cell) -> Option<Self> {
        match c {
            Cell::Bool(v) => Some(*v),
            Cell::Integer(v) => Some(*v != 0),
            _ => None,
        }
    }
}
impl<T: Decode> Decode for Option<T> {
    fn decode(c: &Cell) -> Option<Self> {
        if matches!(c, Cell::Null) {
            Some(None)
        } else {
            T::decode(c).map(Some)
        }
    }
}
#[derive(Clone)]
pub struct Row(pub(crate) Vec<Cell>);
impl Row {
    pub fn new(values: Vec<Cell>) -> Self {
        Self(values)
    }

    pub fn get<I: Into<usize>, T: Decode>(&self, index: I) -> Result<T> {
        self.0
            .get(index.into())
            .and_then(T::decode)
            .ok_or_else(|| Error::new(500, "storage row violates repository contract"))
    }
}
