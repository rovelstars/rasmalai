// json_stress: ~1.5 MB document of 20000 flat objects,
// 5 rounds of parse + compact reserialize, checksum = byte sum of output.
enum Val {
    Int(i64),
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Arr(Vec<Val>),
    Obj(Vec<(String, Val)>),
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.pos < self.b.len() && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn parse_str(&mut self) -> String {
        self.pos += 1; // opening quote
        let s = self.pos;
        while self.b[self.pos] != b'"' {
            if self.b[self.pos] == b'\\' {
                self.pos += 1;
            }
            self.pos += 1;
        }
        let o = String::from_utf8_lossy(&self.b[s..self.pos]).into_owned();
        self.pos += 1; // closing quote
        o
    }

    fn parse_val(&mut self) -> Val {
        self.ws();
        match self.b[self.pos] {
            b'{' => {
                self.pos += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.b[self.pos] != b'}' {
                    loop {
                        self.ws();
                        let k = self.parse_str();
                        self.ws();
                        self.pos += 1; // colon
                        let v = self.parse_val();
                        pairs.push((k, v));
                        self.ws();
                        if self.b[self.pos] == b',' {
                            self.pos += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.ws();
                self.pos += 1; // closing brace
                Val::Obj(pairs)
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b[self.pos] != b']' {
                    loop {
                        items.push(self.parse_val());
                        self.ws();
                        if self.b[self.pos] == b',' {
                            self.pos += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.ws();
                self.pos += 1; // closing bracket
                Val::Arr(items)
            }
            b'"' => Val::Str(self.parse_str()),
            b't' => {
                self.pos += 4;
                Val::Bool(true)
            }
            b'f' => {
                self.pos += 5;
                Val::Bool(false)
            }
            b'n' => {
                self.pos += 4;
                Val::Null
            }
            _ => {
                let s = self.pos;
                let mut is_float = false;
                if self.b[self.pos] == b'-' {
                    self.pos += 1;
                }
                while self.b[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                if self.b[self.pos] == b'.' {
                    is_float = true;
                    self.pos += 1;
                    while self.b[self.pos].is_ascii_digit() {
                        self.pos += 1;
                    }
                }
                if self.b[self.pos] == b'e' || self.b[self.pos] == b'E' {
                    is_float = true;
                    self.pos += 1;
                    if self.b[self.pos] == b'+' || self.b[self.pos] == b'-' {
                        self.pos += 1;
                    }
                    while self.b[self.pos].is_ascii_digit() {
                        self.pos += 1;
                    }
                }
                let text = std::str::from_utf8(&self.b[s..self.pos]).unwrap();
                if is_float {
                    Val::Num(text.parse().unwrap())
                } else {
                    Val::Int(text.parse().unwrap())
                }
            }
        }
    }
}

fn ser(o: &mut String, v: &Val) {
    match v {
        Val::Int(n) => o.push_str(&n.to_string()),
        Val::Num(f) => {
            let mut s = format!("{}", f);
            if !s.contains('.') && !s.contains('e') {
                s.push_str(".0");
            }
            o.push_str(&s);
        }
        Val::Str(s) => {
            o.push('"');
            o.push_str(s);
            o.push('"');
        }
        Val::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        Val::Null => o.push_str("null"),
        Val::Arr(items) => {
            o.push('[');
            for (i, c) in items.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                ser(o, c);
            }
            o.push(']');
        }
        Val::Obj(pairs) => {
            o.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                o.push('"');
                o.push_str(k);
                o.push_str("\":");
                ser(o, v);
            }
            o.push('}');
        }
    }
}

fn main() {
    let piece = "{\"id\": 7, \"name\": \"name-7\", \"tags\": [\"a\", \"b\", \"c\"], \"score\": 1.5, \"ok\": true}";
    let chunk: String = (0..500).map(|_| piece).collect::<Vec<_>>().join(",");
    let doc = format!("[{}]", (0..40).map(|_| chunk.as_str()).collect::<Vec<_>>().join(","));
    println!("RESULT doclen {}", doc.len());

    let mut charsum: i64 = 0;
    for _ in 0..5 {
        let mut ps = Parser { b: doc.as_bytes(), pos: 0 };
        let v = ps.parse_val();
        let mut o = String::new();
        ser(&mut o, &v);
        charsum += o.bytes().map(|b| b as i64).sum::<i64>();
    }
    println!("RESULT checksum {}", charsum);
}
