use crate::parser::ast::{Call, Expression, Selector};
use crate::vm::builtin::signature_from_t;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::{make_ident_name, make_method_name};
use crate::vm::symbols::{ContextType, DefineType};

#[derive(Debug)]
pub enum CallType {
    Func {
        name: String,
        func_dt: DefineType,
        expr: Expression,
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

impl CallType {
    pub fn from_call(pkg: &str, call: &Call, c: &mut Compiler) -> (Self, String) {
        match call.func.as_ref() {
            Expression::Selector(sel) => {
                if let Expression::Ident(id) = sel.x.as_ref() {
                    if let Some(path) = c.symbols.get_package_path(&id.name) {
                        return CallType::from_call(
                            &path,
                            &Call {
                                pos: (0, 0),
                                args: vec![],
                                func: Box::new(Expression::Ident(sel.sel.clone())),
                                dots: None,
                            },
                            c,
                        );
                    }
                }

                let sellt = c.compile_expression(pkg, &sel.x).unwrap().strip_var();
                let method_name = sel.sel.name.to_string();

                fn find_sel(
                    c: &mut Compiler,
                    pkg: &str,
                    sel: &Selector,
                    dt: DefineType,
                    method_name: String,
                ) -> (CallType, String) {
                    match dt.clone() {
                        DefineType::Ref(inner) => {
                            find_sel(c, pkg, sel, inner.strip_var(), method_name)
                        }
                        DefineType::Interface { methods, .. } => {
                            let m = find_method(&method_name, methods);

                            match m {
                                Some((i, m)) => (
                                    CallType::DynamicDispatch {
                                        method_index: i,
                                        method_dt: m,
                                        iface_expr: *sel.x.clone(),
                                    },
                                    pkg.to_string(),
                                ),
                                None => panic!("interface method not found"),
                            }
                        }
                        DefineType::Struct { name, .. } => {
                            let (_, _, methods) = c
                                .symbols
                                .resolve(pkg, &name)
                                .unwrap()
                                .get_type()
                                .0
                                .as_struct()
                                .unwrap();

                            match find_method(&method_name, methods.clone()) {
                                Some((_, m)) => {
                                    return (
                                        CallType::Method {
                                            mangled_name: make_method_name(
                                                pkg,
                                                dt.clone(),
                                                &method_name,
                                            ),
                                            method_name: method_name.clone(),
                                            struct_expr: *sel.x.clone(),
                                            method_dt: m,
                                            struct_dt: dt.clone(),
                                        },
                                        pkg.to_string(),
                                    );
                                }
                                None => {
                                    panic!(
                                        "method '{}' not found in struct: {} methods: {:#?}",
                                        method_name, name, methods
                                    )
                                }
                            }
                        }
                        DefineType::Spec { name, .. } => {
                            let (_, _, methods, _) = c
                                .symbols
                                .resolve(pkg, &name)
                                .unwrap()
                                .get_type()
                                .0
                                .as_spec()
                                .unwrap();

                            match find_method(&method_name, methods.clone()) {
                                Some((_, m)) => {
                                    return (
                                        CallType::Method {
                                            mangled_name: make_method_name(
                                                pkg,
                                                dt.clone(),
                                                &method_name,
                                            ),
                                            method_name: method_name.clone(),
                                            struct_expr: *sel.x.clone(),
                                            method_dt: m,
                                            struct_dt: dt.clone(),
                                        },
                                        pkg.to_string(),
                                    );
                                }
                                None => {
                                    let n = make_method_name(pkg, dt.clone(), &method_name);
                                    match c.symbols.resolve(pkg, &n) {
                                        Some(s) => {
                                            let m = s.get_type();
                                            return (
                                                CallType::Method {
                                                    mangled_name: make_method_name(
                                                        pkg,
                                                        dt.clone(),
                                                        &method_name,
                                                    ),
                                                    method_name: method_name.clone(),
                                                    struct_expr: *sel.x.clone(),
                                                    method_dt: m.0,
                                                    struct_dt: dt.clone(),
                                                },
                                                pkg.to_string(),
                                            );
                                        }
                                        None => panic!("cannot find function {}", method_name),
                                    }
                                }
                            }
                        }
                        DefineType::Package { path, alias } => {
                            return CallType::from_call(
                                &path,
                                &Call {
                                    pos: (0, 0),
                                    args: vec![],
                                    func: sel.x.clone(),
                                    dots: None,
                                },
                                c,
                            );
                        }
                        _ => unimplemented!("call selector: {:#?}", dt),
                    }
                }

                find_sel(
                    c,
                    pkg,
                    sel,
                    sellt.clone().strip_var().strip_const(),
                    method_name,
                )
            }
            Expression::Ident(id) => {
                let mut t = c
                    .symbols
                    .resolve(pkg, &id.name)
                    .unwrap_or_else(|| panic!("unresolved: {:#?} pkg: {}", id, pkg))
                    .get_type()
                    .0
                    .strip_var();

                if !t.is_type() {
                    assert!(t.is_func(), "{}+{}->{:#?}", pkg, id.name, t);
                } else {
                    t = signature_from_t(t).unwrap();
                }

                return (
                    Self::Func {
                        name: make_ident_name(pkg, &id.name),
                        func_dt: t,
                        expr: *call.func.clone(),
                    },
                    pkg.to_string(),
                );
            }
            _ => unimplemented!("call: {:#?}", call),
        }
    }
}
