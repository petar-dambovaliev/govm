use crate::parser::ast::{CompositeLit, Element, Expression, Ident};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::OpCode;
use crate::vm::symbols::{ContextType, DefineType, Scope};
use crate::vm::Error;

pub(crate) fn compile_struct(
    clit: &CompositeLit,
    name: &Ident,
    c: &mut Compiler,
) -> Result<DefineType, Error> {
    let (s, dt) = match c.symbols.resolve(name.name.as_str()) {
        Some(s) => s.as_local(),
        None => panic!("struct `{}` does not exist", name.name),
    };

    let (name, inner_types) = match dt {
        DefineType::Struct { name, fields, .. } => (name, fields),
        _ => panic!("expect struct"),
    };

    if let Some(ct) = inner_types.first() {
        if let ContextType::Unnamed(_) = ct {
            panic!("expected named or embedded type");
        }
    }

    let opcode = if s.scope == Scope::Global {
        OpCode::GetGlobal
    } else {
        OpCode::GetLocal
    };
    c.emit_opcode(opcode);
    c.emit_u16(s.index);

    //sort by order of definition
    let mut clit_values = clit.val.values.clone();

    let key_required = clit
        .val
        .values
        .first()
        .map(|a| a.key.is_some())
        .unwrap_or_default();

    if key_required {
        clit_values.sort_by_key(|val| {
            inner_types
                .iter()
                .map(|inner_type| match inner_type {
                    ContextType::Named(s, d) => (s, d),
                    ContextType::Embedded(s, d) => (s, d),
                    _ => panic!(),
                })
                .position(|x| {
                    assert_eq!(key_required, val.key.is_some(), "val: {:#?}", val);
                    let k_el = val.key.as_ref().unwrap();
                    let k = match k_el {
                        Element::Expr(expr) => expr.as_ident().unwrap().clone(),
                        _ => panic!("ident"),
                    };

                    x.0 == k.name.as_str()
                })
        });
    } else {
        assert_eq!(inner_types.len(), clit_values.len());

        for (clit_value, ct) in clit_values.iter_mut().zip(inner_types.clone()) {
            clit_value.key = Some(Element::Expr(Expression::Ident(Ident {
                pos: 0,
                name: ct.as_named().unwrap().0,
            })));
        }
    }

    for inner_type in inner_types.iter().rev() {
        let (kk, inner_type) = match inner_type {
            ContextType::Named(s, d) => (s.clone(), d.clone()),
            ContextType::Embedded(s, d) => (s.clone(), d.clone()),
            _ => panic!(),
        };

        let found = clit_values.iter().find(|a| {
            let k = a.key.as_ref().unwrap();

            let id = match k {
                Element::Expr(expr) => expr.clone(),
                _ => panic!("expr"),
            }
            .as_ident()
            .unwrap()
            .clone();
            id.name == kk
        });

        match found {
            Some(kel) => {
                //compile values
                let el_expr = match &kel.val {
                    Element::Expr(expr) => expr.clone(),
                    _ => panic!("expr"),
                };

                let rt = c.compile_expression(&el_expr)?;

                let in_t = match inner_type {
                    DefineType::Type(a, _b) => *a,
                    _ => inner_type.clone(),
                };

                if !(in_t.is_nullable() && rt.is_nil()) {
                    if rt.is_const_coerceable_to(&in_t) {
                        match in_t {
                            DefineType::Float32 => {
                                c.emit_opcode(OpCode::CastToFloat32);
                                c.emit_u8(0);
                            }
                            DefineType::Float64 => {
                                c.emit_opcode(OpCode::CastToFloat64);
                                c.emit_u8(0);
                            }
                            t => unimplemented!("cannot coerce: {:#?}", t),
                        }
                    } else {
                        let r = rt.strip_const().strip_var().strip_type();
                        if in_t.is_struct() && r.is_struct() {
                            assert!(DefineType::eq_structs(&in_t, &r));
                        } else {
                            assert_eq!(in_t, r, "{:#?}", el_expr);
                        }
                    }
                }
            }
            None => {
                let def_val = c.make_type_default_val(inner_type);
                let _ = c.compile_expression(&def_val)?;
            }
        }
    }

    // let obj = Object::string(name.clone(), &mut self.gc);
    // let idx = self.add_constant(obj);
    // self.emit_opcode(OpCode::Const);
    // self.emit_u16(idx);

    c.emit_opcode(OpCode::Struct);
    c.emit_u16(inner_types.len().try_into().unwrap());
    return Ok(DefineType::Struct {
        name,
        fields: inner_types,
        methods: vec![],
    });
}
