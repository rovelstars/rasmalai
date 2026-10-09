use diagnostics::{Code, Diagnostic, Span};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct SpanEdit {
    pub span: Span,
    pub replacement: String,
}

pub fn apply_edits(src: &str, edits: &[SpanEdit]) -> Result<String, String> {
    let mut ordered: Vec<(u32, u32, &str)> = Vec::with_capacity(edits.len());
    for e in edits {
        if e.span.start > e.span.end {
            return Err(format!(
                "edit span {}..{} is inverted",
                e.span.start, e.span.end
            ));
        }
        if (e.span.end as usize) > src.len() {
            return Err(format!(
                "edit span {}..{} outside file of {} bytes",
                e.span.start,
                e.span.end,
                src.len()
            ));
        }
        ordered.push((e.span.start, e.span.end, e.replacement.as_str()));
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for w in ordered.windows(2) {
        if w[1].0 < w[0].1 {
            return Err(format!(
                "overlapping edits {}..{} and {}..{}",
                w[0].0, w[0].1, w[1].0, w[1].1
            ));
        }
    }
    let mut out = src.to_string();
    for (start, end, replacement) in ordered.iter().rev() {
        out.replace_range((*start as usize)..(*end as usize), replacement);
    }
    Ok(out)
}

pub fn unified_diff(rel: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut out = format!("--- {rel}\n+++ {rel}\n");
    if a == b {
        return out;
    }
    let ops = diff_ops(&a, &b);
    const CTX: usize = 3;
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (k, o) in ops.iter().enumerate() {
        if o.0 == b' ' {
            continue;
        }
        match groups.last_mut() {
            Some((_, e)) if k - *e <= CTX * 2 => *e = k,
            _ => groups.push((k, k)),
        }
    }
    for (s, e) in groups {
        let hs = s.saturating_sub(CTX);
        let he = (e + CTX + 1).min(ops.len());
        let mut ao = 0usize;
        let mut bo = 0usize;
        for k in 0..hs {
            match ops[k].0 {
                b' ' | b'-' => ao += 1,
                _ => {}
            }
            match ops[k].0 {
                b' ' | b'+' => bo += 1,
                _ => {}
            }
        }
        let mut ac = 0usize;
        let mut bc = 0usize;
        for k in hs..he {
            match ops[k].0 {
                b' ' | b'-' => ac += 1,
                _ => {}
            }
            match ops[k].0 {
                b' ' | b'+' => bc += 1,
                _ => {}
            }
        }
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", ao + 1, ac, bo + 1, bc));
        for k in hs..he {
            out.push(ops[k].0 as char);
            out.push_str(ops[k].1);
            out.push('\n');
        }
    }
    out
}

fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(u8, &'a str)> {
    const CAP: usize = 250_000;
    if a.len() * b.len() > CAP {
        let mut ops = Vec::new();
        let pre = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
        for l in &a[..pre] {
            ops.push((b' ', *l));
        }
        for l in &a[pre..] {
            ops.push((b'-', *l));
        }
        for l in &b[pre..] {
            ops.push((b'+', *l));
        }
        return ops;
    }
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push((b' ', a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push((b'-', a[i]));
            i += 1;
        } else {
            ops.push((b'+', b[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push((b'-', a[i]));
        i += 1;
    }
    while j < m {
        ops.push((b'+', b[j]));
        j += 1;
    }
    ops
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn whole_ident_at(text: &[u8], pos: usize, name: &str) -> bool {
    let nb = name.as_bytes();
    if text.len() < pos + nb.len() {
        return false;
    }
    if &text[pos..pos + nb.len()] != nb {
        return false;
    }
    if pos > 0 && is_ident_char(text[pos - 1]) {
        return false;
    }
    let e = pos + nb.len();
    if e < text.len() && is_ident_char(text[e]) {
        return false;
    }
    true
}

fn skip_spaces(text: &[u8], mut i: usize) -> usize {
    while i < text.len() && (text[i] == b' ' || text[i] == b'\t' || text[i] == b'\r') {
        i += 1;
    }
    i
}

fn match_close(text: &[u8], open: usize, o: u8, c: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    let mut in_str = false;
    let mut in_line_comment = false;
    while i < text.len() {
        let b = text[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < text.len() && text[i + 1] == b'/' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if b == o {
            depth += 1;
        } else if b == c {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

pub struct PushSite {
    pub arg_start: usize,
    pub arg_end: usize,
    pub value: String,
}

pub fn find_push_sites(text: &str, array: &str) -> Vec<PushSite> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !whole_ident_at(bytes, i, array) {
            i += 1;
            continue;
        }
        let mut j = skip_spaces(bytes, i + array.len());
        if j >= bytes.len() || bytes[j] != b'.' {
            i += 1;
            continue;
        }
        j = skip_spaces(bytes, j + 1);
        if !whole_ident_at(bytes, j, "push") {
            i += 1;
            continue;
        }
        j = skip_spaces(bytes, j + 4);
        if j >= bytes.len() || bytes[j] != b'(' {
            i += 1;
            continue;
        }
        match match_close(bytes, j, b'(', b')') {
            Some(close) => {
                let value = text[j + 1..close].trim().to_string();
                out.push(PushSite {
                    arg_start: j + 1,
                    arg_end: close,
                    value,
                });
                i = close + 1;
            }
            None => {
                i = j + 1;
            }
        }
    }
    out
}

pub struct SetSite {
    pub full_start: usize,
    pub value_start: usize,
    pub value_end: usize,
    pub value: String,
}

pub fn find_array_sets(text: &str, array: &str) -> Vec<SetSite> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !whole_ident_at(bytes, i, array) {
            i += 1;
            continue;
        }
        let mut j = skip_spaces(bytes, i + array.len());
        if j >= bytes.len() || bytes[j] != b'[' {
            i += 1;
            continue;
        }
        let close = match match_close(bytes, j, b'[', b']') {
            Some(c) => c,
            None => {
                i = j + 1;
                continue;
            }
        };
        j = skip_spaces(bytes, close + 1);
        if j >= bytes.len() || bytes[j] != b'=' {
            i = j + 1;
            continue;
        }
        if j + 1 < bytes.len() && (bytes[j + 1] == b'=' || bytes[j + 1] == b'>') {
            i = j + 1;
            continue;
        }
        if i > 0 && (bytes[i - 1] == b'!' || bytes[i - 1] == b'=' || bytes[i - 1] == b'<') {
            i += 1;
            continue;
        }
        let vs = j + 1;
        let line_end = bytes[vs..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|k| vs + k)
            .unwrap_or(bytes.len());
        let semi = bytes[vs..line_end].iter().position(|&b| b == b';');
        let semi = match semi {
            Some(k) => vs + k,
            None => {
                i = j + 1;
                continue;
            }
        };
        let value = text[vs..semi].trim().to_string();
        if value.is_empty() {
            i = j + 1;
            continue;
        }
        out.push(SetSite {
            full_start: i,
            value_start: vs,
            value_end: semi,
            value,
        });
        i = semi + 1;
    }
    out
}

pub fn find_pops(text: &str, array: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !whole_ident_at(bytes, i, array) {
            i += 1;
            continue;
        }
        let mut j = skip_spaces(bytes, i + array.len());
        if j < bytes.len() && bytes[j] == b'.' {
            j = skip_spaces(bytes, j + 1);
            if whole_ident_at(bytes, j, "pop") {
                return true;
            }
        }
        i += 1;
    }
    false
}

pub struct WhileRange {
    pub line_start: usize,
    pub open_brace: usize,
    pub close_brace: usize,
    pub line_end: usize,
    pub header: String,
}

pub fn find_while_ranges(text: &str) -> Vec<WhileRange> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !whole_ident_at(bytes, i, "while") {
            i += 1;
            continue;
        }
        let line_start = bytes[..i]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|k| k + 1)
            .unwrap_or(0);
        let line_end = bytes[i..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|k| i + k)
            .unwrap_or(bytes.len());
        let header = text[i..line_end].to_string();
        let brace = match header.find('{') {
            Some(k) => i + k,
            None => {
                i += 5;
                continue;
            }
        };
        let close = match match_close(bytes, brace, b'{', b'}') {
            Some(c) => c,
            None => {
                i += 5;
                continue;
            }
        };
        let close_line_end = bytes[close..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|k| close + k)
            .unwrap_or(bytes.len());
        out.push(WhileRange {
            line_start,
            open_brace: brace,
            close_brace: close,
            line_end: close_line_end,
            header,
        });
        i = brace + 1;
    }
    out.sort_by_key(|r| (r.line_start, r.close_brace));
    out
}

pub fn store_array_name(text: &str, anchor: Span) -> Option<String> {
    let bytes = text.as_bytes();
    let s = (anchor.start as usize).min(bytes.len());
    let line_start = bytes[..s]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|k| k + 1)
        .unwrap_or(0);
    let line_end = bytes[s..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|k| s + k)
        .unwrap_or(bytes.len());
    let mut found: Option<(String, usize)> = None;
    let mut i = line_start;
    while i < line_end {
        let b = bytes[i];
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i + 1;
            while j < line_end && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let name = text[i..j].to_string();
            let k = skip_spaces(bytes, j);
            if k < line_end && bytes[k] == b'[' {
                if found.is_some() {
                    return None;
                }
                found = Some((name, k));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let (name, bracket) = found?;
    let after: usize = {
        let mut depth = 0usize;
        let mut k = bracket;
        let mut end = None;
        while k < line_end {
            if bytes[k] == b'[' {
                depth += 1;
            } else if bytes[k] == b']' {
                depth -= 1;
                if depth == 0 {
                    end = Some(k);
                    break;
                }
            }
            k += 1;
        }
        end?
    };
    let rest = text[after + 1..line_end].trim_start();
    if !rest.starts_with('=') {
        return None;
    }
    if rest.as_bytes().get(1) == Some(&b'=') {
        return None;
    }
    Some(name)
}

pub fn fn_range(text: &str, func: &str) -> Option<(usize, usize)> {
    let short = func.rsplit('.').next().unwrap_or(func);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !whole_ident_at(bytes, i, "fn") {
            i += 1;
            continue;
        }
        let mut j = skip_spaces(bytes, i + 2);
        let ns = j;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'.')
        {
            j += 1;
        }
        if text.get(ns..j).is_none() {
            i += 1;
            continue;
        }
        let name = &text[ns..j];
        if name != short && name != func {
            i = j;
            continue;
        }
        let k = skip_spaces(bytes, j);
        if k >= bytes.len() || bytes[k] != b'(' {
            i = j;
            continue;
        }
        let close = match_close(bytes, k, b'(', b')')?;
        let mut h = skip_spaces(bytes, close + 1);
        if bytes.get(h) == Some(&b':') {
            while h < bytes.len() && bytes[h] != b'{' && bytes[h] != b'\n' {
                h += 1;
            }
        }
        while h < bytes.len() && bytes[h] != b'{' {
            if bytes[h] == b'\n' || bytes[h] == b';' {
                break;
            }
            h += 1;
        }
        if h >= bytes.len() || bytes[h] != b'{' {
            i = j;
            continue;
        }
        let end = match_close(bytes, h, b'{', b'}')?;
        return Some((i, end + 1));
    }
    None
}

pub fn detect_indent_unit(text: &str, anchor_line_start: usize) -> String {
    let anchor_indent = indent_of(&text[anchor_line_start..]);
    let mut scan = 0;
    for line in text.lines() {
        scan += 1;
        if scan > 400 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let ind = indent_of(line);
        if ind.len() > anchor_indent.len() && ind.starts_with(&anchor_indent) {
            let unit = ind[anchor_indent.len()..].to_string();
            if !unit.is_empty() {
                return unit;
            }
        }
    }
    "    ".to_string()
}

fn indent_of(line: &str) -> String {
    line.bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .map(|b| b as char)
        .collect()
}

pub fn zero_fill_template(
    indent: &str,
    unit: &str,
    bound: &str,
    array: &str,
    fresh: &str,
) -> String {
    format!(
        "{i}let {f} = 0;\n{i}while {f} < {b} {{\n{i}{u}{a}.push(0);\n{i}{u}{f} = {f} + 1;\n{i}}}\n",
        i = indent,
        u = unit,
        b = bound,
        a = array,
        f = fresh
    )
}

pub fn fresh_name(text: &str, base: &str) -> String {
    if !whole_word_present(text.as_bytes(), base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let cand = format!("{base}{n}");
        if !whole_word_present(text.as_bytes(), &cand) {
            return cand;
        }
        n += 1;
    }
}

fn whole_word_present(text: &[u8], word: &str) -> bool {
    let nb = word.as_bytes();
    if nb.is_empty() || text.len() < nb.len() {
        return false;
    }
    for i in 0..=text.len() - nb.len() {
        if whole_ident_at(text, i, word) {
            return true;
        }
    }
    false
}

fn line_start_of(text: &str, off: usize) -> usize {
    let bytes = text.as_bytes();
    let s = off.min(bytes.len());
    bytes[..s]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|k| k + 1)
        .unwrap_or(0)
}

