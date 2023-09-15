use crate::parser::ast::{Call, Expression};
use crate::vm::builtin::signature_from_t;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::symbols::DefineType;

pub enum CallType {
    Func {
        name: String,
        func_dt: DefineType,
    },
    Method {
        mangled_name: String,
        method_name: String,
        struct_expr: Expression,
        method_dt: DefineType,
        struct_dt: DefineType,
    },
    DynamicDispatch {
        method_index: usize,
        method_dt: DefineType,
        iface_expr: Expression,
    },
}

fn find_method(name: &str, methods: Vec<DefineType>) -> Option<(DefineType, usize)> {
    for (i, method) in methods.iter().enumerate() {
        let (m, _, _, _) = method.as_func();
        if m == name {
            return Some((method.clone(), i));
        }
    }
    None
}

impl CallType {
    pub fn from_call(call: &Call, c: &mut Compiler) -> Self {
        match call.func.as_ref() {
            Expression::Selector(sel) => {
                let sellt = c
                    .symbols
                    .resolve(&sel.x.as_ident().unwrap().name)
                    .unwrap()
                    .get_type()
                    .strip_var();

                let method_name = sel.sel.name.to_string();

                match sellt.clone() {
                    DefineType::Interface { name, methods } => {
                        let m = find_method(&method_name, methods);

                        match m {
                            Some((m, i)) => Self::DynamicDispatch {
                                method_index: i,
                                method_dt: m,
                                iface_expr: *sel.x.clone(),
                            },
                            None => panic!("interface method not found"),
                        }
                    }
                    DefineType::Struct { name, methods, .. } => {
                        let (_, _, methods) = c
                            .symbols
                            .resolve(&name)
                            .unwrap()
                            .get_type()
                            .as_struct()
                            .unwrap();

                        match find_method(&method_name, methods) {
                            Some((m, _)) => {
                                return Self::Method {
                                    mangled_name: Self::make_method_name(
                                        sellt.clone(),
                                        &method_name,
                                    ),
                                    method_name,
                                    struct_expr: *sel.x.clone(),
                                    method_dt: m,
                                    struct_dt: sellt.clone(),
                                };
                            }
                            None => panic!("struct method not found"),
                        }
                    }
                    _ => unimplemented!("call selector: {:#?}", sellt),
                }
            }
            Expression::Ident(id) => {
                let mut t = c
                    .symbols
                    .resolve(&id.name)
                    .unwrap_or_else(|| panic!("unresolved: {:#?}", id))
                    .get_type()
                    .strip_var();

                if !t.is_type() {
                    assert!(t.is_func());
                } else {
                    t = signature_from_t(t).unwrap();
                }

                return Self::Func {
                    name: id.name.to_string(),
                    func_dt: t,
                };
            }
            _ => unimplemented!("call: {:#?}", call),
        }
    }
    fn make_method_name(dt: DefineType, f_name: &str) -> String {
        format!("0x{:#?}{}", dt, f_name)
    }
}
