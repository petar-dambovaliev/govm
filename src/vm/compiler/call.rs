use crate::parser::ast::{Call, Expression, Selector};
use crate::vm::builtin::signature_from_t;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::symbols::{ContextType, DefineType};

#[derive(Debug)]
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

fn find_method(name: &str, methods: Vec<DefineType>) -> Option<(usize, DefineType)> {
    for (i, method) in methods.iter().enumerate() {
        let (m, _, _, _) = method.as_func();
        if m == name {
            return Some((i, method.clone()));
        }
    }
    None
}

fn find_method_by_name_with_path(
    strct: DefineType,
    name: &str,
    mut current_path: Vec<usize>,
) -> Option<(Vec<usize>, DefineType)> {
    let (_, children, m) = strct.as_struct().unwrap();
    if let Some((a, b)) = find_method(name, m) {
        current_path.push(a);
        return Some((current_path, b));
    }

    for (i, child) in children.iter().enumerate() {
        if let ContextType::Embedded(_, dt) = child {
            if dt.is_struct() {
                let mut child_path = current_path.clone();
                child_path.push(i);

                if let Some((path, method_name)) =
                    find_method_by_name_with_path(dt.clone(), name, child_path)
                {
                    return Some((path, method_name));
                }
            }
        }
    }

    None
}

impl CallType {
    pub fn from_call(call: &Call, c: &mut Compiler) -> Self {
        match call.func.as_ref() {
            Expression::Selector(sel) => {
                let sellt = c.compile_expression(&sel.x).unwrap().strip_var();
                let method_name = sel.sel.name.to_string();

                fn find_sel(
                    c: &mut Compiler,
                    sel: &Selector,
                    dt: DefineType,
                    method_name: String,
                ) -> CallType {
                    match dt.clone() {
                        DefineType::Ref(inner) => find_sel(c, sel, inner.strip_var(), method_name),
                        DefineType::Interface { methods, .. } => {
                            let m = find_method(&method_name, methods);

                            match m {
                                Some((i, m)) => CallType::DynamicDispatch {
                                    method_index: i,
                                    method_dt: m,
                                    iface_expr: *sel.x.clone(),
                                },
                                None => panic!("interface method not found"),
                            }
                        }
                        DefineType::Struct { name, .. } => {
                            let (_, _, methods) = c
                                .symbols
                                .resolve(&name)
                                .unwrap()
                                .get_type()
                                .as_struct()
                                .unwrap();

                            match find_method(&method_name, methods) {
                                Some((_, m)) => {
                                    return CallType::Method {
                                        mangled_name: CallType::make_method_name(
                                            dt.clone(),
                                            &method_name,
                                        ),
                                        method_name,
                                        struct_expr: *sel.x.clone(),
                                        method_dt: m,
                                        struct_dt: dt.clone(),
                                    };
                                }
                                None => panic!("struct method not found"),
                            }
                        }
                        _ => unimplemented!("call selector: {:#?}", dt),
                    }
                }
                find_sel(c, sel, sellt.clone().strip_var(), method_name)
            }
            Expression::Ident(id) => {
                let mut t = c
                    .symbols
                    .resolve(&id.name)
                    .unwrap_or_else(|| panic!("unresolved: {:#?}", id))
                    .get_type()
                    .strip_var();

                //panic!("{:#?}", t);

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
        let p = if dt.is_struct() {
            let (name, _, _) = dt.as_struct().unwrap();
            name
        } else {
            format!("{:#?}", dt)
        };
        format!("0x{:#?}{}", p, f_name)
    }
}
