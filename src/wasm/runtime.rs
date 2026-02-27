use crate::wasm::types::{TypeCompareInfo, CmpFieldKind};
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

pub struct UdfRuntime {
    engine: Engine,
    linker: Linker<HostState>,
}

pub struct HostState {
    pub limits: StoreLimits,
    pub query_id: String,
    pub database: String,
    pub schema: String,
    pub user: String,
    pub config: std::collections::HashMap<String, String>,
    pub logs: Vec<String>,
    pub monotonic_epoch: std::time::Instant,
    pub type_layouts: Vec<TypeCompareInfo>,
}

impl HostState {
    pub fn new() -> Self {
        Self {
            limits: StoreLimitsBuilder::new()
                .memory_size(64 * 1024 * 1024)
                .build(),
            query_id: String::new(),
            database: String::new(),
            schema: String::new(),
            user: String::new(),
            config: std::collections::HashMap::new(),
            logs: Vec::new(),
            monotonic_epoch: std::time::Instant::now(),
            type_layouts: Vec::new(),
        }
    }

    pub fn with_type_layouts(mut self, layouts: Vec<TypeCompareInfo>) -> Self {
        self.type_layouts = layouts;
        self
    }

    pub fn with_query_metadata(
        mut self,
        query_id: &str,
        database: &str,
        schema: &str,
        user: &str,
    ) -> Self {
        self.query_id = query_id.to_string();
        self.database = database.to_string();
        self.schema = schema.to_string();
        self.user = user.to_string();
        self
    }

    pub fn with_config(mut self, key: &str, value: &str) -> Self {
        self.config.insert(key.to_string(), value.to_string());
        self
    }
}

fn validate_read(
    memory: &wasmtime::Memory,
    caller: &wasmtime::Caller<'_, HostState>,
    ptr: i32,
    len: i32,
) -> Result<(), wasmtime::Error> {
    if ptr < 0 || len < 0 {
        return Err(wasmtime::Error::msg("negative pointer or length"));
    }
    let end = (ptr as u64) + (len as u64);
    if end > memory.data_size(caller) as u64 {
        return Err(wasmtime::Error::msg(format!(
            "out of bounds memory read: ptr={}, len={}, mem_size={}",
            ptr,
            len,
            memory.data_size(caller)
        )));
    }
    Ok(())
}

fn validate_write(
    memory: &wasmtime::Memory,
    caller: &wasmtime::Caller<'_, HostState>,
    ptr: i32,
    len: usize,
) -> Result<(), wasmtime::Error> {
    if ptr < 0 {
        return Err(wasmtime::Error::msg("negative pointer"));
    }
    let end = (ptr as u64) + (len as u64);
    if end > memory.data_size(caller) as u64 {
        return Err(wasmtime::Error::msg(format!(
            "out of bounds memory write: ptr={}, len={}, mem_size={}",
            ptr,
            len,
            memory.data_size(caller)
        )));
    }
    Ok(())
}

fn get_heap_ptr_global(
    caller: &mut wasmtime::Caller<'_, HostState>,
) -> Result<wasmtime::Global, wasmtime::Error> {
    caller
        .get_export("heap_ptr")
        .and_then(|e| e.into_global())
        .ok_or_else(|| wasmtime::Error::msg("missing heap_ptr global export"))
}

fn get_memory(
    caller: &mut wasmtime::Caller<'_, HostState>,
) -> Result<wasmtime::Memory, wasmtime::Error> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("missing memory export"))
}

fn host_alloc(
    caller: &mut wasmtime::Caller<'_, HostState>,
    size: i32,
) -> Result<i32, wasmtime::Error> {
    let aligned = (size + 7) & !7;

    let g = get_heap_ptr_global(caller)?;
    let old_ptr = g
        .get(&mut *caller)
        .i32()
        .ok_or_else(|| wasmtime::Error::msg("heap_ptr is not i32"))?;

    let new_ptr = old_ptr
        .checked_add(aligned)
        .ok_or_else(|| wasmtime::Error::msg("out of memory: heap pointer overflow"))?;

    let memory = get_memory(caller)?;
    let mem_bytes = memory.data_size(&*caller) as i32;
    if new_ptr > mem_bytes {
        let pages_needed = ((new_ptr - mem_bytes) as u32 + 65535) >> 16;
        memory
            .grow(&mut *caller, pages_needed as u64)
            .map_err(|_| {
                wasmtime::Error::msg("out of memory: WASM linear memory could not be grown")
            })?;
    }

    let g = get_heap_ptr_global(caller)?;
    g.set(&mut *caller, wasmtime::Val::I32(new_ptr))
        .map_err(|e| wasmtime::Error::msg(format!("failed to update heap_ptr: {}", e)))?;

    Ok(old_ptr)
}

