//! Small, strict JSON codec. No runtime or build-time third-party dependencies.
use crate::{Error, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::String(s.into())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl Value {
    pub fn object(pairs: impl IntoIterator<Item = (impl Into<String>, Value)>) -> Self {
        Self::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    pub fn array(strings: impl IntoIterator<Item = String>) -> Self {
        Self::Array(strings.into_iter().map(Self::String).collect())
    }

    pub fn get(&self, k: &str) -> Result<&Self> {
        self.map()?
            .get(k)
            .ok_or_else(|| Error::from(format!("missing JSON field {k}")))
    }

    pub fn optional(&self, k: &str) -> Option<&Self> {
        if let Self::Object(m) = self {
            m.get(k)
        } else {
            None
        }
    }

    pub fn map(&self) -> Result<&BTreeMap<String, Self>> {
        if let Self::Object(m) = self {
            Ok(m)
        } else {
            Err("expected JSON object".into())
        }
    }

    pub fn list(&self) -> Result<&[Self]> {
        if let Self::Array(v) = self {
            Ok(v)
        } else {
            Err("expected JSON array".into())
        }
    }

    pub fn str(&self) -> Result<&str> {
        if let Self::String(s) = self {
            Ok(s)
        } else {
            Err("expected JSON string".into())
        }
    }

    pub fn strings(&self) -> Result<Vec<String>> {
        self.list()?
            .iter()
            .map(|v| Ok(v.str()?.to_owned()))
            .collect()
    }

    pub fn set(&mut self, k: &str, v: Self) -> Result<()> {
        if let Self::Object(m) = self {
            m.insert(k.into(), v);
            Ok(())
        } else {
            Err("expected JSON object".into())
        }
    }

    pub fn version(&self) -> Result<()> {
        if self.get("schema_version")? == &Self::Number("1".into()) {
            Ok(())
        } else {
            Err("unsupported schema_version (expected 1)".into())
        }
    }
}

pub fn encode(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.clone(),
        Value::String(s) => {
            let mut out = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
        Value::Array(a) => format!("[{}]", a.iter().map(encode).collect::<Vec<_>>().join(",")),
        Value::Object(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{}:{}", encode(&Value::from(k.clone())), encode(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

pub fn parse(s: &str) -> Result<Value> {
    struct Parser<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl Parser<'_> {
        fn ws(&mut self) {
            while self
                .s
                .get(self.i)
                .is_some_and(|b| b.is_ascii_whitespace() && b" \t\r\n".contains(b))
            {
                self.i += 1
            }
        }

        fn take(&mut self, b: u8) -> Result<()> {
            if self.s.get(self.i) == Some(&b) {
                self.i += 1;
                Ok(())
            } else {
                Err(format!("invalid JSON at byte {}", self.i).into())
            }
        }

        fn hex(&mut self) -> Result<u32> {
            let end = self.i + 4;
            let s = std::str::from_utf8(self.s.get(self.i..end).ok_or("short Unicode escape")?)?;
            if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid Unicode escape".into());
            }
            let n = u32::from_str_radix(s, 16)?;
            self.i = end;
            Ok(n)
        }

        fn string(&mut self) -> Result<String> {
            self.take(b'"')?;
            let mut out = Vec::new();
            loop {
                let b = *self.s.get(self.i).ok_or("unterminated JSON string")?;
                self.i += 1;
                match b {
                    b'"' => return Ok(String::from_utf8(out)?),
                    0..=31 => return Err("control character in JSON string".into()),
                    b'\\' => {
                        let e = *self.s.get(self.i).ok_or("short escape")?;
                        self.i += 1;
                        match e {
                            b'"' | b'\\' | b'/' => out.push(e),
                            b'b' => out.push(8),
                            b'f' => out.push(12),
                            b'n' => out.push(10),
                            b'r' => out.push(13),
                            b't' => out.push(9),
                            b'u' => {
                                let mut n = self.hex()?;
                                if (0xd800..=0xdbff).contains(&n) {
                                    self.take(b'\\')?;
                                    self.take(b'u')?;
                                    let lo = self.hex()?;
                                    if !(0xdc00..=0xdfff).contains(&lo) {
                                        return Err("invalid Unicode surrogate".into());
                                    }
                                    n = 0x10000 + ((n - 0xd800) << 10) + (lo - 0xdc00)
                                }
                                let c = char::from_u32(n).ok_or("invalid Unicode scalar")?;
                                let mut buf = [0; 4];
                                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes())
                            }
                            _ => return Err("invalid JSON escape".into()),
                        }
                    }
                    _ => out.push(b),
                }
            }
        }

        fn value(&mut self, depth: usize) -> Result<Value> {
            if depth > 128 {
                return Err("JSON nesting exceeds 128".into());
            }
            self.ws();
            match self.s.get(self.i).copied().ok_or("unexpected JSON end")? {
                b'"' => Ok(Value::String(self.string()?)),
                b'[' => {
                    self.i += 1;
                    self.ws();
                    let mut a = Vec::new();
                    if self.s.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(Value::Array(a));
                    }
                    loop {
                        a.push(self.value(depth + 1)?);
                        self.ws();
                        if self.s.get(self.i) == Some(&b']') {
                            self.i += 1;
                            break;
                        }
                        self.take(b',')?;
                    }
                    Ok(Value::Array(a))
                }
                b'{' => {
                    self.i += 1;
                    self.ws();
                    let mut m = BTreeMap::new();
                    if self.s.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Ok(Value::Object(m));
                    }
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        self.take(b':')?;
                        if m.insert(k, self.value(depth + 1)?).is_some() {
                            return Err("duplicate JSON key".into());
                        }
                        self.ws();
                        if self.s.get(self.i) == Some(&b'}') {
                            self.i += 1;
                            break;
                        }
                        self.take(b',')?;
                    }
                    Ok(Value::Object(m))
                }
                b'n' | b't' | b'f' => {
                    for (literal, v) in [
                        ("null", Value::Null),
                        ("true", Value::Bool(true)),
                        ("false", Value::Bool(false)),
                    ] {
                        if self.s[self.i..].starts_with(literal.as_bytes()) {
                            self.i += literal.len();
                            return Ok(v);
                        }
                    }
                    Err("invalid JSON literal".into())
                }
                b'-' | b'0'..=b'9' => {
                    let start = self.i;
                    if self.s[self.i] == b'-' {
                        self.i += 1
                    }
                    match self.s.get(self.i) {
                        Some(b'0') => self.i += 1,
                        Some(b'1'..=b'9') => {
                            while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                                self.i += 1
                            }
                        }
                        _ => return Err("invalid JSON number".into()),
                    }
                    if self.s.get(self.i) == Some(&b'.') {
                        self.i += 1;
                        let begin = self.i;
                        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                            self.i += 1
                        }
                        if begin == self.i {
                            return Err("invalid fraction".into());
                        }
                    }
                    if self.s.get(self.i).is_some_and(|b| b"eE".contains(b)) {
                        self.i += 1;
                        if self.s.get(self.i).is_some_and(|b| b"+-".contains(b)) {
                            self.i += 1
                        }
                        let begin = self.i;
                        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                            self.i += 1
                        }
                        if begin == self.i {
                            return Err("invalid exponent".into());
                        }
                    }
                    Ok(Value::Number(
                        std::str::from_utf8(&self.s[start..self.i])?.into(),
                    ))
                }
                _ => Err("invalid JSON value".into()),
            }
        }
    }
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err("trailing JSON data".into());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = parse(
            r#"{"schema_version":1,"s":"\uD83E\uDD8C\n\u0000","a":[true,false,null,-1.5e+3]}"#,
        )
        .unwrap();
        assert_eq!(parse(&encode(&v)).unwrap(), v)
    }

    #[test]
    fn rejects() {
        for s in [
            "01",
            "[1,]",
            "{\"x\":1,\"x\":2}",
            "\"\\uD800\"",
            "\"\\u+123\"",
            "true false",
            "1e",
            "\"\n\"",
        ] {
            assert!(parse(s).is_err(), "{s}")
        }
    }
}
