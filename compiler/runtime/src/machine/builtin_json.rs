use super::*;

impl<'a> Machine<'a> {
    pub(super) fn gmap_key(v: &Value) -> Result<crate::value::GMapKey, ExecError> {
        match v {
            Value::Int(i) => Ok(crate::value::GMapKey::Int(*i)),
            Value::Float(f, _) => {
                let bits = if f.is_nan() { f64::NAN.to_bits() } else { f.to_bits() };
                Ok(crate::value::GMapKey::Float(bits))
            }
            Value::Bool(b) => Ok(crate::value::GMapKey::Bool(*b)),
            Value::Str(s) => Ok(crate::value::GMapKey::Str(s.clone())),
            Value::Obj { slot, epoch } => Ok(crate::value::GMapKey::Obj(*slot, *epoch)),
            other => Err(ExecError::Fatal(format!("unhashable key type `{}`", other.display()))),
        }
    }


    pub(super) fn json_from_tape(
        &mut self,
        doc: &crate::native::json_tape::JsonDoc,
        idx: usize,
    ) -> Result<Value, ExecError> {
        use crate::native::json_tape::*;
        match tape_tag(doc.nodes[idx]) {
            TAPE_NULL => Ok(Value::Null),
            TAPE_BOOL => Ok(Value::Bool(bool_at(doc, idx))),
            TAPE_INT => Ok(Value::Int(int_at(doc, idx))),
            TAPE_FLOAT => Ok(Value::Float(float_at(doc, idx), FloatKind::Strict)),
            TAPE_STR => match std::str::from_utf8(str_at(doc, idx)) {
                Ok(s) => Ok(Value::Str(s.to_string())),
                Err(_) => Ok(Value::Str(
                    String::from_utf8_lossy(str_at(doc, idx)).into_owned(),
                )),
            },
            TAPE_ARRAY => {
                let mut elems = Vec::with_capacity(array_count(&doc.nodes, idx));
                for e in array_iter(&doc.nodes, idx) {
                    elems.push(self.json_from_tape(doc, e)?);
                }
                Ok(self.new_array(elems))
            }
            _ => {
                let h = self.gmaps.alloc();
                self.json_maps.insert(h);
                for (k, v) in object_iter(&doc.nodes, idx) {
                    let key = match std::str::from_utf8(key_at(doc, k)) {
                        Ok(s) => s.to_string(),
                        Err(_) => String::from_utf8_lossy(key_at(doc, k)).into_owned(),
                    };
                    let child = self.json_from_tape(doc, v)?;
                    let kept = self.shared(child);
                    self.gmaps.with_mut(h, |m| {
                        m.set(crate::value::GMapKey::Str(key), kept)
                    });
                }
                Ok(Value::Pointer(h))
            }
        }
    }


    pub(super) fn typed_to_value(
        &mut self,
        v: crate::native::json_tape::TypedVal,
    ) -> Result<Value, ExecError> {
        use crate::native::json_tape::TypedVal;
        match v {
            TypedVal::Null => Ok(Value::Null),
            TypedVal::Bool(b) => Ok(Value::Bool(b)),
            TypedVal::Int(n) => Ok(Value::Int(n)),
            TypedVal::Float(f) => Ok(Value::Float(f, FloatKind::Strict)),
            TypedVal::Str(bytes) => match String::from_utf8(bytes) {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) => Ok(Value::Str(String::from_utf8_lossy(e.as_bytes()).into_owned())),
            },
            TypedVal::Nested(doc) => self.json_from_tape(&doc, 0),
        }
    }


    pub(super) fn json_free_deep(&mut self, h: i64, st: &mut crate::value::GenericMapState) -> Result<(), ExecError> {
        if !self.json_maps.remove(&h) {
            return Ok(());
        }
        let vals: Vec<Value> = st.live_vals();
        for v in vals {
            let n = match v {
                Value::Pointer(n) => n,
                _ => continue,
            };
            if self.json_maps.contains(&n) {
                if let Some(mut sub) = self.gmaps.remove(n) {
                    self.json_free_deep(n, &mut sub)?;
                    for sv in sub.drain() {
                        self.drop_value(sv)?;
                    }
                }
            }
        }
        Ok(())
    }


    pub(super) fn json_write(&self, out: &mut String, v: &Value, depth: usize) -> Result<(), ExecError> {
        if depth > 64 {
            return Err(ExecError::Fatal("json max depth exceeded".to_string()));
        }
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Pointer(h) if self.json_maps.contains(h) => {
                out.push('{');
                let mut first = true;
                if let Some(entries) = self.gmaps.with(*h, |m| {
                    m.entries
                        .iter()
                        .filter(|e| e.active)
                        .map(|e| (e.key.clone(), e.val.clone()))
                        .collect::<Vec<_>>()
                }) {
                    for (k, val) in entries {
                        if let crate::value::GMapKey::Str(ks) = k {
                            if !first {
                                out.push(',');
                            }
                            first = false;
                            out.push('"');
                            crate::native::json_escape_into(out, &ks);
                            out.push('"');
                            out.push(':');
                            self.json_write(out, &val, depth + 1)?;
                        }
                    }
                }
                out.push('}');
            }
            Value::Float(f, _) => {
                if !f.is_finite() {
                    out.push_str("null");
                } else {
                    out.push_str(&crate::native::fmt_float(*f));
                }
            }
            Value::Str(s) => {
                out.push('"');
                crate::native::json_escape_into(out, s);
                out.push('"');
            }
            Value::Array { id } => {
                out.push('[');
                if let Some(elems) = self.arrays.with_live(*id, |live| live.elems.clone()) {
                    let mut first = true;
                    for e in elems {
                        if !first {
                            out.push(',');
                        }
                        first = false;
                        self.json_write(out, &e, depth + 1)?;
                    }
                }
                out.push(']');
            }
            _ => out.push_str("null"),
        }
        Ok(())
    }

}
