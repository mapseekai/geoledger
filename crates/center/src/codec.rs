//! Center wire numbers: exact i64/u64 integers (including integral exponent
//! spellings), and finite binary64 nonintegral decimals with magnitude < 2^53.
//! Nonzero underflow is rejected. Decimal fractions use binary64 rounding;
//! integers are validated from their original digits before any conversion.
//! No shared codec flags or local object encodings change.
use crate::{Error, MAX_BYTES, Result, bad};
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};

pub(crate) fn stored<T: serde::de::DeserializeOwned>(source: &str) -> Result<T> {
    let value = parse(source.as_bytes()).map_err(|error| {
        Error::new(500, "stored Center number or JSON violates codec contract").caused_by(error)
    })?;
    serde_json::from_value(value).map_err(Error::stored_json)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Value> {
    let source = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let mut normalized = String::with_capacity(source.len());
    let mut i = 0;
    let mut quoted = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if quoted {
            let start = i;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    quoted = false;
                    break;
                }
            }
            normalized.push_str(&source[start..i]);
        } else if b == b'"' {
            quoted = true;
            normalized.push('"');
            i += 1;
        } else if b == b'-' || b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                i += 1;
            }
            normalized.push_str(&number(&source[start..i])?);
        } else {
            // Outside strings valid JSON is ASCII; leave syntax rejection to serde.
            if !b.is_ascii() {
                return Err(bad());
            }
            normalized.push(b as char);
            i += 1;
        }
    }
    serde_json::from_str(&normalized).map_err(Error::invalid_json)
}
fn number(token: &str) -> Result<std::borrow::Cow<'_, str>> {
    number_grammar(token)?;
    let negative = token.starts_with('-');
    if !token.contains(['.', 'e', 'E']) {
        if negative {
            token.parse::<i64>().map_err(|_| range())?;
        } else {
            token.parse::<u64>().map_err(|_| range())?;
        }
        return Ok(if token == "-0" { "0" } else { token }.into());
    }
    let unsigned = token.trim_start_matches('-');
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok("0".into());
    }
    let exponent: i64 = exponent.parse().map_err(|_| range())?;
    let shift = exponent.saturating_sub(fraction as i64);
    let integral = shift >= 0
        || (shift.unsigned_abs() < digits.len() as u64
            && digits[digits.len() - shift.unsigned_abs() as usize..]
                .bytes()
                .all(|b| b == b'0'));
    if integral {
        let mut exact = if shift < 0 {
            digits[..digits.len() - shift.unsigned_abs() as usize].to_owned()
        } else {
            digits.to_owned()
        };
        if shift > 20 || exact.len() + shift.max(0) as usize > 20 {
            return Err(range());
        }
        exact.extend(std::iter::repeat_n('0', shift.max(0) as usize));
        if negative {
            exact.insert(0, '-');
            exact.parse::<i64>().map_err(|_| range())?;
        } else {
            exact.parse::<u64>().map_err(|_| range())?;
        }
        Ok(exact.into())
    } else {
        let value: f64 = token.parse().map_err(|_| range())?;
        if !value.is_finite() || value == 0.0 || value.abs() >= 9007199254740992.0 {
            return Err(range());
        }
        if value.fract() == 0.0 {
            Ok((value as i64).to_string().into())
        } else {
            Ok(serde_json::Number::from_f64(value)
                .ok_or_else(range)?
                .to_string()
                .into())
        }
    }
}
fn number_grammar(token: &str) -> Result<()> {
    let bytes = token.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return Err(bad()),
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return Err(bad());
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return Err(bad());
        }
    }
    if i != bytes.len() {
        return Err(bad());
    }
    Ok(())
}
fn range() -> Error {
    Error::new(
        422,
        "number outside Center exact integer / finite binary64 decimal range",
    )
}

pub(crate) struct Budget {
    remaining: usize,
}
impl Budget {
    pub fn new() -> Self {
        Self {
            remaining: MAX_BYTES / 2,
        }
    }
    pub fn take(&mut self, value: &Value) -> Result<()> {
        // Charge the expanded Value tree as well as its serialized representation.
        fn cost(v: &Value) -> usize {
            std::mem::size_of::<Value>()
                + match v {
                    Value::String(s) => s.capacity(),
                    Value::Array(a) => {
                        (a.capacity() - a.len()) * std::mem::size_of::<Value>()
                            + a.iter().map(cost).sum::<usize>()
                    }
                    Value::Object(o) => o.iter().map(|(k, v)| k.capacity() + 64 + cost(v)).sum(),
                    _ => 0,
                }
        }
        let n = cost(value);
        self.remaining = self
            .remaining
            .checked_sub(n)
            .ok_or_else(|| Error::new(413, "response too large; use a smaller page"))?;
        Ok(())
    }
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_BYTES - self.0.len() {
            return Err(io::Error::other("response limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| Error::new(413, "response too large; use a smaller page"))?;
    Ok(writer.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_numeric_tokens_before_value_conversion() {
        for token in [
            "18446744073709551617",
            "18446744073709551617.0",
            "184467440737095516170e-1",
            "-9223372036854775809",
            "1e400",
            "1e-400",
        ] {
            assert!(
                parse(format!("{{\"a\":[[{token}]]}}").as_bytes()).is_err(),
                "{token}"
            );
        }
        for token in [
            "9007199254740993",
            "9007199254740993.0",
            "90071992547409930e-1",
        ] {
            assert_eq!(
                parse(token.as_bytes()).unwrap_or(Value::Null).to_string(),
                "9007199254740993"
            );
        }
        assert_eq!(
            parse(b"18446744073709551615")
                .unwrap_or(Value::Null)
                .to_string(),
            "18446744073709551615"
        );
        assert_eq!(
            parse(b"-9223372036854775808")
                .unwrap_or(Value::Null)
                .to_string(),
            "-9223372036854775808"
        );
        assert!(parse(br#"{"quoted":"18446744073709551617", "x":0.25}"#).is_ok());
        assert_eq!(
            parse(format!("1{}e-1000", "0".repeat(1000)).as_bytes()).unwrap_or(Value::Null),
            1
        );
        for token in ["01", "-", "1.", "1e", "1e+", "--1", "1-2"] {
            assert!(parse(token.as_bytes()).is_err(), "{token}");
        }
        assert!(parse(br#"{"n":18446744073709551617,"n":0}"#).is_err());
        assert_eq!(
            stored::<Value>("18446744073709551617")
                .err()
                .map(|e| e.status),
            Some(500)
        );
    }
    #[test]
    fn bounded_encoding_and_expansion() {
        assert!(encode(&"x".repeat(MAX_BYTES)).is_err());
        assert!(
            Budget::new()
                .take(&serde_json::json!(vec![0; 100_000]))
                .is_err()
        );
    }
}