fn line_end_of(text: &str, off: usize) -> usize {
    let bytes = text.as_bytes();
    let s = off.min(bytes.len());
    bytes[s..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|k| s + k)
        .unwrap_or(bytes.len())
}

#[derive(Debug)]
pub struct FileFix {
    pub edits: Vec<SpanEdit>,
    pub summary: String,
}

pub fn plan_zeroinit_fix(
    text: &str,
    func: &str,
    anchor: Span,
    array: &str,
) -> Result<FileFix, String> {
    let (fstart, fend) = fn_range(text, func).ok_or_else(|| {
        format!("no autofix available for P201 (zero-init): cannot locate function `{func}`")
    })?;
    let ftext = &text[fstart..fend];
    let off = |p: usize| p + fstart;
    if find_pops(ftext, array) {
        return Err(format!(
            "no autofix available for P201 (zero-init): array `{array}` is popped in `{func}`"
        ));
    }
    let anchor_s = anchor.start as usize;
    let pushes = find_push_sites(ftext, array);
    let sets = find_array_sets(ftext, array);
    let store_idx = sets
        .iter()
        .position(|s| off(s.full_start) <= anchor_s && anchor_s < off(s.value_end) + 1);
    let store = match store_idx {
        Some(i) => i,
        None => {
            return Err(format!(
                "no autofix available for P201 (zero-init): store span does not match `{array}` in `{func}`"
            ));
        }
    };
    let mut garbage_pushes: Vec<&PushSite> = Vec::new();
    for p in &pushes {
        if off(p.arg_end) > anchor_s {
            return Err(format!(
                "no autofix available for P201 (zero-init): array `{array}` is pushed after the nest in `{func}`"
            ));
        }
        if p.value != "0" {
            garbage_pushes.push(p);
        }
    }
    let mut garbage_sets: Vec<usize> = Vec::new();
    for (i, s) in sets.iter().enumerate() {
        if i == store {
            continue;
        }
        if s.value == "0" {
            return Err(format!(
                "no autofix available for P201 (zero-init): array `{array}` has an extra zero write outside the nest in `{func}`"
            ));
        }
        garbage_sets.push(i);
    }
    if !garbage_pushes.is_empty() && !garbage_sets.is_empty() {
        return Err(format!(
            "no autofix available for P201 (zero-init): array `{array}` has mixed garbage fills in `{func}`"
        ));
    }
    if !garbage_pushes.is_empty() {
        let mut edits = Vec::new();
        for p in garbage_pushes {
            let span = Span {
                start: off(p.arg_start) as u32,
                end: off(p.arg_end) as u32,
            };
            edits.push(SpanEdit {
                span,
                replacement: "0".to_string(),
            });
        }
        edits.sort_by_key(|e| (e.span.start, e.span.end));
        return Ok(FileFix {
            edits,
            summary: format!("zero-fill array `{array}` (push value(s) set to 0)"),
        });
    }
    if !garbage_sets.is_empty() {
        return plan_garbage_loop_delete(
            text,
            fstart,
            fend,
            func,
            array,
            anchor_s,
            &sets,
            &garbage_sets,
            &pushes,
        );
    }
    let has_push = !pushes.is_empty();
    if has_push {
        return Err(format!(
            "no autofix available for P201 (zero-init): zero fill of `{array}` is already clean but the proof still fails in `{func}`"
        ));
    }
    plan_fill_insert(text, fstart, fend, func, array, anchor_s)
}