fn host_write_string(
    caller: &mut wasmtime::Caller<'_, HostState>,
    s: &[u8],
) -> Result<(i32, i32), wasmtime::Error> {
    let len = s.len() as i32;
    if len == 0 {
        return Ok((0, 0));
    }
    let ptr = host_alloc(caller, len)?;
    let memory = get_memory(caller)?;
    validate_write(&memory, &*caller, ptr, s.len())?;
    memory.data_mut(&mut *caller)[ptr as usize..ptr as usize + s.len()].copy_from_slice(s);
    Ok((ptr, len))
}

fn host_compare_fields(
    data: &[u8],
    layouts: &[TypeCompareInfo],
    info: &TypeCompareInfo,
    ptr_a: i32,
    ptr_b: i32,
) -> Result<i32, wasmtime::Error> {
    for field in &info.fields {
        let oa = ptr_a as usize + field.offset as usize;
        let ob = ptr_b as usize + field.offset as usize;
        let eq = match field.kind {
            CmpFieldKind::I8 => {
                data.get(oa).copied() == data.get(ob).copied()
            }
            CmpFieldKind::I16 => {
                oa + 2 <= data.len() && ob + 2 <= data.len()
                    && data[oa..oa + 2] == data[ob..ob + 2]
            }
            CmpFieldKind::I32 | CmpFieldKind::F32 => {
                oa + 4 <= data.len() && ob + 4 <= data.len()
                    && data[oa..oa + 4] == data[ob..ob + 4]
            }
            CmpFieldKind::I64 | CmpFieldKind::F64 => {
                oa + 8 <= data.len() && ob + 8 <= data.len()
                    && data[oa..oa + 8] == data[ob..ob + 8]
            }
            CmpFieldKind::String => {
                if oa + 8 > data.len() || ob + 8 > data.len() {
                    return Err(wasmtime::Error::msg("rt_eq: out of bounds reading string field"));
                }
                let ptr1 = u32::from_le_bytes(data[oa..oa + 4].try_into().unwrap()) as usize;
                let len1 = u32::from_le_bytes(data[oa + 4..oa + 8].try_into().unwrap()) as usize;
                let ptr2 = u32::from_le_bytes(data[ob..ob + 4].try_into().unwrap()) as usize;
                let len2 = u32::from_le_bytes(data[ob + 4..ob + 8].try_into().unwrap()) as usize;
                if len1 != len2 {
                    false
                } else if len1 == 0 {
                    true
                } else if ptr1 + len1 > data.len() || ptr2 + len2 > data.len() {
                    return Err(wasmtime::Error::msg("rt_eq: out of bounds reading string data"));
                } else {
                    data[ptr1..ptr1 + len1] == data[ptr2..ptr2 + len2]
                }
            }
            CmpFieldKind::Interface => {
                if oa + 8 > data.len() || ob + 8 > data.len() {
                    return Err(wasmtime::Error::msg("rt_eq: out of bounds reading interface field"));
                }
                data[oa..oa + 8] == data[ob..ob + 8]
            }
            CmpFieldKind::Struct(nested_tid) => {
                let nested_tid = nested_tid as usize;
                if nested_tid >= layouts.len() {
                    return Ok(0);
                }
                let nested_info = &layouts[nested_tid];
                if !nested_info.comparable {
                    return Ok(0);
                }
                if oa + 4 > data.len() || ob + 4 > data.len() {
                    return Err(wasmtime::Error::msg("rt_eq: out of bounds reading nested struct pointer"));
                }
                let nested_a = u32::from_le_bytes(data[oa..oa + 4].try_into().unwrap()) as i32;
                let nested_b = u32::from_le_bytes(data[ob..ob + 4].try_into().unwrap()) as i32;
                if nested_a == nested_b {
                    true
                } else {
                    host_compare_fields(data, layouts, nested_info, nested_a, nested_b)? == 1
                }
            }
        };
        if !eq {
            return Ok(0);
        }
    }
    Ok(1)
}

