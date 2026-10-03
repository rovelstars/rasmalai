pub enum JVal {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<JVal>),
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    pub fn get(&self, key: &str) -> Option<&JVal> {
        if let JVal::Obj(entries) = self {
            entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }

    pub fn at(&self, idx: usize) -> Option<&JVal> {
        if let JVal::Arr(items) = self {
            items.get(idx)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let JVal::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn last_str(&self) -> Option<&str> {
        if let JVal::Arr(items) = self {
            items.last().and_then(|v| v.as_str())
        } else {
            None
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

fn hex_val(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

impl<'a> Cursor<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn expect(&mut self, word: &str) -> Result<(), String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(())
        } else {
            Err(format!("expected `{word}` at {}", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(format!("expected string at {}", self.pos));
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let b = *self.bytes.get(self.pos).ok_or("unterminated string")?;
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let e = *self.bytes.get(self.pos).ok_or("dangling escape")?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0c'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&code) {
                                if self.bytes.get(self.pos) == Some(&b'\\')
                                    && self.bytes.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let low = self.parse_hex4()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    } else {
                                        return Err("bad low surrogate".to_string());
                                    }
                                } else {
                                    return Err("lone high surrogate".to_string());
                                }
                            }
                            out.push(char::from_u32(code).ok_or("bad codepoint")?);
                        }
                        _ => return Err(format!("bad escape `\\{}`", e as char)),
                    }
                }
                _ => {
                    let rest = &self.bytes[self.pos..];
                    let text = std::str::from_utf8(rest).map_err(|_| "bad utf8".to_string())?;
                    let ch = text.chars().next().ok_or("unterminated string")?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut code = 0u32;
        for _ in 0..4 {
            let b = *self.bytes.get(self.pos).ok_or("bad unicode escape")?;
            code = code * 16 + hex_val(b).ok_or("bad unicode escape")?;
            self.pos += 1;
        }
        Ok(code)
    }

    fn parse_value(&mut self) -> Result<JVal, String> {
        self.skip_ws();
        let b = *self.bytes.get(self.pos).ok_or("unexpected end")?;
        match b {
            b'"' => Ok(JVal::Str(self.parse_string()?)),
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b't' => {
                self.expect("true")?;
                Ok(JVal::Bool(true))
            }
            b'f' => {
                self.expect("false")?;
                Ok(JVal::Bool(false))
            }
            b'n' => {
                self.expect("null")?;
                Ok(JVal::Null)
            }
            _ => self.parse_number(),
        }
    }

    fn parse_object(&mut self) -> Result<JVal, String> {
        self.pos += 1;
        let mut entries = Vec::new();
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b'}') {
                self.pos += 1;
                return Ok(JVal::Obj(entries));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(format!("expected `:` at {}", self.pos));
            }
            self.pos += 1;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => continue,
                _ => return Err(format!("expected `,` or `}}` at {}", self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<JVal, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Ok(JVal::Arr(items));
            }
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => continue,
                _ => return Err(format!("expected `,` or `]` at {}", self.pos)),
            }
        }
    }

    fn parse_number(&mut self) -> Result<JVal, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        while self.pos < self.bytes.len() && (self.bytes[self.pos].is_ascii_digit() || matches!(self.bytes[self.pos], b'.' | b'e' | b'E' | b'+' | b'-')) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(format!("bad value at {}", self.pos));
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "bad number".to_string())?;
        Ok(JVal::Num(raw.to_string()))
    }
}

pub fn parse_json(text: &str) -> Result<JVal, String> {
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = cursor.parse_value()?;
    cursor.skip_ws();
    if cursor.pos != cursor.bytes.len() {
        return Err(format!("trailing bytes at {}", cursor.pos));
    }
    Ok(value)
}