fn while_header_iv(header: &str) -> Option<String> {
    let h = header.trim();
    let h = h.strip_prefix("while")?;
    let h = h.trim_start();
    let cond = h.strip_suffix('{').unwrap_or(h);
    let mut parts = cond.split('<');
    let iv = parts.next()?.trim();
    if iv.is_empty() || !iv.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(iv.to_string())
}

fn while_bound_text(header: &str) -> Option<String> {
    let h = header.trim();
    let h = h.strip_prefix("while")?;
    let h = h.trim_start();
    let cond = h.strip_suffix('{').unwrap_or(h).trim();
    let k = cond.find('<')?;
    let bound = cond[k + 1..].trim();
    if bound.is_empty() || bound.starts_with('=') {
        return None;
    }
    Some(bound.to_string())
}

#[allow(clippy::too_many_arguments)]
fn plan_garbage_loop_delete(
    text: &str,
    fstart: usize,
    fend: usize,
    func: &str,
    array: &str,
    anchor_s: usize,
    sets: &[SetSite],
    garbage: &[usize],
    pushes: &[PushSite],
) -> Result<FileFix, String> {
    let ranges = find_while_ranges(&text[fstart..fend]);
    let off = |p: usize| p + fstart;
    let mut holder: Option<&WhileRange> = None;
    for gi in garbage {
        let s = &sets[*gi];
        let mut best: Option<&WhileRange> = None;
        for r in &ranges {
            if off(r.line_start) <= off(s.full_start) && off(s.value_end) <= off(r.close_brace) {
                match best {
                    Some(b) if b.line_start >= r.line_start => {}
                    _ => best = Some(r),
                }
            }
        }
        let r = match best {
            Some(r) => r,
            None => {
                return Err(format!(
                    "no autofix available for P201 (zero-init): garbage write to `{array}` is not inside a counted loop in `{func}`"
                ));
            }
        };
        if off(r.close_brace) > anchor_s {
            return Err(format!(
                "no autofix available for P201 (zero-init): garbage write to `{array}` is not before the nest in `{func}`"
            ));
        }
        match holder {
            Some(h) if h.line_start == r.line_start => {}
            Some(_) => {
                return Err(format!(
                    "no autofix available for P201 (zero-init): garbage writes to `{array}` span multiple loops in `{func}`"
                ));
            }
            None => holder = Some(r),
        }
    }
    let r = holder.expect("garbage non-empty");
    let iv = while_header_iv(&r.header).ok_or_else(|| {
        format!("no autofix available for P201 (zero-init): cannot read loop header in `{func}`")
    })?;
    let body = &text[off(r.open_brace) + 1..off(r.close_brace)];
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        let nospace: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if nospace == format!("{iv}={iv}+1;") {
            continue;
        }
        let is_push =
            t.contains(&format!("{array}.push(")) || t.contains(&format!("{array} .push("));
        let is_set = find_array_sets(t, array).len() == 1;
        if !is_push && !is_set {
            return Err(format!(
                "no autofix available for P201 (zero-init): garbage loop in `{func}` does other work"
            ));
        }
    }
    if !pushes.iter().any(|p| p.value == "0") {
        return Err(format!(
            "no autofix available for P201 (zero-init): no clean base fill for `{array}` in `{func}`"
        ));
    }
    let del_start = off(r.line_start);
    let mut del_end = off(r.line_end);
    if text.as_bytes().get(del_end) == Some(&b'\n') {
        del_end += 1;
    }
    let mut edits = vec![SpanEdit {
        span: Span {
            start: del_start as u32,
            end: del_end as u32,
        },
        replacement: String::new(),
    }];
    let let_line = format!("let {iv} =");
    let bytes = text.as_bytes();
    let mut ls = line_start_of(text, del_start.saturating_sub(1));
    let mut let_span: Option<(usize, usize)> = None;
    let mut guard = 0;
    while guard < 6 {
        guard += 1;
        let le = line_end_of(text, ls);
        let line = &text[ls..le.min(text.len())];
        if line.trim().is_empty() || line.trim().starts_with("//") {
            if ls == 0 {
                break;
            }
            ls = line_start_of(text, ls.saturating_sub(1));
            continue;
        }
        if line.trim_start().starts_with(&let_line) {
            let mut le2 = le;
            if bytes.get(le2) == Some(&b'\n') {
                le2 += 1;
            }
            let_span = Some((ls, le2));
        }
        break;
    }
    if let Some((ls2, le2)) = let_span {
        let mut count = 0;
        let mut i = fstart;
        while i < fend {
            if whole_ident_at(bytes, i, &iv) {
                let inside_loop = i >= del_start && i < del_end;
                let inside_let = i >= ls2 && i < le2;
                if !inside_loop && !inside_let {
                    count += 1;
                }
                i += iv.len();
            } else {
                i += 1;
            }
        }
        if count == 0 {
            edits.push(SpanEdit {
                span: Span {
                    start: ls2 as u32,
                    end: le2 as u32,
                },
                replacement: String::new(),
            });
        }
    }
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    Ok(FileFix {
        edits,
        summary: format!("remove garbage prefill of `{array}` before the nest"),
    })
}