impl UdfRuntime {
    pub fn new() -> Result<Self, wasmtime::Error> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.wasm_function_references(true);
        config.wasm_gc(true);

        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);

        Self::register_host_functions(&mut linker)?;

        Ok(Self { engine, linker })
    }

    fn register_host_functions(
        linker: &mut Linker<HostState>,
    ) -> Result<(), wasmtime::Error> {
        linker.func_wrap(
            "env",
            "ctx_log",
            |mut caller: wasmtime::Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), wasmtime::Error> {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                validate_read(&memory, &caller, ptr, len)?;
                let data = memory.data(&caller);
                let msg =
                    std::str::from_utf8(&data[ptr as usize..(ptr + len) as usize])
                        .map_err(|_| wasmtime::Error::msg("invalid UTF-8 in ctx_log message"))?
                        .to_string();
                caller.data_mut().logs.push(msg);
                Ok(())
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_query_id",
            |mut caller: wasmtime::Caller<'_, HostState>, out_ptr: i32| -> Result<i32, wasmtime::Error> {
                let query_id = caller.data().query_id.clone();
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                let bytes = query_id.as_bytes();
                validate_write(&memory, &caller, out_ptr, bytes.len())?;
                memory.data_mut(&mut caller)
                    [out_ptr as usize..out_ptr as usize + bytes.len()]
                    .copy_from_slice(bytes);
                Ok(bytes.len() as i32)
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_database",
            |mut caller: wasmtime::Caller<'_, HostState>, out_ptr: i32| -> Result<i32, wasmtime::Error> {
                let database = caller.data().database.clone();
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                let bytes = database.as_bytes();
                validate_write(&memory, &caller, out_ptr, bytes.len())?;
                memory.data_mut(&mut caller)
                    [out_ptr as usize..out_ptr as usize + bytes.len()]
                    .copy_from_slice(bytes);
                Ok(bytes.len() as i32)
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_schema",
            |mut caller: wasmtime::Caller<'_, HostState>, out_ptr: i32| -> Result<i32, wasmtime::Error> {
                let schema = caller.data().schema.clone();
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                let bytes = schema.as_bytes();
                validate_write(&memory, &caller, out_ptr, bytes.len())?;
                memory.data_mut(&mut caller)
                    [out_ptr as usize..out_ptr as usize + bytes.len()]
                    .copy_from_slice(bytes);
                Ok(bytes.len() as i32)
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_user",
            |mut caller: wasmtime::Caller<'_, HostState>, out_ptr: i32| -> Result<i32, wasmtime::Error> {
                let user = caller.data().user.clone();
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                let bytes = user.as_bytes();
                validate_write(&memory, &caller, out_ptr, bytes.len())?;
                memory.data_mut(&mut caller)
                    [out_ptr as usize..out_ptr as usize + bytes.len()]
                    .copy_from_slice(bytes);
                Ok(bytes.len() as i32)
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_config",
            |mut caller: wasmtime::Caller<'_, HostState>,
             key_ptr: i32,
             key_len: i32,
             out_ptr: i32|
             -> Result<i32, wasmtime::Error> {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                validate_read(&memory, &caller, key_ptr, key_len)?;
                let data = memory.data(&caller);
                let key = std::str::from_utf8(
                    &data[key_ptr as usize..(key_ptr + key_len) as usize],
                )
                .map_err(|_| wasmtime::Error::msg("invalid UTF-8 in config key"))?
                .to_string();
                let value =
                    caller.data().config.get(&key).cloned().unwrap_or_default();
                let bytes = value.as_bytes();
                validate_write(&memory, &caller, out_ptr, bytes.len())?;
                memory.data_mut(&mut caller)
                    [out_ptr as usize..out_ptr as usize + bytes.len()]
                    .copy_from_slice(bytes);
                Ok(bytes.len() as i32)
            },
        )?;

        linker.func_wrap(
            "env",
            "ctx_oom",
            |_caller: wasmtime::Caller<'_, HostState>| -> Result<(), wasmtime::Error> {
                Err(wasmtime::Error::msg(
                    "out of memory: WASM linear memory could not be grown",
                ))
            },
        )?;

        linker.func_wrap(
            "env",
            "nowUnixNano",
            |_caller: wasmtime::Caller<'_, HostState>| -> i64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as i64
            },
        )?;

        linker.func_wrap(
            "env",
            "monotonicNano",
            |caller: wasmtime::Caller<'_, HostState>| -> i64 {
                caller.data().monotonic_epoch.elapsed().as_nanos() as i64
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_streq",
            |mut caller: wasmtime::Caller<'_, HostState>,
             ptr1: i32, len1: i32, ptr2: i32, len2: i32| -> Result<i32, wasmtime::Error> {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                validate_read(&memory, &caller, ptr1, len1)?;
                validate_read(&memory, &caller, ptr2, len2)?;
                let data = memory.data(&caller);
                let s1 = &data[ptr1 as usize..(ptr1 + len1) as usize];
                let s2 = &data[ptr2 as usize..(ptr2 + len2) as usize];
                Ok(if s1 == s2 { 1 } else { 0 })
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_strcmp",
            |mut caller: wasmtime::Caller<'_, HostState>,
             ptr1: i32, len1: i32, ptr2: i32, len2: i32| -> Result<i32, wasmtime::Error> {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                validate_read(&memory, &caller, ptr1, len1)?;
                validate_read(&memory, &caller, ptr2, len2)?;
                let data = memory.data(&caller);
                let s1 = &data[ptr1 as usize..(ptr1 + len1) as usize];
                let s2 = &data[ptr2 as usize..(ptr2 + len2) as usize];
                Ok(match s1.cmp(s2) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_alloc",
            |mut caller: wasmtime::Caller<'_, HostState>, size: i32| -> Result<i32, wasmtime::Error> {
                host_alloc(&mut caller, size)
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_i64_to_str",
            |mut caller: wasmtime::Caller<'_, HostState>, val: i64| -> Result<(i32, i32), wasmtime::Error> {
                let s = val.to_string();
                host_write_string(&mut caller, s.as_bytes())
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_f64_to_str",
            |mut caller: wasmtime::Caller<'_, HostState>, val: f64| -> Result<(i32, i32), wasmtime::Error> {
                let s = if val.is_nan() {
                    "NaN".to_string()
                } else if val.is_infinite() {
                    if val.is_sign_positive() { "+Inf".to_string() } else { "-Inf".to_string() }
                } else {
                    let formatted = format!("{}", val);
                    if !formatted.contains('.') && !formatted.contains('e') && !formatted.contains('E') {
                        format!("{}.0", formatted)
                    } else {
                        formatted
                    }
                };
                host_write_string(&mut caller, s.as_bytes())
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_str_concat",
            |mut caller: wasmtime::Caller<'_, HostState>,
             ptr1: i32, len1: i32, ptr2: i32, len2: i32| -> Result<(i32, i32), wasmtime::Error> {
                let total = len1 as u32 + len2 as u32;
                if total == 0 {
                    return Ok((0, 0));
                }
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("missing memory export"))?;
                validate_read(&memory, &caller, ptr1, len1)?;
                validate_read(&memory, &caller, ptr2, len2)?;

                let mut buf = vec![0u8; total as usize];
                let data = memory.data(&caller);
                buf[..len1 as usize].copy_from_slice(&data[ptr1 as usize..(ptr1 + len1) as usize]);
                buf[len1 as usize..].copy_from_slice(&data[ptr2 as usize..(ptr2 + len2) as usize]);

                host_write_string(&mut caller, &buf)
            },
        )?;

        linker.func_wrap(
            "env",
            "rt_eq",
            |mut caller: wasmtime::Caller<'_, HostState>,
             tid: i32, ptr_a: i32, ptr_b: i32| -> Result<i32, wasmtime::Error> {
                if ptr_a == ptr_b {
                    return Ok(1);
                }
                let layouts = caller.data().type_layouts.clone();
                let tid = tid as usize;
                if tid >= layouts.len() {
                    return Ok(0);
                }
                let info = &layouts[tid];
                if !info.comparable {
                    return Ok(0);
                }
                let memory = get_memory(&mut caller)?;
                host_compare_fields(memory.data(&caller), &layouts, info, ptr_a, ptr_b)
            },
        )?;

        linker.func_wrap(
            "env",
            "Float64bits",
            |_caller: wasmtime::Caller<'_, HostState>, f: f64| -> i64 {
                f.to_bits() as i64
            },
        )?;

        linker.func_wrap(
            "env",
            "Float64frombits",
            |_caller: wasmtime::Caller<'_, HostState>, b: i64| -> f64 {
                f64::from_bits(b as u64)
            },
        )?;

        linker.func_wrap(
            "env",
            "Float32bits",
            |_caller: wasmtime::Caller<'_, HostState>, f: f32| -> i32 {
                f.to_bits() as i32
            },
        )?;

        linker.func_wrap(
            "env",
            "Float32frombits",
            |_caller: wasmtime::Caller<'_, HostState>, b: i32| -> f32 {
                f32::from_bits(b as u32)
            },
        )?;

        Ok(())
    }

    pub fn load_module(&self, wasm_bytes: &[u8]) -> Result<Module, wasmtime::Error> {
        Module::new(&self.engine, wasm_bytes)
    }

    pub fn create_store(
        &self,
        state: HostState,
        fuel: u64,
    ) -> Result<Store<HostState>, wasmtime::Error> {
        let mut store = Store::new(&self.engine, state);
        store.set_fuel(fuel)?;
        store.limiter(|s| &mut s.limits);
        Ok(store)
    }

    pub fn instantiate(
        &self,
        store: &mut Store<HostState>,
        module: &Module,
    ) -> Result<wasmtime::Instance, wasmtime::Error> {
        self.linker.instantiate(store, module)
    }
}
