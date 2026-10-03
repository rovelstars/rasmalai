use super::*;

impl<'a> Machine<'a> {
    pub(super) fn builtin(&mut self, name: &str, args: Vec<Value>, span: Span) -> Result<Value, ExecError> {
        match name {
            "print" => {
                let line: Vec<String> = args.iter().map(|v| self.to_pretty_colored(v, 1)).collect();
                self.output.push(line.join(" "));
                Ok(Value::Null)
            }
            "streq" => {
                let eq = match args.as_slice() {
                    [a, b] => match (a, b) {
                        (Value::Str(x), Value::Str(y)) => x == y,
                        _ => false,
                    },
                    _ => false,
                };
                Ok(Value::Bool(eq))
            }
            "strcmp" => {
                let ord = match args.as_slice() {
                    [a, b] => match (a, b) {
                        (Value::Str(x), Value::Str(y)) => {
                            if x == y {
                                0
                            } else if x < y {
                                -1
                            } else {
                                1
                            }
                        }
                        _ => 0,
                    },
                    _ => 0,
                };
                Ok(Value::Int(ord))
            }
            "assert" => match args.as_slice() {
                [c, m] => {
                    if !c.truthy() {
                        let text = m.display();
                        if crate::native::assert_strict() {
                            self.note_error(span);
                            return Err(ExecError::Fatal(format!(
                                "assertion failed: {text}"
                            )));
                        }
                        self.output.push(text);
                        crate::native::test_flag_set();
                    }
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal("assert(condition, message) only".to_string())),
            },
            "__testCheck" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Bool(crate::native::test_flag_check()))
            }
            "__rnx_clock_mono" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(unsafe { crate::native::rnx_clock_monotonic_nanos() } as i64))
            }
            "__rnx_black_box" => match args.as_slice() {
                [v] => Ok(v.clone()),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_any_box" => match args.as_slice() {
                [_, v] => Ok(v.clone()),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_any_unbox" => match args.as_slice() {
                [v] => Ok(v.clone()),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_any_unbox_heap" => match args.as_slice() {
                [v] => Ok(v.clone()),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_any_release_box" => match args.as_slice() {
                [_] => Ok(Value::Null),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_any_retain" => match args.as_slice() {
                [_] => Ok(Value::Null),
                _ => Err(self.native_value_error(span)),
            }
            "__rnx_crypto_random_u64" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(unsafe { crate::native::rnx_crypto_random_u64() } as i64))
            }
            "__rnx_file_open" => match args.as_slice() {
                [Value::Str(path), Value::Int(mode)] => {
                    Ok(Value::Int(crate::native::file_open_impl(path, *mode as u64) as usize as i64))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_file_close" | "__rnx_file_flush" => match args.as_slice() {
                [Value::Int(handle)] => {
                    let h = *handle as usize as *mut u8;
                    if name == "__rnx_file_close" {
                        unsafe { crate::native::rnx_file_close(h) };
                    } else {
                        unsafe { crate::native::rnx_file_flush(h) };
                    }
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle) only"))),
            },
            "__rnx_file_seek" => match args.as_slice() {
                [Value::Int(handle), Value::Int(pos)] => {
                    Ok(Value::Int(crate::native::file_seek_impl(*handle as usize as *mut u8, *pos)))
                }
                _ => Err(ExecError::Fatal("__rnx_file_seek(handle, pos) only".to_string())),
            },
            "__rnx_file_tell" => match args.as_slice() {
                [Value::Int(handle)] => {
                    Ok(Value::Int(crate::native::file_tell_impl(*handle as usize as *mut u8)))
                }
                _ => Err(ExecError::Fatal("__rnx_file_tell(handle) only".to_string())),
            },
            "__rnx_io_is_tty" => match args.as_slice() {
                [Value::Int(fd)] => Ok(Value::Bool(crate::native::io_is_tty_impl(*fd))),
                _ => Err(ExecError::Fatal("__rnx_io_is_tty(fd) only".to_string())),
            },
            "__rnx_io_winsize" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let (cols, rows) = crate::native::io_winsize_impl();
                let id = self.arrays.alloc(2);
                let _ = self.arrays.with_live(id, |live| {
                    live.elems.push(Value::Int(cols));
                    live.elems.push(Value::Int(rows));
                });
                Ok(Value::Array { id })
            }
            "__rnx_io_set_raw" => match args.as_slice() {
                [Value::Int(fd), Value::Int(enabled)] => {
                    match crate::native::io_set_raw_impl(*fd, *enabled != 0) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(ExecError::Fatal("__rnx_io_set_raw(fd, enabled) only".to_string())),
            },
            "__rnx_file_from_handle" => match args.as_slice() {
                [Value::Int(fd), Value::Int(readable), Value::Int(writable)] => {
                    Ok(Value::Int(crate::native::file_from_handle_impl(*fd, *readable != 0, *writable != 0) as usize as i64))
                }
                _ => Err(ExecError::Fatal("__rnx_file_from_handle(fd, readable, writable) only".to_string())),
            },
            "__rnx_bytes_alloc" => match args.as_slice() {
                [Value::Int(cap)] => {
                    Ok(Value::Int(crate::native::bytes_alloc_impl(*cap) as usize as i64))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_bytes_free" => match args.as_slice() {
                [Value::Int(handle)] => {
                    crate::native::bytes_free_impl(*handle as usize as *mut u8);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_bytes_len" | "__rnx_bytes_cap" => match args.as_slice() {
                [Value::Int(handle)] => {
                    Ok(Value::Int(crate::native::bytes_len_impl(*handle as usize as *mut u8)))
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle) only"))),
            },
            "__rnx_bytes_data" => match args.as_slice() {
                [Value::Int(handle)] => {
                    Ok(Value::Int(unsafe {
                        crate::native::rnx_bytes_data(*handle as usize as *mut u8) as usize as i64
                    }))
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle) only"))),
            },
            "__rnx_bytes_copy_within" => match args.as_slice() {
                [Value::Int(handle), Value::Int(target), Value::Int(start), Value::Int(end)] => {
                    crate::native::bytes_copy_within_impl(
                        *handle as usize as *mut u8,
                        *target,
                        *start,
                        *end,
                    );
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_bytes_copy_within(handle, target, start, end) only".to_string(),
                )),
            },
            "__rnx_bytes_read_u8" | "__rnx_bytes_read_i8" | "__rnx_bytes_read_u16le"
            | "__rnx_bytes_read_u16be" | "__rnx_bytes_read_i16le" | "__rnx_bytes_read_i16be"
            | "__rnx_bytes_read_u32le" | "__rnx_bytes_read_u32be" | "__rnx_bytes_read_i32le"
            | "__rnx_bytes_read_i32be" | "__rnx_bytes_read_i64le" | "__rnx_bytes_read_i64be" => {
                match args.as_slice() {
                    [Value::Int(handle), Value::Int(off)] => {
                        let h = *handle as usize as *mut u8;
                        let v = match name {
                            "__rnx_bytes_read_u8" => crate::native::bytes_read_impl(h, *off, 1) as i64,
                            "__rnx_bytes_read_i8" => {
                                let b = crate::native::bytes_read_impl(h, *off, 1);
                                ((b << 56) as i64) >> 56
                            }
                            "__rnx_bytes_read_u16le" => crate::native::bytes_read_impl(h, *off, 2) as i64,
                            "__rnx_bytes_read_u16be" => crate::native::bytes_read_be_impl(h, *off, 2) as i64,
                            "__rnx_bytes_read_i16le" => {
                                ((crate::native::bytes_read_impl(h, *off, 2) << 48) as i64) >> 48
                            }
                            "__rnx_bytes_read_i16be" => {
                                ((crate::native::bytes_read_be_impl(h, *off, 2) << 48) as i64) >> 48
                            }
                            "__rnx_bytes_read_u32le" => crate::native::bytes_read_impl(h, *off, 4) as i64,
                            "__rnx_bytes_read_u32be" => crate::native::bytes_read_be_impl(h, *off, 4) as i64,
                            "__rnx_bytes_read_i32le" => {
                                ((crate::native::bytes_read_impl(h, *off, 4) << 32) as i64) >> 32
                            }
                            "__rnx_bytes_read_i32be" => {
                                ((crate::native::bytes_read_be_impl(h, *off, 4) << 32) as i64) >> 32
                            }
                            "__rnx_bytes_read_i64le" => {
                                crate::native::bytes_read_impl(h, *off, 8) as i64
                            }
                            _ => crate::native::bytes_read_be_impl(h, *off, 8) as i64,
                        };
                        Ok(Value::Int(v))
                    }
                    _ => Err(ExecError::Fatal(format!("{name}(handle, offset) only"))),
                }
            }
            "__rnx_bytes_write_u8" | "__rnx_bytes_write_u16le" | "__rnx_bytes_write_u16be"
            | "__rnx_bytes_write_u32le" | "__rnx_bytes_write_u32be" | "__rnx_bytes_write_u64le"
            | "__rnx_bytes_write_u64be" => match args.as_slice() {
                [Value::Int(handle), Value::Int(off), Value::Int(val)] => {
                    let h = *handle as usize as *mut u8;
                    match name {
                        "__rnx_bytes_write_u8" => crate::native::bytes_write_impl(h, *off, 1, *val as u64),
                        "__rnx_bytes_write_u16le" => crate::native::bytes_write_impl(h, *off, 2, *val as u64),
                        "__rnx_bytes_write_u16be" => crate::native::bytes_write_be_impl(h, *off, 2, *val as u64),
                        "__rnx_bytes_write_u32le" => crate::native::bytes_write_impl(h, *off, 4, *val as u64),
                        "__rnx_bytes_write_u32be" => crate::native::bytes_write_be_impl(h, *off, 4, *val as u64),
                        "__rnx_bytes_write_u64le" => crate::native::bytes_write_impl(h, *off, 8, *val as u64),
                        _ => crate::native::bytes_write_be_impl(h, *off, 8, *val as u64),
                    }
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle, offset, value) only"))),
            },
            "__rnx_bytes_read_f32le" | "__rnx_bytes_read_f32be" | "__rnx_bytes_read_f64le"
            | "__rnx_bytes_read_f64be" => match args.as_slice() {
                [Value::Int(handle), Value::Int(off)] => {
                    let h = *handle as usize as *mut u8;
                    let bits = match name {
                        "__rnx_bytes_read_f32le" => {
                            let b = crate::native::bytes_read_impl(h, *off, 4) as u32;
                            (f32::from_bits(b) as f64).to_bits()
                        }
                        "__rnx_bytes_read_f32be" => {
                            let b = crate::native::bytes_read_be_impl(h, *off, 4) as u32;
                            (f32::from_bits(b) as f64).to_bits()
                        }
                        "__rnx_bytes_read_f64le" => crate::native::bytes_read_impl(h, *off, 8),
                        _ => crate::native::bytes_read_be_impl(h, *off, 8),
                    };
                    Ok(Value::Float(f64::from_bits(bits), lir::instr::FloatKind::Strict))
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle, offset) only"))),
            },
            "__rnx_bytes_write_f32le" | "__rnx_bytes_write_f32be" | "__rnx_bytes_write_f64le"
            | "__rnx_bytes_write_f64be" => match args.as_slice() {
                [Value::Int(handle), Value::Int(off), Value::Float(val, _)] => {
                    let h = *handle as usize as *mut u8;
                    match name {
                        "__rnx_bytes_write_f32le" => {
                            let v = *val as f32;
                            crate::native::bytes_write_impl(h, *off, 4, v.to_bits() as u64)
                        }
                        "__rnx_bytes_write_f32be" => {
                            let v = *val as f32;
                            crate::native::bytes_write_be_impl(h, *off, 4, v.to_bits() as u64)
                        }
                        "__rnx_bytes_write_f64le" => {
                            crate::native::bytes_write_impl(h, *off, 8, val.to_bits())
                        }
                        _ => crate::native::bytes_write_be_impl(h, *off, 8, val.to_bits()),
                    }
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(format!("{name}(handle, offset, value) only"))),
            },
            "__rnx_bytes_read_string" => match args.as_slice() {
                [Value::Int(handle), Value::Int(off), Value::Int(len)] => {
                    Ok(Value::Str(crate::native::bytes_read_string_impl(
                        *handle as usize as *mut u8,
                        *off,
                        *len,
                    )))
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_bytes_read_string(handle, offset, length) only".to_string(),
                )),
            },
            "__rnx_bytes_write_string" => match args.as_slice() {
                [Value::Int(handle), Value::Int(off), Value::Str(text)] => {
                    Ok(Value::Int(crate::native::bytes_write_string_impl(
                        *handle as usize as *mut u8,
                        *off,
                        text.as_bytes(),
                    )))
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_bytes_write_string(handle, offset, text) only".to_string(),
                )),
            },
                        "__rnx_file_read_bytes" => match args.as_slice() {
                [Value::Int(fh), Value::Int(bh), Value::Int(off), Value::Int(len)] => {
                    Ok(Value::Int(crate::native::file_read_bytes_impl(
                        *fh as usize as *mut u8,
                        *bh as usize as *mut u8,
                        *off,
                        *len,
                    )))
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_file_read_bytes(handle, buf, offset, len) only".to_string(),
                )),
            },
            "__rnx_file_write_bytes" => match args.as_slice() {
                [Value::Int(fh), Value::Int(bh), Value::Int(off), Value::Int(len)] => {
                    let fhandle = *fh as usize as *mut u8;
                    let bhandle = *bh as usize as *mut u8;
                    let mut captured: Option<i64> = None;
                    if !bhandle.is_null()
                        && let Some(fd) = crate::native::stdio_fd_of(fhandle)
                        && (fd == 1 || fd == 2)
                    {
                        let cap = crate::native::bytes_len_impl(bhandle);
                        let want = if *len == -1 { cap - *off } else { *len };
                        if *off >= 0 && want >= 0 && *off + want <= cap {
                            let chunk = crate::native::bytes_state(bhandle).data
                                [*off as usize..(*off + want) as usize]
                                .to_vec();
                            let text = String::from_utf8_lossy(&chunk).into_owned();
                            if fd == 1 {
                                self.output.push(text);
                            } else {
                                use std::io::Write as _;
                                let _ = write!(std::io::stderr(), "{text}");
                                let _ = std::io::stderr().flush();
                            }
                            captured = Some(want);
                        }
                    }
                    match captured {
                        Some(n) => Ok(Value::Int(n)),
                        None => Ok(Value::Int(crate::native::file_write_bytes_impl(
                            fhandle, bhandle, *off, *len,
                        ))),
                    }
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_file_write_bytes(handle, buf, offset, len) only".to_string(),
                )),
            },
            "__rnx_file_read_text" => match args.as_slice() {                [Value::Int(handle)] => {
                    let h = *handle as usize as *mut u8;
                    if h.is_null() {
                        return Ok(Value::Str(String::new()));
                    }
                    Ok(Value::Str(crate::native::file_read_impl(h).unwrap_or_default()))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_file_read_text_err" => match args.as_slice() {
                [Value::Int(handle)] => {
                    let h = *handle as usize as *mut u8;
                    Ok(Value::Str(crate::native::file_read_text_err_impl(h)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_file_write_text" => match args.as_slice() {
                [Value::Int(handle), Value::Str(text)] => {
                    let h = *handle as usize as *mut u8;
                    match crate::native::stdio_fd_of(h) {
                        Some(1) => {
                            self.output.push(text.clone());
                            Ok(Value::Int(text.len() as i64))
                        }
                        Some(2) => {
                            use std::io::Write as _;
                            let _ = write!(std::io::stderr(), "{text}");
                            let _ = std::io::stderr().flush();
                            Ok(Value::Int(text.len() as i64))
                        }
                        _ => Ok(Value::Int(crate::native::file_write_impl(h, text.as_bytes()))),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_path_exists" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(std::path::Path::new(path).exists())),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_path_remove" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(crate::native::path_remove_impl(path))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_exists" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(crate::native::fs_exists_impl(path))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_is_file" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(crate::native::fs_is_file_impl(path))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_is_dir" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(crate::native::fs_is_dir_impl(path))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_stat" => match args.as_slice() {
                [Value::Str(path)] => {
                    let fields = crate::native::fs_stat_fields_impl(path);
                    let id = self.arrays.alloc(fields.len());
                    for v in fields {
                        let _ = self.arrays.with_live(id, |live| live.elems.push(Value::Int(v)));
                    }
                    Ok(Value::Array { id })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_stat_err" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Str(crate::native::fs_stat_err_impl(path))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_dir" => match args.as_slice() {
                [Value::Str(path)] => {
                    let items = crate::native::fs_read_dir_impl(path).unwrap_or_default();
                    let id = self.arrays.alloc(items.len());
                    for e in items {
                        let _ = self.arrays.with_live(id, |live| live.elems.push(Value::Str(e)));
                    }
                    Ok(Value::Array { id })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_dir_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_read_dir_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_pool_depth" => match args.as_slice() {
                [Value::Int(d)] => Ok(Value::Int(crate::native::fs_pool_depth_impl(*d))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_glob" => match args.as_slice() {
                [Value::Str(pattern)] => {
                    let items = crate::native::fs_glob_impl(pattern).unwrap_or_default();
                    let id = self.arrays.alloc(items.len());
                    for e in items {
                        let _ = self.arrays.with_live(id, |live| live.elems.push(Value::Str(e)));
                    }
                    Ok(Value::Array { id })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_glob_err" => match args.as_slice() {
                [Value::Str(pattern)] => match crate::native::fs_glob_impl(pattern) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_link" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Str(
                    crate::native::fs_read_link_impl(path).unwrap_or_default(),
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_link_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_read_link_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_remove" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(
                    crate::native::fs_remove_impl(path).unwrap_or(false),
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_remove_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_remove_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_remove_all" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Bool(
                    crate::native::fs_remove_all_impl(path).unwrap_or(false),
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_remove_all_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_remove_all_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mkdir_err" => match args.as_slice() {
                [Value::Str(path), Value::Int(recursive)] => {
                    match crate::native::fs_mkdir_impl(path, *recursive != 0) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_copy_err" => match args.as_slice() {
                [Value::Str(from), Value::Str(to), Value::Int(mode)] => {
                    match crate::native::fs_copy_impl(from, to, *mode) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_move_err" => match args.as_slice() {
                [Value::Str(from), Value::Str(to)] => {
                    match crate::native::fs_move_impl(from, to) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_rename_err" => match args.as_slice() {
                [Value::Str(from), Value::Str(to)] => {
                    match crate::native::fs_rename_impl(from, to) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_truncate_err" => match args.as_slice() {
                [Value::Str(path), Value::Int(len)] => {
                    match crate::native::fs_truncate_impl(path, *len) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_chmod_err" => match args.as_slice() {
                [Value::Str(path), Value::Int(mode)] => {
                    match crate::native::fs_chmod_impl(path, *mode) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_symlink_err" => match args.as_slice() {
                [Value::Str(target), Value::Str(link)] => {
                    match crate::native::fs_symlink_impl(target, link) {
                        Ok(()) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_fsync_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_fsync_impl(path) {
                    Ok(()) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_text" => match args.as_slice() {
                [Value::Str(path)] => Ok(Value::Str(
                    crate::native::fs_read_text_impl(path).unwrap_or_default(),
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_text_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_read_text_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_write_text" => match args.as_slice() {
                [Value::Str(path), Value::Str(text), Value::Int(mode)] => Ok(Value::Int(
                    crate::native::fs_write_text_impl(path, text, *mode).unwrap_or(-1),
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_write_text_err" => match args.as_slice() {
                [Value::Str(path), Value::Str(text), Value::Int(mode)] => {
                    match crate::native::fs_write_text_impl(path, text, *mode) {
                        Ok(_) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_bytes" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_read_bytes_impl(path) {
                    Ok(data) => Ok(Value::Int(
                        Box::into_raw(Box::new(crate::native::ByteBufferState { data }))
                            as usize as i64,
                    )),
                    Err(_) => Ok(Value::Int(0)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_read_bytes_err" => match args.as_slice() {
                [Value::Str(path)] => match crate::native::fs_read_bytes_impl(path) {
                    Ok(_) => Ok(Value::Str(String::new())),
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_write_bytes" => match args.as_slice() {
                [Value::Str(path), Value::Int(handle), Value::Int(mode)] => {
                    let data = crate::native::bytes_state(*handle as usize as *mut u8)
                        .data
                        .clone();
                    Ok(Value::Int(
                        crate::native::fs_write_bytes_impl(path, &data, *mode).unwrap_or(-1),
                    ))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_write_bytes_err" => match args.as_slice() {
                [Value::Str(path), Value::Int(handle), Value::Int(mode)] => {
                    let data = crate::native::bytes_state(*handle as usize as *mut u8)
                        .data
                        .clone();
                    match crate::native::fs_write_bytes_impl(path, &data, *mode) {
                        Ok(_) => Ok(Value::Str(String::new())),
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap" => match args.as_slice() {
                [Value::Str(path), Value::Int(mode)] => Ok(Value::Int(
                    crate::native::fs_mmap_impl(path, *mode)
                        .unwrap_or(std::ptr::null_mut()) as usize as i64,
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_err" => match args.as_slice() {
                [Value::Str(path), Value::Int(mode)] => {
                    match crate::native::fs_mmap_impl(path, *mode) {
                        Ok(handle) => {
                            crate::native::fs_mmap_close_impl(handle);
                            Ok(Value::Str(String::new()))
                        }
                        Err(e) => Ok(Value::Str(e)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_anon" => match args.as_slice() {
                [Value::Int(len)] => Ok(Value::Int(
                    crate::native::fs_mmap_anon_impl(*len).unwrap_or(std::ptr::null_mut())
                        as usize as i64,
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_anon_err" => match args.as_slice() {
                [Value::Int(len)] => match crate::native::fs_mmap_anon_impl(*len) {
                    Ok(handle) => {
                        crate::native::fs_mmap_close_impl(handle);
                        Ok(Value::Str(String::new()))
                    }
                    Err(e) => Ok(Value::Str(e)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_addr" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::fs_mmap_addr_impl(
                    *handle as usize as *mut u8,
                ))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_len" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::fs_mmap_len_impl(
                    *handle as usize as *mut u8,
                ))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_flush" => match args.as_slice() {
                [Value::Int(handle)] => {
                    let _ = crate::native::fs_mmap_flush_impl(*handle as usize as *mut u8);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_fs_mmap_close" => match args.as_slice() {
                [Value::Int(handle)] => {
                    crate::native::fs_mmap_close_impl(*handle as usize as *mut u8);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_env_args_count" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(unsafe { crate::native::rnx_env_args_count() }))
            }
            "__rnx_env_args_get" => match args.as_slice() {
                [Value::Int(i)] => {
                    let p = unsafe { crate::native::rnx_env_args_get(*i) };
                    if p.is_null() {
                        return Err(ExecError::Fatal("arg index out of bounds".to_string()));
                    }
                    Ok(Value::Str(crate::native::native_str(p)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_env_get" => match args.as_slice() {
                [Value::Str(key)] => Ok(Value::Str(crate::native::env_get_impl(key))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_env_set" => match args.as_slice() {
                [Value::Str(key), Value::Str(val)] => {
                    crate::native::env_set_impl(key, val);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_env_cwd" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let p = unsafe { crate::native::rnx_env_cwd() };
                Ok(Value::Str(crate::native::native_str(p)))
            }
            "__rnx_host_version" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let p = unsafe { crate::native::rnx_host_version() };
                Ok(Value::Str(crate::native::native_str(p)))
            }
            "__rnx_env_exit" => match args.as_slice() {
                [Value::Int(code)] => {
                    use std::io::Write as _;
                    for line in self.output.drain(..) {
                        if line.ends_with('\n') {
                            let _ = write!(std::io::stdout(), "{line}");
                        } else {
                            println!("{line}");
                        }
                    }
                    let _ = std::io::stdout().flush();
                    unsafe { crate::native::rnx_env_exit(*code) };
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_connect_start" => match args.as_slice() {
                [Value::Str(host), Value::Int(port)] => {
                    Ok(Value::Int(crate::native::reactor::net_connect_start(host, *port)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_take_error" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::net_take_error(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_connect_wait" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::net_connect_wait(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_recv_or_wait" => match args.as_slice() {
                [Value::Int(handle), Value::Int(max)] => {
                    Ok(Value::Int(crate::native::reactor::net_recv_or_wait(*handle, *max)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_send_or_wait" => match args.as_slice() {
                [Value::Int(handle), Value::Int(byte)] => {
                    Ok(Value::Int(crate::native::reactor::net_send_or_wait(*handle, *byte)))
                }
                _ => Err(self.native_value_error(span)),
            },
                                                            "__rnx_net_recv_get" => match args.as_slice() {
                [Value::Int(handle), Value::Int(idx)] => {
                    Ok(Value::Int(crate::native::reactor::net_recv_get(*handle, *idx)))
                }
                _ => Err(self.native_value_error(span)),
            },
                        "__rnx_net_error_text" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Str(crate::native::reactor::net_error_text(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_close" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::net_close(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_dns_lookup_start" => match args.as_slice() {
                [Value::Str(host)] => {
                    Ok(Value::Int(crate::native::reactor::dns_lookup_start(host)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_dns_lookup_wait" => match args.as_slice() {
                [Value::Int(slot)] => Ok(Value::Int(crate::native::reactor::dns_lookup_wait(*slot))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_dns_lookup_get" => match args.as_slice() {
                [Value::Int(slot)] => Ok(Value::Str(crate::native::reactor::dns_lookup_get(*slot))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_dns_lookup_error" => match args.as_slice() {
                [Value::Int(slot)] => {
                    Ok(Value::Str(crate::native::reactor::dns_lookup_error(*slot)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_listener_bind" => match args.as_slice() {
                [Value::Str(host), Value::Int(port)] => {
                    Ok(Value::Int(crate::native::reactor::net_listener_bind(host, *port)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_listener_port" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::net_listener_port(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_listener_accept_start" => match args.as_slice() {
                [Value::Int(handle)] => {
                    Ok(Value::Int(crate::native::reactor::net_listener_accept_start(*handle)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_listener_accept_wait" => match args.as_slice() {
                [Value::Int(slot)] => Ok(Value::Int(crate::native::reactor::net_listener_accept_wait(*slot))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_net_listener_close" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::net_listener_close(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_connect_start" => match args.as_slice() {
                [Value::Int(tcp), Value::Str(domain)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_connect_start(*tcp, domain)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_handshake_start" => match args.as_slice() {
                [Value::Int(handle)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_handshake_start(*handle)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_handshake_wait" => match args.as_slice() {
                [Value::Int(slot)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_handshake_wait(*slot)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_recv_or_wait" => match args.as_slice() {
                [Value::Int(handle), Value::Int(max)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_recv_or_wait(*handle, *max)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_send_or_wait" => match args.as_slice() {
                [Value::Int(handle), Value::Int(byte)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_send_or_wait(*handle, *byte)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_recv_get" => match args.as_slice() {
                [Value::Int(handle), Value::Int(idx)] => {
                    Ok(Value::Int(crate::native::reactor::tls::tls_recv_get(*handle, *idx)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_error_text" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Str(crate::native::reactor::tls::tls_error_text(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_tls_close" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::reactor::tls::tls_close(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_pid" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(std::process::id() as i64))
            }
            "__rnx_process_remove_env" => match args.as_slice() {
                [Value::Str(key)] => {
                    unsafe { std::env::remove_var(key) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_all_env_count" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(std::env::vars().count() as i64))
            }
            "__rnx_process_all_env_get" => match args.as_slice() {
                [Value::Int(index)] => match std::env::vars().nth(*index as usize) {
                    Some((k, v)) => Ok(Value::Str(format!("{k}={v}"))),
                    None => Ok(Value::Str(String::new())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_chdir" => match args.as_slice() {
                [Value::Str(path)] => {
                    Ok(Value::Int(crate::native::process_chdir_impl(path)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_spawn" => match args.as_slice() {
                [Value::Str(cmd), Value::Array { id: a }, Value::Str(cwd), Value::Array { id: e }, Value::Int(si), Value::Int(so), Value::Int(se)] => {
                    let arg_list = self.strings_of(*a)?;
                    let env_list = self.strings_of(*e)?;
                    Ok(Value::Int(crate::native::process_spawn_impl(cmd, &arg_list, cwd, &env_list, *si, *so, *se)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_run" => match args.as_slice() {
                [Value::Str(cmd), Value::Array { id: a }, Value::Str(cwd), Value::Array { id: e }, Value::Int(si), Value::Int(so), Value::Int(se)] => {
                    let arg_list = self.strings_of(*a)?;
                    let env_list = self.strings_of(*e)?;
                    Ok(Value::Int(crate::native::process_run_impl(cmd, &arg_list, cwd, &env_list, *si, *so, *se)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_pid_of" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_pid_of_impl(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_write_stdin" => match args.as_slice() {
                [Value::Int(handle), Value::Int(buf), Value::Int(off), Value::Int(len)] => {
                    Ok(Value::Int(crate::native::process_write_stdin_impl(*handle, *buf as usize as *mut u8, *off, *len)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_read_stdout" => match args.as_slice() {
                [Value::Int(handle), Value::Int(buf), Value::Int(off), Value::Int(len)] => {
                    Ok(Value::Int(crate::native::process_read_pipe_impl(*handle, 1, *buf as usize as *mut u8, *off, *len)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_read_stderr" => match args.as_slice() {
                [Value::Int(handle), Value::Int(buf), Value::Int(off), Value::Int(len)] => {
                    Ok(Value::Int(crate::native::process_read_pipe_impl(*handle, 2, *buf as usize as *mut u8, *off, *len)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_close_stdin" => match args.as_slice() {
                [Value::Int(handle)] => {
                    crate::native::process_close_stdin_impl(*handle);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_wait" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_wait_impl(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_try_wait" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_try_wait_impl(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_kill" => match args.as_slice() {
                [Value::Int(handle), Value::Int(sig)] => Ok(Value::Int(crate::native::process_kill_impl(*handle, *sig))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_take_stdout" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_take_pipe_impl(*handle, 1))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_take_stderr" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_take_pipe_impl(*handle, 2))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_exit_code" => match args.as_slice() {
                [Value::Int(handle)] => Ok(Value::Int(crate::native::process_exit_code_impl(*handle))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_process_forget" => match args.as_slice() {
                [Value::Int(handle)] => {
                    crate::native::process_forget_impl(*handle);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_os_platform" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Str(std::env::consts::OS.to_string()))
            }
            "__rnx_os_arch" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Str(std::env::consts::ARCH.to_string()))
            }
            "__rnx_os_hostname" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let p = unsafe { crate::native::rnx_os_hostname() };
                let s = crate::native::native_str(p);
                unsafe { crate::native::rnx_release_str(p) };
                Ok(Value::Str(s))
            }
            "__rnx_os_tmpdir" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let p = unsafe { crate::native::rnx_os_tmpdir() };
                let s = crate::native::native_str(p);
                unsafe { crate::native::rnx_release_str(p) };
                Ok(Value::Str(s))
            }
            "__rnx_os_homedir" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                let p = unsafe { crate::native::rnx_os_homedir() };
                let s = crate::native::native_str(p);
                unsafe { crate::native::rnx_release_str(p) };
                Ok(Value::Str(s))
            }
            "__rnx_os_cpu_count" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(unsafe { crate::native::rnx_os_cpu_count() }))
            }
            "__rnx_os_uptime" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Float(f64::from_bits(unsafe { crate::native::rnx_os_uptime() }), lir::instr::FloatKind::Strict))
            }
            "__rnx_string_len" => match args.as_slice() {
                [Value::Str(s)] => Ok(Value::Int(s.chars().count() as i64)),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_slice" => match args.as_slice() {
                [Value::Str(s), Value::Int(start), Value::Int(end)] => {
                    let ascii = s.is_ascii();
                    let total = if ascii { s.len() as i64 } else { s.chars().count() as i64 };
                    let lo = (*start).clamp(0, total) as usize;
                    let hi = (*end).clamp(0, total) as usize;
                    if lo >= hi {
                        return Ok(Value::Str(String::new()));
                    }
                    if ascii {
                        return Ok(Value::Str(s[lo..hi].to_string()));
                    }
                    let mut start_b = s.len();
                    let mut end_b = s.len();
                    let mut ci = 0usize;
                    for (b, _) in s.char_indices() {
                        if ci == lo {
                            start_b = b;
                        }
                        if ci == hi {
                            end_b = b;
                            break;
                        }
                        ci += 1;
                    }
                    if ci < hi {
                        end_b = s.len();
                    }
                    Ok(Value::Str(s[start_b.min(end_b)..end_b].to_string()))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_index_of" => match args.as_slice() {
                [Value::Str(s), Value::Str(nd)] => {
                    if nd.is_empty() {
                        return Ok(Value::Int(0));
                    }
                    match s.find(nd.as_str()) {
                        Some(byte) => {
                            if s.is_ascii() {
                                Ok(Value::Int(byte as i64))
                            } else {
                                Ok(Value::Int(s[..byte].chars().count() as i64))
                            }
                        }
                        None => Ok(Value::Int(-1)),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_index_of_from" => match args.as_slice() {
                [Value::Str(s), Value::Str(nd), Value::Int(from)] => {
                    let ascii = s.is_ascii();
                    let total = if ascii { s.len() as i64 } else { s.chars().count() as i64 };
                    let start = (*from).clamp(0, total);
                    if nd.is_empty() {
                        return Ok(Value::Int(start));
                    }
                    if ascii {
                        let su = start as usize;
                        match s[su..].find(nd.as_str()) {
                            Some(rel) => Ok(Value::Int(start + rel as i64)),
                            None => Ok(Value::Int(-1)),
                        }
                    } else {
                        let su = start as usize;
                        let mut byte_off = s.len();
                        let mut ci = 0usize;
                        for (b, _) in s.char_indices() {
                            if ci == su {
                                byte_off = b;
                                break;
                            }
                            ci += 1;
                        }
                        match s[byte_off..].find(nd.as_str()) {
                            Some(rel) => {
                                Ok(Value::Int(start + s[byte_off..byte_off + rel].chars().count() as i64))
                            }
                            None => Ok(Value::Int(-1)),
                        }
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_trim" => match args.as_slice() {
                [Value::Str(s)] => Ok(Value::Str(s.trim().to_string())),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_concat" => match args.as_slice() {
                [Value::Str(a), Value::Str(b)] => {
                    let mut out = a.clone();
                    out.push_str(b);
                    Ok(Value::Str(out))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_split" => match args.as_slice() {
                [Value::Str(s), Value::Str(nd)] => {
                    let parts: Vec<Value> = if nd.is_empty() {
                        s.chars().map(|c| Value::Str(c.to_string())).collect()
                    } else {
                        let mut out = Vec::new();
                        let mut start = 0usize;
                        for (i, _) in s.match_indices(nd.as_str()) {
                            out.push(Value::Str(s[start..i].to_string()));
                            start = i + nd.len();
                        }
                        out.push(Value::Str(s[start..].to_string()));
                        out
                    };
                    let id = self.arrays.alloc(parts.len());
                    for e in parts {
                        let _ = self.arrays.with_live(id, |live| live.elems.push(e));
                    }
                    Ok(Value::Array { id })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_int_to_str" => match args.as_slice() {
                [Value::Int(v)] => Ok(Value::Str(v.to_string())),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_bool_to_str" => match args.as_slice() {
                [Value::Bool(b)] => Ok(Value::Str(b.to_string())),
                [Value::Int(v)] => Ok(Value::Str((*v != 0).to_string())),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_float_to_str" => match args.as_slice() {
                [Value::Float(f, _)] => {
                    let p = unsafe { crate::native::rnx_float_to_str(f.to_bits()) };
                    let s = crate::native::native_str(p);
                    unsafe { crate::native::rnx_release_str(p) };
                    Ok(Value::Str(s))
                }
                [Value::Int(v)] => {
                    let p = unsafe { crate::native::rnx_float_to_str(*v as u64) };
                    let s = crate::native::native_str(p);
                    unsafe { crate::native::rnx_release_str(p) };
                    Ok(Value::Str(s))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_char_code_at" => match args.as_slice() {                [Value::Str(s), Value::Int(i)] => {
                    if *i < 0 {
                        return Ok(Value::Int(-1));
                    }
                    Ok(Value::Int(s.chars().nth(*i as usize).map(|c| c as i64).unwrap_or(-1)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_string_from_char_code" => match args.as_slice() {
                [Value::Int(c)] => {
                    if *c < 0 {
                        return Ok(Value::Str(String::new()));
                    }
                    match char::from_u32(*c as u32) {
                        Some(ch) => Ok(Value::Str(ch.to_string())),
                        None => Ok(Value::Str(String::new())),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_array_len" => match args.as_slice() {
                [Value::Array { id }] => match self.arrays.len(*id) {
                    Some(n) => Ok(Value::Int(n as i64)),
                    None => Err(ExecError::Fatal("len on dead array".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_array_pop" => match args.as_slice() {
                [Value::Array { id }, _] => match self.arrays.with_live(*id, |live| {
                    live.elems.pop().unwrap_or(Value::Null)
                }) {
                    Some(v) => Ok(v),
                    None => Err(ExecError::Fatal("pop on dead array".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_array_slice" => match args.as_slice() {
                [Value::Array { id }, Value::Int(start), Value::Int(end), Value::Int(incl), ..] => {
                    let id = *id;
                    let n = self.arrays.len(id).unwrap_or(0) as i64;
                    let mut lo = (*start).clamp(0, n);
                    let mut hi = if *incl != 0 { end.saturating_add(1) } else { *end }.clamp(0, n);
                    if lo >= hi {
                        lo = 0;
                        hi = 0;
                    }
                    let elems: Vec<Value> = self
                        .arrays
                        .with_live(id, |live| {
                            live.elems[(lo as usize)..(hi as usize)].to_vec()
                        })
                        .unwrap_or_default();
                    let out = self.arrays.alloc(elems.len());
                    for e in elems {
                        let v = self.shared(e);
                        let _ = self.arrays.with_live(out, |live| live.elems.push(v));
                    }
                    Ok(Value::Array { id: out })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_any_tag" => match args.as_slice() {
                [v] => Ok(Value::Int(match v {
                    Value::Int(_) => 0,
                    Value::Bool(_) => 1,
                    Value::Float(_, _) => 2,
                    Value::Str(_) => 3,
                    _ => 4,
                })),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_obj_class" => match args.as_slice() {
                [Value::Obj { slot, epoch }] => {
                    let ci = self.arena.get(*slot, *epoch).unwrap_or(usize::MAX);
                    Ok(Value::Int(if ci == usize::MAX { 0 } else { ci as i64 + 1 }))
                }
                [Value::Struct { class, .. }] => Ok(Value::Int(*class as i64 + 1)),
                [_] => Ok(Value::Int(0)),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_error_class" => match args.as_slice() {
                [Value::Obj { slot, epoch }] => {
                    let ci = self.arena.get(*slot, *epoch).unwrap_or(usize::MAX);
                    Ok(Value::Int(if ci == usize::MAX { 0 } else { ci as i64 + 1 }))
                }
                [Value::Struct { class, .. }] => Ok(Value::Int(*class as i64 + 1)),
                [_] => Ok(Value::Int(0)),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_error_unbox" => match args.as_slice() {
                [v @ Value::Obj { .. }] | [v @ Value::Struct { .. }] => Ok(v.clone()),
                [_] => Ok(Value::Null),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_error_str" => match args.as_slice() {
                [v] => Ok(Value::Str(v.display())),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_note_type" => Ok(Value::Null),
            "__rnx_io_pretty" => match args.as_slice() {
                [v, Value::Int(fd)] => Ok(Value::Str(self.to_pretty_colored(v, *fd))),
                _ => Err(ExecError::Fatal("__rnx_io_pretty(value, fd) only".to_string())),
            },
            "__rnx_any_to_str" => match args.as_slice() {
                [v] => Ok(Value::Str(self.to_pretty(v))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_eq_any_str" => match args.as_slice() {
                [v, Value::Str(s)] => Ok(Value::Bool(matches!(v, Value::Str(t) if t == s))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_eq_any" => match args.as_slice() {
                [a, b] => Ok(Value::Bool(eq(a, b))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_type_name" => match args.as_slice() {
                [Value::Int(idx)] => {
                    let name = self
                        .module
                        .classes
                        .get(*idx as usize)
                        .map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    Ok(Value::Str(name))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_typeof_any" => match args.as_slice() {
                [v] => {
                    let name = match v {
                        Value::Int(_) => "Int".to_string(),
                        Value::Bool(_) => "Bool".to_string(),
                        Value::Float(_, _) => "Float".to_string(),
                        Value::Str(_) => "String".to_string(),
                        Value::Null => "Null".to_string(),
                        Value::Array { .. } => "Array".to_string(),
                        Value::Range { .. } => "Range".to_string(),
                        Value::Enum { enu, .. } => self
                            .module
                            .enums
                            .get(*enu)
                            .map(|e| e.name.rsplit('.').next().unwrap_or(&e.name).to_string())
                            .unwrap_or_else(|| "Unknown".to_string()),
                        Value::Obj { slot, epoch } => self
                            .arena
                            .get(*slot, *epoch)
                            .and_then(|c| self.module.classes.get(c))
                            .map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string())
                            .unwrap_or_else(|| "Unknown".to_string()),
                        Value::Struct { class, .. } => self
                            .module
                            .classes
                            .get(*class)
                            .map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string())
                            .unwrap_or_else(|| "Unknown".to_string()),
                        _ => "Unknown".to_string(),
                    };
                    Ok(Value::Str(name))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_new" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(self.maps.alloc()))
            }
            "__rnx_map_set" => match args.as_slice() {
                [Value::Int(h), Value::Str(k), Value::Int(v)] => {
                    match self.maps.with_mut(*h, |m| m.set(k, *v as u64)) {
                        Some(_) => {}
                        None => return Err(ExecError::Fatal("set on dead map".to_string())),
                    }
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_get" => match args.as_slice() {
                [Value::Int(h), Value::Str(k)] => match self.maps.with(*h, |m| m.get(k)) {
                    Some(v) => Ok(Value::Int(v as i64)),
                    None => Err(ExecError::Fatal("get on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_has" => match args.as_slice() {
                [Value::Int(h), Value::Str(k)] => match self.maps.with(*h, |m| m.has(k)) {
                    Some(v) => Ok(Value::Bool(v)),
                    None => Err(ExecError::Fatal("has on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_delete" => match args.as_slice() {
                [Value::Int(h), Value::Str(k)] => match self.maps.with_mut(*h, |m| m.delete(k)) {
                    Some(v) => Ok(Value::Bool(v)),
                    None => Err(ExecError::Fatal("delete on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_len" => match args.as_slice() {
                [Value::Int(h)] => match self.maps.with(*h, |m| m.len()) {
                    Some(v) => Ok(Value::Int(v as i64)),
                    None => Err(ExecError::Fatal("len on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_clear" => match args.as_slice() {
                [Value::Int(h)] => match self.maps.with_mut(*h, |m| {
                    m.clear();
                }) {
                    Some(_) => Ok(Value::Null),
                    None => Err(ExecError::Fatal("clear on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_keys" => match args.as_slice() {
                [Value::Int(h)] => match self.maps.with(*h, |m| {
                    m.keys().into_iter().map(Value::Str).collect::<Vec<_>>()
                }) {
                    Some(v) => Ok(self.new_array(v)),
                    None => Err(ExecError::Fatal("keys on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_map_values" => match args.as_slice() {
                [Value::Int(h)] => match self.maps.with(*h, |m| {
                    m.values().into_iter().map(|v| Value::Int(v as i64)).collect::<Vec<_>>()
                }) {
                    Some(v) => Ok(self.new_array(v)),
                    None => Err(ExecError::Fatal("values on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_new" => {
                if !args.is_empty() {
                    return Err(self.native_arity_error(span));
                }
                Ok(Value::Int(self.gmaps.alloc()))
            }
            "__rnx_gmap_free" => match args.as_slice() {
                [Value::Int(h)] => match self.gmaps.remove(*h) {
                    Some(mut st) => {
                        self.json_free_deep(*h, &mut st)?;
                        for v in st.drain() {
                            self.drop_value(v)?;
                        }
                        Ok(Value::Null)
                    }
                    None => Err(ExecError::Fatal("free on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_set" => match args.as_slice() {
                [Value::Int(h), k, v] => {
                    let key = Self::gmap_key(k)?;
                    let val = self.shared(v.clone());
                    let old = match self.gmaps.with_mut(*h, |m| m.set(key, val)) {
                        Some(v) => v,
                        None => return Err(ExecError::Fatal("set on dead map".to_string())),
                    };
                    if let Some(o) = old {
                        self.drop_value(o)?;
                    }
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_get" => match args.as_slice() {
                [Value::Int(h), k] => {
                    let key = Self::gmap_key(k)?;
                    match self.gmaps.with(*h, |m| m.get(&key)) {
                        Some(Some(v)) => Ok(self.shared(v)),
                        Some(None) => Ok(Value::Null),
                        None => Err(ExecError::Fatal("get on dead map".to_string())),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_has" => match args.as_slice() {
                [Value::Int(h), k] => {
                    let key = Self::gmap_key(k)?;
                    match self.gmaps.with(*h, |m| m.has(&key)) {
                        Some(v) => Ok(Value::Bool(v)),
                        None => Err(ExecError::Fatal("has on dead map".to_string())),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_delete" => match args.as_slice() {
                [Value::Int(h), k] => {
                    let key = Self::gmap_key(k)?;
                    let old = match self.gmaps.with_mut(*h, |m| m.delete(&key)) {
                        Some(v) => v,
                        None => return Err(ExecError::Fatal("delete on dead map".to_string())),
                    };
                    let found = old.is_some();
                    if let Some(o) = old {
                        self.drop_value(o)?;
                    }
                    Ok(Value::Bool(found))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_len" => match args.as_slice() {
                [Value::Int(h)] => match self.gmaps.with(*h, |m| m.len()) {
                    Some(v) => Ok(Value::Int(v as i64)),
                    None => Err(ExecError::Fatal("len on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_clear" => match args.as_slice() {
                [Value::Int(h)] => {
                    let old = match self.gmaps.with_mut(*h, |m| m.clear()) {
                        Some(v) => v,
                        None => return Err(ExecError::Fatal("clear on dead map".to_string())),
                    };
                    for v in old {
                        self.drop_value(v)?;
                    }
                    Ok(Value::Null)
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_keys" => match args.as_slice() {
                [Value::Int(h)] => match self.gmaps.with(*h, |m| m.live_keys()) {
                    Some(v) => Ok(self.new_array(v)),
                    None => Err(ExecError::Fatal("keys on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_gmap_values" => match args.as_slice() {
                [Value::Int(h)] => match self.gmaps.with(*h, |m| m.live_vals()) {
                    Some(v) => Ok(self.new_array(v)),
                    None => Err(ExecError::Fatal("values on dead map".to_string())),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_json_parse" => match args.as_slice() {
                [Value::Str(s)] => {
                    match crate::native::json_tape::parse_tape(s.as_bytes()) {
                        Ok(doc) => self.json_from_tape(&doc, 0),
                        Err(e) => {
                            if e.is_depth {
                                Err(ExecError::Fatal("json max depth exceeded".to_string()))
                            } else {
                                Err(ExecError::Fatal(format!(
                                    "json parse error: {} at offset {}",
                                    e.msg, e.offset
                                )))
                            }
                        }
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_json_stringify" => match args.as_slice() {
                [v] => {
                    let mut out = String::new();
                    self.json_write(&mut out, v, 0)?;
                    Ok(Value::Str(out))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_json_stringify_into" => match args.as_slice() {
                [v, Value::Int(handle), Value::Int(pos)] => {
                    let mut out = String::new();
                    self.json_write(&mut out, v, 0)?;
                    Ok(Value::Int(crate::native::bytes_write_string_impl(
                        *handle as usize as *mut u8,
                        *pos,
                        out.as_bytes(),
                    )))
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_json_stringify_into(value, buf, pos) only".to_string(),
                )),
            },
            "__rnx_json_parse_typed" => match args.as_slice() {
                [Value::Str(text), Value::Str(desc)] => {
                    use crate::native::json_tape::{parse_typed, split_typed_desc};
                    let fields = split_typed_desc(desc);
                    match parse_typed(text.as_bytes(), &fields) {
                        Ok(slots) => {
                            let mut elems = Vec::with_capacity(slots.len());
                            for v in slots {
                                elems.push(self.typed_to_value(v)?);
                            }
                            Ok(self.new_array(elems))
                        }
                        Err(e) => {
                            if e.is_depth {
                                Err(ExecError::Fatal("json max depth exceeded".to_string()))
                            } else {
                                Err(ExecError::Fatal(format!(
                                    "json parse error: {} at offset {}",
                                    e.msg, e.offset
                                )))
                            }
                        }
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_json_unwrap" => match args.as_slice() {
                [Value::Pointer(h)] => {
                    if self.json_maps.contains(h) {
                        Ok(Value::Int(*h))
                    } else {
                        Err(ExecError::Fatal("json object expected".to_string()))
                    }
                }
                [Value::Int(h)] => {
                    if self.json_maps.contains(h) {
                        Ok(Value::Int(*h))
                    } else {
                        Err(ExecError::Fatal("json object expected".to_string()))
                    }
                }
                _ => Err(ExecError::Fatal("json object expected".to_string())),
            },
            "__rnx_sync_atomic_get" => match args.as_slice() {
                [Value::Int(id)] => Ok(Value::Int(unsafe { crate::native::rnx_sync_atomic_get(*id) })),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_atomic_set" => match args.as_slice() {
                [Value::Int(id), Value::Int(v)] => {
                    unsafe { crate::native::rnx_sync_atomic_set(*id, *v) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_atomic_fetch_add" => match args.as_slice() {
                [Value::Int(id), Value::Int(d)] => {
                    Ok(Value::Int(unsafe { crate::native::rnx_sync_atomic_fetch_add(*id, *d) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_atomic_cas" => match args.as_slice() {
                [Value::Int(id), Value::Int(e), Value::Int(n)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_sync_atomic_cas(*id, *e, *n) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_send" => match args.as_slice() {
                [Value::Int(id), v] => {
                    crate::ichan::send(*id, v.clone(), self.heaps());
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_send_str" => match args.as_slice() {
                [Value::Int(id), Value::Str(_)] => {
                    let v = args[1].clone();
                    crate::ichan::send(*id, v, self.heaps());
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_send_obj" => match args.as_slice() {
                [Value::Int(id), Value::Obj { .. }] => {
                    let v = args[1].clone();
                    let mut heaps = self.heaps();
                    heaps.retain_value(&v);
                    crate::ichan::send(*id, v, heaps);
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_send_array" => match args.as_slice() {
                [Value::Int(id), Value::Array { .. }] => {
                    let v = args[1].clone();
                    let mut heaps = self.heaps();
                    heaps.retain_value(&v);
                    crate::ichan::send(*id, v, heaps);
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(
                    "__rnx_sync_channel_send_array(id, val) only".to_string(),
                )),
            },
            "__rnx_sync_channel_recv" => match args.as_slice() {
                [Value::Int(id)] => {
                    let heaps = self.heaps();
                    Ok(crate::ichan::recv(*id, &heaps))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_try_recv" => match args.as_slice() {
                [Value::Int(id)] => {
                    let heaps = self.heaps();
                    Ok(crate::ichan::try_recv(*id, &heaps).unwrap_or(Value::Int(-1)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_len" => match args.as_slice() {
                [Value::Int(id)] => Ok(Value::Int(crate::ichan::len(*id))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_sync_channel_drop" => match args.as_slice() {
                [Value::Int(id)] => {
                    let heaps = self.heaps();
                    let mut failure: Option<ExecError> = None;
                    crate::ichan::drain(*id, &heaps, |v| {
                        if failure.is_none() {
                            if let Err(e) = self.drop_value(v) {
                                failure = Some(e);
                            }
                        }
                    });
                    match failure {
                        Some(e) => Err(e),
                        None => Ok(Value::Null),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_mutex_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_mutex_lock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_mutex_unlock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_mutex_unlock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_mutex_try_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_mutex_try_lock(*id) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_read_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_rwlock_read_lock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_read_unlock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_rwlock_read_unlock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_write_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_rwlock_write_lock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_write_unlock" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_rwlock_write_unlock(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_try_read_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_rwlock_try_read_lock(*id) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_rwlock_try_write_lock" => match args.as_slice() {
                [Value::Int(id)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_rwlock_try_write_lock(*id) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_condvar_wait" => match args.as_slice() {
                [Value::Int(cv), Value::Int(m)] => {
                    unsafe { crate::native::rnx_condvar_wait(*cv, *m) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_condvar_wait_timeout" => match args.as_slice() {
                [Value::Int(cv), Value::Int(m), Value::Int(ms)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_condvar_wait_timeout(*cv, *m, *ms) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_condvar_notify_one" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_condvar_notify_one(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_condvar_notify_all" => match args.as_slice() {
                [Value::Int(id)] => {
                    unsafe { crate::native::rnx_condvar_notify_all(*id) };
                    Ok(Value::Null)
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_barrier_wait" => match args.as_slice() {
                [Value::Int(id), Value::Int(n)] => {
                    Ok(Value::Bool(unsafe { crate::native::rnx_barrier_wait(*id, *n) }))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_debug_live_count" => match args.as_slice() {
                [] => Ok(Value::Int(
                    self.arena.live_count() as i64 + self.arrays.live_count() as i64,
                )),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_thread_join_val" => match args.as_slice() {
                [Value::Int(t)] => match crate::threads::join_value(*t) {
                    Ok((v, output)) => {
                        self.output.extend(output);
                        Ok(v)
                    }
                    Err(m) => Err(ExecError::Fatal(m)),
                },
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_thread_join_err" => match args.as_slice() {
                [Value::Int(t)] => Ok(Value::Str(crate::threads::join_error_text(*t))),
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_task_await_val" => match args.as_slice() {
                [Value::Pointer(h)] => {
                    let tag = unsafe { crate::native::rnx_task_tag(*h as *mut u8) };
                    let raw = unsafe { crate::native::rnx_task_await_val(*h as *mut u8) };
                    let payload = unsafe { crate::native::rnx_any_unbox(raw) };
                    unsafe { crate::native::rnx_any_release(raw) };
                    Ok(match tag {
                        1 => Value::Bool(payload != 0),
                        2 => Value::Float(f64::from_bits(payload), lir::instr::FloatKind::Strict),
                        3 => {
                            let s = crate::native::native_str(payload as *const u8);
                            unsafe { crate::native::rnx_str_free(payload as *mut u8) };
                            Value::Str(s)
                        }
                        6 => Value::Null,
                        _ => Value::Int(payload as i64),
                    })
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_task_await_err" => match args.as_slice() {
                [Value::Pointer(h)] => {
                    let p = unsafe { crate::native::rnx_task_await_err(*h as *mut u8) };
                    Ok(Value::Str(crate::native::native_str(p)))
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_pool_new" => match args.as_slice() {
                [Value::Int(w)] => Ok(Value::Pointer(unsafe { crate::native::rnx_pool_new(*w) })),
                _ => Err(self.native_value_error(span)),
            },
            n if n.starts_with("__rnx_math_") => {
                let mut xs = Vec::with_capacity(args.len());
                for a in &args {
                    match a {
                        Value::Float(f, _) => xs.push(*f),
                        Value::Int(i) => xs.push(*i as f64),
                        _ => return Err(ExecError::Fatal(format!("{n} needs float args"))),
                    }
                }
                let out = match (n, xs.as_slice()) {
                    ("__rnx_math_sqrt", [x]) => x.sqrt(),
                    ("__rnx_math_sin", [x]) => x.sin(),
                    ("__rnx_math_cos", [x]) => x.cos(),
                    ("__rnx_math_tan", [x]) => x.tan(),
                    ("__rnx_math_atan2", [y, x]) => y.atan2(*x),
                    ("__rnx_math_pow", [b, e]) => b.powf(*e),
                    ("__rnx_math_floor", [x]) => x.floor(),
                    ("__rnx_math_ceil", [x]) => x.ceil(),
                    ("__rnx_math_round", [x]) => x.round(),
                    ("__rnx_math_log", [x]) => x.ln(),
                    _ => return Err(ExecError::Fatal(format!("unknown builtin `{n}`"))),
                };
                Ok(Value::Float(out, lir::instr::FloatKind::Strict))
            }
            "__rnx_float_nan" => match args.as_slice() {
                [] => Ok(Value::Float(
                    f64::from_bits(crate::native::CANONICAL_NAN_BITS),
                    lir::instr::FloatKind::Strict,
                )),
                _ => Err(ExecError::Fatal("`Float.nan` takes no args".to_string())),
            },
            "__rnx_float_to_bits" => match args.as_slice() {
                [Value::Float(f, _)] => Ok(Value::Int(f.to_bits() as i64)),
                _ => Err(ExecError::Fatal("`toBits` needs a Float receiver".to_string())),
            },
            "__rnx_float_from_bits" => match args.as_slice() {
                [Value::Int(b)] => Ok(Value::Float(
                    f64::from_bits(*b as u64),
                    lir::instr::FloatKind::Strict,
                )),
                _ => Err(ExecError::Fatal("`Float.fromBits` needs an Int argument".to_string())),
            },
            "__rnx_float_fma" => match args.as_slice() {
                [Value::Float(a, _), Value::Float(b, _), Value::Float(c, _)] => Ok(Value::Float(
                    a.mul_add(*b, *c),
                    lir::instr::FloatKind::Strict,
                )),
                _ => Err(ExecError::Fatal("`Float.fma` needs 3 Float arguments".to_string())),
            },            "__rnx_prng_seed" => match args.as_slice() {
                [Value::Array { id }, Value::Int(seed)] => {
                    let s = crate::native::prng_seed_state(*seed as u64);
                    let wrote = self.arrays.with_live(*id, |live| {
                        if live.elems.len() < 4 {
                            return false;
                        }
                        for (i, w) in s.iter().enumerate() {
                            live.elems[i] = Value::Int(*w as i64);
                        }
                        true
                    });
                    match wrote {
                        Some(true) => Ok(Value::Null),
                        Some(false) => {
                            Err(ExecError::Fatal("prng state needs 4 slots".to_string()))
                        }
                        None => Err(ExecError::Fatal("prng seed of dead array".to_string())),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            "__rnx_prng_next" => match args.as_slice() {
                [Value::Array { id }] => {
                    let step: Option<Result<u64, &'static str>> =
                        self.arrays.with_live(*id, |live| {
                            if live.elems.len() < 4 {
                                return Err("prng state needs 4 slots");
                            }
                            let mut s = [0u64; 4];
                            for (i, w) in s.iter_mut().enumerate() {
                                match live.elems[i] {
                                    Value::Int(v) => *w = v as u64,
                                    _ => return Err("prng state must hold ints"),
                                }
                            }
                            let out = crate::native::xoshiro_next(&mut s);
                            for (i, w) in s.iter().enumerate() {
                                live.elems[i] = Value::Int(*w as i64);
                            }
                            Ok(out)
                        });
                    match step {
                        Some(Ok(out)) => Ok(Value::Int(out as i64)),
                        Some(Err(m)) => Err(ExecError::Fatal(m.to_string())),
                        None => Err(ExecError::Fatal("prng next of dead array".to_string())),
                    }
                }
                _ => Err(self.native_value_error(span)),
            },
            _ => Err(ExecError::Fatal(format!("unknown builtin `{name}`"))),
        }
    }


    pub(super) fn shared(&mut self, v: Value) -> Value {        match &v {
            Value::Obj { slot, .. } => self.arena.retain(*slot),
            Value::Array { id } => self.arrays.retain(*id),
            Value::Enum { payload, .. } => {
                let mut items = Vec::with_capacity(payload.len());
                for p in payload {
                    items.push(self.shared(p.clone()));
                }
                return match v {
                    Value::Enum { enu, variant, name, .. } => Value::Enum {
                        enu,
                        variant,
                        name,
                        payload: items,
                    },
                    _ => unreachable!(),
                };
            }
            Value::Struct { fields, class } => {
                return Value::Struct {
                    class: *class,
                    fields: fields.clone(),
                };
            }
            _ => {}
        }
        v
    }


    pub(super) fn to_error(&self, v: Value) -> Value {
        match v {
            Value::Error(_) => v,
            Value::Obj { .. } | Value::Struct { .. } => v,
            Value::Int(_)
            | Value::Float(_, _)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::Null => v,
            other => {
                let tag = other.type_tag(self.module);
                let message = other.display();
                Value::Error(ErrorVal { tag, message })
            }
        }
    }
}