fn plan_fill_insert(
    text: &str,
    fstart: usize,
    fend: usize,
    func: &str,
    array: &str,
    anchor_s: usize,
) -> Result<FileFix, String> {
    let ranges = find_while_ranges(&text[fstart..fend]);
    let off = |p: usize| p + fstart;
    let mut outer: Option<&WhileRange> = None;
    for r in &ranges {
        if off(r.line_start) <= anchor_s && anchor_s <= off(r.close_brace) {
            match outer {
                Some(o) if off(o.line_start) <= off(r.line_start) => {}
                _ => outer = Some(r),
            }
        }
    }
    let outer = match outer {
        Some(o) => o,
        None => {
            return Err(format!(
                "no autofix available for P201 (zero-init): cannot locate the nest in `{func}`"
            ));
        }
    };
    let mut sibling: Option<(&WhileRange, String)> = None;
    for r in &ranges {
        if off(r.close_brace) > off(outer.line_start) {
            continue;
        }
        if !text[off(r.open_brace)..off(r.close_brace)].contains(".push(") {
            continue;
        }
        if let Some(bound) = while_bound_text(&r.header) {
            sibling = Some((r, bound));
            break;
        }
    }
    let bound = match sibling {
        Some((_, bound)) => bound,
        None => {
            return Err(format!(
                "no autofix available for P201 (zero-init): no fill loop to copy a bound from in `{func}`"
            ));
        }
    };
    let anchor_line = off(outer.line_start);
    let line_text = &text[anchor_line..line_end_of(text, anchor_line)];
    let indent = indent_of(line_text);
    let unit = detect_indent_unit(&text[fstart..fend], anchor_line - fstart);
    let fresh = fresh_name(text, "fixfill");
    let rendered = zero_fill_template(&indent, &unit, &bound, array, &fresh);
    Ok(FileFix {
        edits: vec![SpanEdit {
            span: Span {
                start: anchor_line as u32,
                end: anchor_line as u32,
            },
            replacement: rendered,
        }],
        summary: format!("insert zero-fill loop for `{array}` before the nest"),
    })
}

