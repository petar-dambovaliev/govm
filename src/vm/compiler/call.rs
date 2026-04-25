use crate::parser::ast::{Call, Expression, Selector};
use crate::vm::builtin::signature_from_t;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::{make_ident_name, make_method_name};
use crate::vm::symbols::DefineType;
use crate::vm::Error;

#[derive(Debug)]
#[allow(dead_code)]
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
        iface_name: String,
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
    pub fn from_call(pkg: &str, call: &Call, c: &mut Compiler) -> Result<(Self, String), Error> {
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

                let sellt = c.compile_expression(pkg, &sel.x)?;
                let method_name = sel.sel.name.to_string();

                fn find_sel(
                    c: &mut Compiler,
                    pkg: &str,
                    sel: &Selector,
                    dt: DefineType,
                    method_name: String,
                ) -> Result<(CallType, String), Error> {
                    match dt.clone() {
                        DefineType::Ref(inner) => {
                            find_sel(c, pkg, sel, *inner, method_name)
                        }
                        DefineType::Interface { name: ref iname, methods, .. } => {
                            let m = find_method(&method_name, methods);

                            match m {
                                Some((i, m)) => Ok((
                                    CallType::DynamicDispatch {
                                        method_index: i,
                                        method_dt: m,
                                        iface_expr: *sel.x.clone(),
                                        iface_name: iname.clone(),
                                    },
                                    pkg.to_string(),
                                )),
                                None => Err(Error::ReferenceError(format!(
                                    "interface method '{}' not found",
                                    method_name
                                ))),
                            }
                        }
                        DefineType::Struct { name, .. } => {
                            let resolved = c
                                .symbols
                                .resolve(pkg, &name)
                                .ok_or_else(|| {
                                    Error::ReferenceError(format!(
                                        "unresolved struct '{}'",
                                        name
                                    ))
                                })?;
                            let (_, _, methods) = resolved
                                .get_type()
                                .0
                                .as_struct()?;

                            match find_method(&method_name, methods.clone()) {
                                Some((_, m)) => {
                                    return Ok((
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
                                    ));
                                }
                                None => {
                                    return Err(Error::ReferenceError(format!(
                                        "method '{}' not found in struct '{}'",
                                        method_name, name
                                    )));
                                }
                            }
                        }
                        DefineType::Spec { name, .. } => {
                            let resolved = c
                                .symbols
                                .resolve(pkg, &name)
                                .ok_or_else(|| {
                                    Error::ReferenceError(format!(
                                        "unresolved spec '{}'",
                                        name
                                    ))
                                })?;
                            let (_, _, methods, _) = resolved
                                .get_type()
                                .0
                                .as_spec()?;

                            match find_method(&method_name, methods.clone()) {
                                Some((_, m)) => {
                                    return Ok((
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
                                    ));
                                }
                                None => {
                                    let n = make_method_name(pkg, dt.clone(), &method_name);
                                    match c.symbols.resolve(pkg, &n) {
                                        Some(s) => {
                                            let m = s.get_type();
                                            return Ok((
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
                                            ));
                                        }
                                        None => {
                                            return Err(Error::ReferenceError(format!(
                                                "cannot find function '{}'",
                                                method_name
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        DefineType::Package { path, alias: _alias } => {
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
                        _ => {
                            return Err(Error::TypeError(format!(
                                "unsupported call selector type: {:?}",
                                dt
                            )));
                        }
                    }
                }

                find_sel(
                    c,
                    pkg,
                    sel,
                    sellt.clone().unwrap_to_base_type(),
                    method_name,
                )
            }
            Expression::Ident(id) => {
                let resolved = c
                    .symbols
                    .resolve(pkg, &id.name)
                    .ok_or_else(|| {
                        Error::ReferenceError(format!(
                            "unresolved identifier '{}' in package '{}'",
                            id.name, pkg
                        ))
                    })?;
                let mut t = resolved.get_type().0;

                if !t.is_type() {
                    if !t.is_func() {
                        return Err(Error::TypeError(format!(
                            "expected function, got {:?} for '{}' in '{}'",
                            t, id.name, pkg
                        )));
                    }
                } else {
                    t = signature_from_t(t).ok_or_else(|| {
                        Error::TypeError(format!(
                            "failed to get signature for type '{}'",
                            id.name
                        ))
                    })?;
                }

                return Ok((
                    Self::Func {
                        name: make_ident_name(pkg, &id.name),
                        func_dt: t,
                        expr: *call.func.clone(),
                    },
                    pkg.to_string(),
                ));
            }
            _ => {
                return Err(Error::SyntaxError(format!(
                    "unsupported call expression: {:?}",
                    call
                )));
            }
        }
    }
}
