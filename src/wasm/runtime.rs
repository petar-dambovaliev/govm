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
        }
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

impl UdfRuntime {
    pub fn new() -> Result<Self, wasmtime::Error> {
        let mut config = Config::new();
        config.consume_fuel(true);

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
            "time_now_unix_nano",
            |_caller: wasmtime::Caller<'_, HostState>| -> i64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as i64
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