pub fn refusal_for_reason(reason: &lir::perflint::MissedReason) -> Option<String> {
    match reason {
        lir::perflint::MissedReason::ZeroInit => None,
        lir::perflint::MissedReason::Alias => Some(
            "no autofix available for P201 (alias): the input and output arrays may alias; rewriting would change semantics"
                .to_string(),
        ),
        lir::perflint::MissedReason::CostGate => Some(
            "no autofix available for P201 (cost-gate): no win to close the loop on".to_string(),
        ),
        lir::perflint::MissedReason::SubscriptRole => Some(
            "no autofix available for P201 (subscript-role): interchange is not legal for this nest shape"
                .to_string(),
        ),
    }
}

pub fn refusal_for_perf_code(code: &str) -> Option<String> {
    match code {
        "P101" | "P102" | "P201" => None,
        _ => Some(format!(
            "no autofix available for {code}: suggest-only lint; apply the hint manually"
        )),
    }
}

#[derive(Clone, Debug)]
pub struct FixRequest {
    pub input: String,
    pub root: PathBuf,
    pub entry: String,
    pub release: bool,
    pub opt_level: u8,
    pub package: Option<String>,
    pub locked: bool,
    pub fix_force: bool,
    pub fix_keep_backup: bool,
    pub cfg: crate::CompileConfig,
}

pub enum FixOutcome {
    Done,
    Exit(i32),
}

struct Baseline {
    tier2_total: usize,
    func_tier2: Vec<String>,
}

fn warn_key(diags: &[Diagnostic]) -> Vec<(String, Option<Span>, String)> {
    let mut out: Vec<(String, Option<Span>, String)> = diags
        .iter()
        .map(|d| (d.code.as_str().to_string(), d.span, d.message.clone()))
        .collect();
    out.sort();
    out
}

fn parse_warnings(text: &str) -> Result<Vec<(String, Option<Span>, String)>, Vec<Diagnostic>> {
    match frontend::parser::Parser::parse_module_all(text) {
        Ok(m) => Ok(warn_key(&m.warnings)),
        Err(e) => Err(e),
    }
}

fn tier2_of(lints: &[lir::perflint::PerfLint]) -> Vec<&lir::perflint::PerfLint> {
    lints
        .iter()
        .filter(|l| {
            !matches!(
                l.code,
                lir::perflint::PerfCode::P101 | lir::perflint::PerfCode::P102
            )
        })
        .collect()
}

fn attribute_path(root: &Path, entry: &Path, span: Option<Span>) -> Option<PathBuf> {
    let (files, mut failed) = crate::report::discover_files(root);
    let probe = Diagnostic::new(Code::P201, "fix probe").with_span(span?);
    let (attr, _) = crate::report::attribute_lenient(&files, &mut failed, &probe, entry);
    Some(attr.path)
}

