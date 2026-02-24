use crate::symbols::DefineType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmType {
    I32,
    I64,
    F32,
    F64,
    Ref(u32),
}

impl WasmType {
    pub fn to_val_type(self) -> wasm_encoder::ValType {
        match self {
            WasmType::I32 => wasm_encoder::ValType::I32,
            WasmType::I64 => wasm_encoder::ValType::I64,
            WasmType::F32 => wasm_encoder::ValType::F32,
            WasmType::F64 => wasm_encoder::ValType::F64,
            WasmType::Ref(type_idx) => wasm_encoder::ValType::Ref(
                wasm_encoder::RefType {
                    nullable: true,
                    heap_type: wasm_encoder::HeapType::Concrete(type_idx),
                }
            ),
        }
    }

    pub fn byte_size(self) -> u32 {
        match self {
            WasmType::I32 | WasmType::F32 => 4,
            WasmType::I64 | WasmType::F64 => 8,
            WasmType::Ref(_) => 4,
        }
    }

    pub fn is_gc_ref(self) -> bool {
        matches!(self, WasmType::Ref(_))
    }

    pub fn gc_type_idx(self) -> Option<u32> {
        match self {
            WasmType::Ref(idx) => Some(idx),
            _ => None,
        }
    }
}

pub fn define_type_to_wasm(dt: &DefineType) -> Vec<WasmType> {
    match dt {
        DefineType::Bool | DefineType::Byte | DefineType::Int8 | DefineType::Int16
        | DefineType::Int32 | DefineType::Uint8 | DefineType::Uint16 | DefineType::Uint32
        | DefineType::Uintptr | DefineType::Rune => vec![WasmType::I32],

        DefineType::Int | DefineType::Int64 | DefineType::Uint | DefineType::Uint64 => {
            vec![WasmType::I64]
        }

        DefineType::Float32 => vec![WasmType::F32],
        DefineType::Float64 => vec![WasmType::F64],

        // Strings are (ptr, len) in linear memory
        DefineType::String => vec![WasmType::I32, WasmType::I32],

        // Pointers, structs, maps, interfaces, closures are i32 pointers into linear memory
        DefineType::Ref(_)
        | DefineType::Struct { .. }
        | DefineType::Map(_, _)
        | DefineType::Interface { .. }
        | DefineType::Spec { .. } => vec![WasmType::I32],

        // Slices are (ptr, len, cap)
        DefineType::Slice(_) => vec![WasmType::I32, WasmType::I32, WasmType::I32],

        // Arrays are pointers into linear memory
        DefineType::Array { .. } => vec![WasmType::I32],

        DefineType::Null => vec![WasmType::I32],

        // Qualified types unwrap
        DefineType::Qualified(_, inner) => define_type_to_wasm(inner),
        DefineType::Type(inner, _) => define_type_to_wasm(inner),

        // Functions are represented as (func_idx, env_ptr)
        DefineType::Func { .. } => vec![WasmType::I32, WasmType::I32],

        DefineType::Variadic(inner) => define_type_to_wasm(&DefineType::Slice(inner.clone())),

        DefineType::Tuple(types) => {
            types.iter().flat_map(define_type_to_wasm).collect()
        }

        DefineType::Channel(_) | DefineType::Iter(_) | DefineType::Complex64
        | DefineType::Complex128 | DefineType::Package { .. } => {
            vec![WasmType::I32]
        }
    }
}

#[derive(Debug, Clone)]
pub struct StructLayout {
    pub fields: Vec<FieldLayout>,
    pub total_size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone)]
pub struct FieldLayout {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub wasm_type: WasmType,
}

fn align_to(offset: u32, alignment: u32) -> u32 {
    (offset + alignment - 1) & !(alignment - 1)
}

pub fn compute_struct_layout(fields: &[(String, DefineType)]) -> StructLayout {
    let mut field_layouts = Vec::with_capacity(fields.len());
    let mut offset: u32 = 0;
    let mut max_align: u32 = 1;

    for (name, dt) in fields {
        let wasm_types = define_type_to_wasm(dt);
        for (i, wt) in wasm_types.iter().enumerate() {
            let size = wt.byte_size();
            let alignment = size;
            offset = align_to(offset, alignment);
            if alignment > max_align {
                max_align = alignment;
            }

            let field_name = if wasm_types.len() > 1 {
                format!("{}_{}", name, i)
            } else {
                name.clone()
            };

            field_layouts.push(FieldLayout {
                name: field_name,
                offset,
                size,
                wasm_type: *wt,
            });
            offset += size;
        }
    }

    let total_size = align_to(offset, max_align);

    StructLayout {
        fields: field_layouts,
        total_size,
        alignment: max_align,
    }
}