fn child_build_artifact(req: &FixRequest) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find rnx exe: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("build")
        .arg("--no-color")
        .arg(&req.input)
        .arg("--entry")
        .arg(&req.entry);
    if req.release {
        cmd.arg("--release");
    }
    if let Some(p) = &req.package {
        cmd.arg("-p").arg(p);
    }
    if req.locked {
        cmd.arg("--locked");
    }
    cmd.env_remove("RNX_DENY_PERF_LINTS");
    cmd.env_remove("RNX_SHOW_PERF_NOTES");
    let out = cmd
        .output()
        .map_err(|e| format!("build failed to run: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("build failed: {}", tail.join(" | ")));
    }
    for line in stdout.lines().rev() {
        if let Some(rest) = line.strip_prefix("artifact: ") {
            let rest = rest.trim_end_matches(" (fresh)");
            return Ok(PathBuf::from(rest.trim()));
        }
    }
    Err("build printed no artifact path".to_string())
}

fn run_artifact(path: &Path, root: &Path) -> (i32, Vec<u8>) {
    match Command::new(path).current_dir(root).output() {
        Ok(out) => (out.status.code().unwrap_or(1), out.stdout),
        Err(_) => (127, Vec::new()),
    }
}

struct PlannedFile {
    path: PathBuf,
    func: String,
    edits: Vec<SpanEdit>,
    summary: String,
}

pub fn run_fix(req: &FixRequest) -> FixOutcome {
    let entry_path = Path::new(&req.input);
    let pre = match crate::load_program_cfg(&req.input, req.opt_level, &req.entry, &req.cfg) {
        Ok(p) => p,
        Err(errs) => {
            let theme = diagnostics::theme::AuraTheme::active();
            eprint!(
                "{}",
                crate::report::render_compile_errors(&theme, entry_path, &errs)
            );
            return FixOutcome::Exit(1);
        }
    };
    let pre_tier2: Vec<&lir::perflint::PerfLint> = tier2_of(&pre.lir.perf_lints);
    let mut plans: Vec<PlannedFile> = Vec::new();
    let mut refusals: Vec<String> = Vec::new();
    let mut missed_spans: Vec<(String, Option<Span>)> = Vec::new();
    let mut missed: Vec<&lir::perflint::MissedReduction> = pre
        .lir
        .missed_reductions
        .iter()
        .filter(|m| m.inner_trip >= lir::accum::MIN_ACCUM_TRIP)
        .collect();
    missed.sort_by(|a, b| {
        a.func
            .cmp(&b.func)
            .then(a.span.start.cmp(&b.span.start))
            .then(a.span.end.cmp(&b.span.end))
    });
    for m in missed {
        missed_spans.push((m.func.clone(), Some(m.span)));
        match m.reason {
            lir::perflint::MissedReason::ZeroInit => {
                let path = match attribute_path(&req.root, entry_path, Some(m.span)) {
                    Some(p) => p,
                    None => {
                        refusals.push(format!(
                            "no autofix available for P201 (zero-init): cannot attribute span to a file"
                        ));
                        continue;
                    }
                };
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        refusals.push(format!(
                            "no autofix available for P201 (zero-init): cannot read {}: {e}",
                            path.display()
                        ));
                        continue;
                    }
                };
                let array = match store_array_name(&text, m.span) {
                    Some(a) => a,
                    None => {
                        refusals.push(format!(
                            "no autofix available for P201 (zero-init): cannot read the output array at {}",
                            path.display()
                        ));
                        continue;
                    }
                };
                match plan_zeroinit_fix(&text, &m.func, m.span, &array) {
                    Ok(plan) => {
                        if let Some(p) = plans
                            .iter_mut()
                            .find(|p| p.path == path && p.func == m.func)
                        {
                            p.edits.extend(plan.edits);
                            p.summary = format!("{}; {}", p.summary, plan.summary);
                        } else {
                            plans.push(PlannedFile {
                                path,
                                func: m.func.clone(),
                                edits: plan.edits,
                                summary: plan.summary,
                            });
                        }
                    }
                    Err(r) => refusals.push(r),
                }
            }
            other => {
                if let Some(r) = refusal_for_reason(&other) {
                    refusals.push(r);
                }
            }
        }
    }
    for l in &pre_tier2 {
        if matches!(l.code, lir::perflint::PerfCode::P201) {
            if !missed_spans
                .iter()
                .any(|(f, s)| f == &l.func && *s == Some(l.span))
            {
                refusals.push(format!(
                    "no autofix available for P201: unattributed diagnostic in `{}`",
                    l.func
                ));
            }
            continue;
        }
        let code = match l.code {
            lir::perflint::PerfCode::P101 | lir::perflint::PerfCode::P102 => continue,
            _ => format!("{:?}", l.code),
        };
        if let Some(r) = refusal_for_perf_code(&code) {
            if !refusals.contains(&r) {
                refusals.push(r);
            }
        }
    }
    if plans.is_empty() {
        if refusals.is_empty() {
            println!("no performance lints to fix");
            return FixOutcome::Done;
        }
        for r in &refusals {
            eprintln!("rnx: {r}");
        }
        eprintln!("error: --fix found no qualifying P201 ZeroInit lints (no autofix available)");
        return FixOutcome::Exit(1);
    }
    plans.sort_by(|a, b| a.path.cmp(&b.path).then(a.func.cmp(&b.func)));
    for p in &mut plans {
        p.edits.sort_by_key(|e| (e.span.start, e.span.end));
    }
    let baseline = Baseline {
        tier2_total: pre_tier2.len(),
        func_tier2: pre_tier2
            .iter()
            .map(|l| format!("{}:{:?}:{}..{}", l.func, l.code, l.span.start, l.span.end))
            .collect(),
    };
    for plan in &plans {
        match fix_one_file(req, &baseline, plan) {
            Ok(()) => {}
            Err(code) => return FixOutcome::Exit(code),
        }
    }
    if !refusals.is_empty() {
        for r in &refusals {
            eprintln!("rnx note: {r}");
        }
    }
    FixOutcome::Done
}

fn func_tier2_codes(lints: &[lir::perflint::PerfLint], func: &str) -> Vec<String> {
    let mut out: Vec<String> = tier2_of(lints)
        .into_iter()
        .filter(|l| l.func == func)
        .map(|l| format!("{:?}", l.code))
        .collect();
    out.sort();
    out
}

fn fix_one_file(req: &FixRequest, baseline: &Baseline, plan: &PlannedFile) -> Result<(), i32> {
    let display = crate::report::relative_display(&plan.path);
    let orig_bytes = match std::fs::read(&plan.path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: --fix cannot read {}: {e}", display);
            return Err(1);
        }
    };
    let orig_text = match String::from_utf8(orig_bytes.clone()) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("error: --fix cannot handle non-UTF8 {}", display);
            return Err(1);
        }
    };
    let pre_warn = match parse_warnings(&orig_text) {
        Ok(w) => w,
        Err(_) => {
            eprintln!("error: --fix pre-fix parse failed for {}", display);
            return Err(1);
        }
    };
    let bak = PathBuf::from(format!("{}.bak", plan.path.display()));
    if bak.exists() && !req.fix_force {
        eprintln!(
            "error: --fix refuses to overwrite existing backup {} (use --fix-force)",
            bak.display()
        );
        return Err(1);
    }
    let pre_artifact = match child_build_artifact(req) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: --fix pre-fix build failed for {display}: {e}");
            return Err(1);
        }
    };
    let (pre_exit, pre_out) = run_artifact(&pre_artifact, &req.root);
    let new_text = match apply_edits(&orig_text, &plan.edits) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: --fix cannot plan edits for {display}: {e}");
            return Err(1);
        }
    };
    print!("{}", unified_diff(&display, &orig_text, &new_text));
    if let Err(e) = std::fs::write(&bak, &orig_bytes) {
        eprintln!("error: --fix cannot write backup {}: {e}", bak.display());
        return Err(1);
    }
    if let Err(e) = std::fs::write(&plan.path, new_text.as_bytes()) {
        eprintln!("error: --fix cannot write {display}: {e}");
        return Err(1);
    }
    let fail = |stage: &str| -> Result<(), i32> {
        let _ = std::fs::write(&plan.path, &orig_bytes);
        eprintln!(
            "error: --fix verification failed for {display} at {stage}; restored from backup"
        );
        Err(1)
    };
    match parse_warnings(&new_text) {
        Ok(w) if w == pre_warn => {}
        Ok(_) => return fail("re-parse (warnings changed)"),
        Err(_) => return fail("re-parse"),
    }
    let post = match crate::load_program_cfg(&req.input, req.opt_level, &req.entry, &req.cfg) {
        Ok(p) => p,
        Err(_) => return fail("rebuild"),
    };
    if !post
        .lir
        .transform_receipts
        .iter()
        .any(|r| r.func == plan.func && r.kind == lir::perflint::ReceiptKind::AccumV1)
    {
        return fail("closed-loop receipt check (no P101 AccumV1)");
    }
    if post
        .lir
        .missed_reductions
        .iter()
        .any(|m| m.func == plan.func && m.reason == lir::perflint::MissedReason::ZeroInit)
    {
        return fail("closed-loop receipt check (P201 ZeroInit remains)");
    }
    let post_codes = func_tier2_codes(&post.lir.perf_lints, &plan.func);
    let mut pre_func_codes: Vec<String> = baseline
        .func_tier2
        .iter()
        .filter(|e| e.starts_with(&format!("{}:", plan.func)))
        .map(|e| e.split(':').nth(1).unwrap_or_default().to_string())
        .collect();
    pre_func_codes.sort();
    for c in &post_codes {
        if !pre_func_codes.contains(c) {
            return fail("closed-loop receipt check (new Tier-2 on the region)");
        }
    }
    if tier2_of(&post.lir.perf_lints).len() > baseline.tier2_total {
        return fail("closed-loop receipt check (new Tier-2 overall)");
    }
    let post_artifact = match child_build_artifact(req) {
        Ok(p) => p,
        Err(_) => return fail("rebuild"),
    };
    let (post_exit, post_out) = run_artifact(&post_artifact, &req.root);
    if pre_exit != post_exit || pre_out != post_out {
        return fail("checksum compare");
    }
    if req.fix_keep_backup {
        println!("fixed: {display} ({}) backup kept", plan.summary);
    } else if let Err(e) = std::fs::remove_file(&bak) {
        eprintln!("warning: --fix cannot remove backup {}: {e}", bak.display());
        println!("fixed: {display} ({})", plan.summary);
    } else {
        println!("fixed: {display} ({})", plan.summary);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILL_NEST: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(7);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\n";

    fn anchor_of(src: &str, needle: &str) -> Span {
        let s = src.find(needle).expect("needle") as u32;
        Span {
            start: s,
            end: s + needle.len() as u32,
        }
    }

    #[test]
    fn edits_apply_descending_and_reject_overlap() {
        let src = "abcdefgh";
        let ok = apply_edits(
            src,
            &[
                SpanEdit {
                    span: Span { start: 0, end: 1 },
                    replacement: "A".to_string(),
                },
                SpanEdit {
                    span: Span { start: 6, end: 8 },
                    replacement: "GH".to_string(),
                },
            ],
        )
        .unwrap();
        assert_eq!(ok, "AbcdefGH");
        let err = apply_edits(
            src,
            &[
                SpanEdit {
                    span: Span { start: 1, end: 4 },
                    replacement: "x".to_string(),
                },
                SpanEdit {
                    span: Span { start: 3, end: 5 },
                    replacement: "y".to_string(),
                },
            ],
        )
        .unwrap_err();
        assert!(err.contains("overlapping"), "{err}");
        let err = apply_edits(
            src,
            &[SpanEdit {
                span: Span { start: 0, end: 99 },
                replacement: String::new(),
            }],
        )
        .unwrap_err();
        assert!(err.contains("outside"), "{err}");
    }

    #[test]
    fn diff_marks_changed_lines() {
        let d = unified_diff("f.rnx", "a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("--- f.rnx"), "{d}");
        assert!(d.contains("-b"), "{d}");
        assert!(d.contains("+B"), "{d}");
    }

    #[test]
    fn push_value_zeroing_plan() {
        let anchor = anchor_of(FILL_NEST, "c[x * n + y] = s;");
        let plan = plan_zeroinit_fix(FILL_NEST, "Main", anchor, "c").expect("plan");
        assert_eq!(plan.edits.len(), 1);
        let fixed = apply_edits(FILL_NEST, &plan.edits).unwrap();
        assert!(fixed.contains("c.push(0);"), "{fixed}");
        assert!(!fixed.contains("c.push(7);"), "{fixed}");
        assert_eq!(fixed.len(), FILL_NEST.len());
    }

    #[test]
    fn clean_fill_has_no_plan() {
        let clean = FILL_NEST.replacen("c.push(7);", "c.push(0);", 1);
        let anchor = anchor_of(&clean, "c[x * n + y] = s;");
        let err = plan_zeroinit_fix(&clean, "Main", anchor, "c").unwrap_err();
        assert!(err.contains("no autofix available"), "{err}");
    }

    #[test]
    fn other_array_push_untouched() {
        let anchor = anchor_of(FILL_NEST, "c[x * n + y] = s;");
        let plan = plan_zeroinit_fix(FILL_NEST, "Main", anchor, "c").expect("plan");
        let fixed = apply_edits(FILL_NEST, &plan.edits).unwrap();
        assert!(fixed.contains("a.push(0);"), "{fixed}");
        assert!(fixed.contains("b.push(0);"), "{fixed}");
    }

    #[test]
    fn garbage_loop_delete_plan() {
        let src = "fn Main(): Int {\n    let n = 16;\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        c.push(0);\n        i = i + 1;\n    }\n    let g = 0;\n    while g < n * n {\n        c[g] = g % 64;\n        g = g + 1;\n    }\n    let x = 0;\n    while x < n {\n        c[x] = c[x] + 1;\n        x = x + 1;\n    }\n    return c[0];\n}\n";
        let anchor = anchor_of(src, "c[x] = c[x] + 1;");
        let plan = plan_zeroinit_fix(src, "Main", anchor, "c").expect("plan");
        let fixed = apply_edits(src, &plan.edits).unwrap();
        assert!(!fixed.contains("c[g]"), "{fixed}");
        assert!(!fixed.contains("while g < n * n"), "{fixed}");
        assert!(!fixed.contains("let g = 0;"), "{fixed}");
        assert!(fixed.contains("c.push(0);"), "{fixed}");
    }

    #[test]
    fn fill_template_copies_indent_and_bound() {
        let t = zero_fill_template("    ", "    ", "n * n", "c", "fixfill");
        assert!(t.contains("while fixfill < n * n {"), "{t}");
        assert!(t.contains("    c.push(0);"), "{t}");
        assert_eq!(
            detect_indent_unit("fn M() {\n    let x = 0;\n        let y = 1;\n}\n", 0),
            "    "
        );
        assert_eq!(fresh_name("let fixfill = 0;\n", "fixfill"), "fixfill2");
        assert_eq!(fresh_name("let x = 0;\n", "fixfill"), "fixfill");
    }

    #[test]
    fn refusals_name_every_nonqualifying_reason() {
        assert!(refusal_for_reason(&lir::perflint::MissedReason::ZeroInit).is_none());
        for r in [
            lir::perflint::MissedReason::Alias,
            lir::perflint::MissedReason::CostGate,
            lir::perflint::MissedReason::SubscriptRole,
        ] {
            let msg = refusal_for_reason(&r).expect("refusal");
            assert!(msg.contains("no autofix available"), "{msg}");
        }
        assert!(refusal_for_perf_code("P201").is_none());
        for c in ["P202", "P203", "P999"] {
            let msg = refusal_for_perf_code(c).expect("refusal");
            assert!(msg.contains("no autofix available"), "{msg}");
        }
    }
}
