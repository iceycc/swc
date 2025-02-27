use crate::debug::dump;
use std::f64;
use swc_atoms::js_word;
use swc_common::DUMMY_SP;
use swc_ecma_ast::*;
use swc_ecma_transforms_base::ext::MapWithMut;
use swc_ecma_utils::{ExprExt, Id, UsageFinder, Value};
use swc_ecma_visit::VisitWith;
use unicode_xid::UnicodeXID;

/// Creates `!e` where e is the expression passed as an argument.
///
/// # Note
///
/// This method returns `!e` if `!!e` is given as a argument.
///
/// TODO: Handle special cases like !1 or !0
pub(super) fn negate(e: &mut Expr, in_bool_ctx: bool) {
    let start_str = dump(&*e);

    match e {
        Expr::Bin(bin @ BinExpr { op: op!("=="), .. })
        | Expr::Bin(bin @ BinExpr { op: op!("!="), .. })
        | Expr::Bin(bin @ BinExpr { op: op!("==="), .. })
        | Expr::Bin(bin @ BinExpr { op: op!("!=="), .. }) => {
            bin.op = match bin.op {
                op!("==") => {
                    op!("!=")
                }
                op!("!=") => {
                    op!("==")
                }
                op!("===") => {
                    op!("!==")
                }
                op!("!==") => {
                    op!("===")
                }
                _ => {
                    unreachable!()
                }
            };
            log::debug!("negate: binary");
            return;
        }

        Expr::Bin(BinExpr {
            left,
            right,
            op: op @ op!("&&"),
            ..
        }) if is_ok_to_negate_rhs(&right) => {
            log::debug!("negate: a && b => !a || !b");

            negate(&mut **left, in_bool_ctx);
            negate(&mut **right, in_bool_ctx);
            *op = op!("||");
            return;
        }

        Expr::Bin(BinExpr {
            left,
            right,
            op: op @ op!("||"),
            ..
        }) if is_ok_to_negate_rhs(&right) => {
            log::debug!("negate: a || b => !a && !b");

            negate(&mut **left, in_bool_ctx);
            negate(&mut **right, in_bool_ctx);
            *op = op!("&&");
            return;
        }

        Expr::Cond(CondExpr { cons, alt, .. })
            if is_ok_to_negate_for_cond(&cons) && is_ok_to_negate_for_cond(&alt) =>
        {
            log::debug!("negate: cond");

            negate(&mut **cons, in_bool_ctx);
            negate(&mut **alt, in_bool_ctx);
            return;
        }

        Expr::Seq(SeqExpr { exprs, .. }) => {
            if let Some(last) = exprs.last_mut() {
                log::debug!("negate: seq");

                negate(&mut **last, in_bool_ctx);
                return;
            }
        }

        _ => {}
    }

    let mut arg = Box::new(e.take());

    match &mut *arg {
        Expr::Unary(UnaryExpr {
            op: op!("!"), arg, ..
        }) => match &mut **arg {
            Expr::Unary(UnaryExpr { op: op!("!"), .. }) => {
                log::debug!("negate: !!bool => !bool");
                *e = *arg.take();
                return;
            }
            Expr::Bin(BinExpr { op: op!("in"), .. })
            | Expr::Bin(BinExpr {
                op: op!("instanceof"),
                ..
            }) => {
                log::debug!("negate: !bool => bool");
                *e = *arg.take();
                return;
            }
            _ => {
                if in_bool_ctx {
                    log::debug!("negate: !expr => expr (in bool context)");
                    *e = *arg.take();
                    return;
                }
            }
        },

        _ => {}
    }

    log::debug!("negate: e => !e");

    *e = Expr::Unary(UnaryExpr {
        span: DUMMY_SP,
        op: op!("!"),
        arg,
    });

    if cfg!(feature = "debug") {
        log::trace!("[Change] Negated `{}` as `{}`", start_str, dump(&*e));
    }
}

pub(crate) fn is_ok_to_negate_for_cond(e: &Expr) -> bool {
    match e {
        Expr::Update(..) => false,
        _ => true,
    }
}

pub(crate) fn is_ok_to_negate_rhs(rhs: &Expr) -> bool {
    match rhs {
        Expr::Member(..) => true,
        Expr::Bin(BinExpr {
            op: op!("===") | op!("!==") | op!("==") | op!("!="),
            ..
        }) => true,

        Expr::Call(..) | Expr::New(..) => false,

        Expr::Update(..) => false,

        Expr::Bin(BinExpr {
            op: op!("&&") | op!("||"),
            left,
            right,
            ..
        }) => is_ok_to_negate_rhs(&left) && is_ok_to_negate_rhs(&right),

        Expr::Bin(BinExpr { left, right, .. }) => {
            is_ok_to_negate_rhs(&left) && is_ok_to_negate_rhs(&right)
        }

        Expr::Assign(e) => is_ok_to_negate_rhs(&e.right),

        Expr::Unary(UnaryExpr {
            op: op!("!") | op!("delete"),
            ..
        }) => true,

        Expr::Seq(e) => {
            if let Some(last) = e.exprs.last() {
                is_ok_to_negate_rhs(&last)
            } else {
                true
            }
        }

        Expr::Cond(e) => is_ok_to_negate_rhs(&e.cons) && is_ok_to_negate_rhs(&e.alt),

        _ => {
            if !rhs.may_have_side_effects() {
                return true;
            }

            if cfg!(feature = "debug") {
                log::warn!("unimplemented: is_ok_to_negate_rhs: `{}`", dump(&*rhs));
            }

            false
        }
    }
}

/// A negative value means that it's efficient to negate the expression.
pub(crate) fn negate_cost(e: &Expr, in_bool_ctx: bool, is_ret_val_ignored: bool) -> Option<isize> {
    fn cost(
        e: &Expr,
        in_bool_ctx: bool,
        bin_op: Option<BinaryOp>,
        is_ret_val_ignored: bool,
    ) -> isize {
        match e {
            Expr::Unary(UnaryExpr {
                op: op!("!"), arg, ..
            }) => {
                // TODO: Check if this argument is actually start of line.
                match &**arg {
                    Expr::Call(CallExpr {
                        callee: ExprOrSuper::Expr(callee),
                        ..
                    }) => match &**callee {
                        Expr::Fn(..) => return 0,
                        _ => {}
                    },
                    _ => {}
                }

                if in_bool_ctx {
                    let c = -cost(arg, true, None, is_ret_val_ignored);
                    return c.min(-1);
                }

                match &**arg {
                    Expr::Unary(UnaryExpr { op: op!("!"), .. }) => -1,

                    _ => 1,
                }
            }
            Expr::Bin(BinExpr {
                op: op!("===") | op!("!==") | op!("==") | op!("!="),
                ..
            }) => 0,

            Expr::Bin(BinExpr {
                op: op @ op!("||") | op @ op!("&&"),
                left,
                right,
                ..
            }) => {
                let l_cost = cost(&left, in_bool_ctx, Some(*op), false);

                if !is_ret_val_ignored && !is_ok_to_negate_rhs(&right) {
                    return l_cost + 3;
                }
                l_cost + cost(&right, in_bool_ctx, Some(*op), is_ret_val_ignored)
            }

            Expr::Cond(CondExpr { cons, alt, .. })
                if is_ok_to_negate_for_cond(&cons) && is_ok_to_negate_for_cond(&alt) =>
            {
                cost(&cons, in_bool_ctx, bin_op, is_ret_val_ignored)
                    + cost(&alt, in_bool_ctx, bin_op, is_ret_val_ignored)
            }

            Expr::Cond(..)
            | Expr::Update(..)
            | Expr::Bin(BinExpr {
                op: op!("in") | op!("instanceof"),
                ..
            }) => 3,

            Expr::Assign(..) => {
                if is_ret_val_ignored {
                    0
                } else {
                    3
                }
            }

            Expr::Seq(e) => {
                if let Some(last) = e.exprs.last() {
                    return cost(&last, in_bool_ctx, bin_op, is_ret_val_ignored);
                }

                if is_ret_val_ignored {
                    0
                } else {
                    1
                }
            }

            _ => {
                if is_ret_val_ignored {
                    0
                } else {
                    1
                }
            }
        }
    }

    let cost = cost(e, in_bool_ctx, None, is_ret_val_ignored);

    if cfg!(feature = "debug") {
        log::trace!("negation cost of `{}` = {}", dump(&*e), cost);
    }

    Some(cost)
}

pub(crate) fn is_pure_undefined(e: &Expr) -> bool {
    match e {
        Expr::Ident(Ident {
            sym: js_word!("undefined"),
            ..
        }) => true,

        Expr::Unary(UnaryExpr {
            op: UnaryOp::Void,
            arg,
            ..
        }) if !arg.may_have_side_effects() => true,

        _ => false,
    }
}

pub(crate) fn is_valid_identifier(s: &str, ascii_only: bool) -> bool {
    if ascii_only {
        if s.chars().any(|c| !c.is_ascii()) {
            return false;
        }
    }

    s.starts_with(|c: char| c.is_xid_start())
        && s.chars().all(|c: char| c.is_xid_continue())
        && !s.contains("𝒶")
        && !s.is_reserved()
}

pub(crate) fn get_lhs_ident(e: &PatOrExpr) -> Option<&Ident> {
    match e {
        PatOrExpr::Expr(v) => match &**v {
            Expr::Ident(i) => Some(i),
            _ => None,
        },
        PatOrExpr::Pat(v) => match &**v {
            Pat::Ident(i) => Some(&i.id),
            Pat::Expr(v) => match &**v {
                Expr::Ident(i) => Some(i),
                _ => None,
            },
            _ => None,
        },
    }
}

pub(crate) fn get_lhs_ident_mut(e: &mut PatOrExpr) -> Option<&mut Ident> {
    match e {
        PatOrExpr::Expr(v) => match &mut **v {
            Expr::Ident(i) => Some(i),
            _ => None,
        },
        PatOrExpr::Pat(v) => match &mut **v {
            Pat::Ident(i) => Some(&mut i.id),
            Pat::Expr(v) => match &mut **v {
                Expr::Ident(i) => Some(i),
                _ => None,
            },
            _ => None,
        },
    }
}

pub(crate) fn is_directive(e: &Stmt) -> bool {
    match e {
        Stmt::Expr(s) => match &*s.expr {
            Expr::Lit(Lit::Str(Str { value, .. })) => value.starts_with("use "),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn is_pure_undefined_or_null(e: &Expr) -> bool {
    is_pure_undefined(e)
        || match e {
            Expr::Lit(Lit::Null(..)) => true,
            _ => false,
        }
}

/// This method does **not** modifies `e`.
///
/// This method is used to test if a whole call can be replaced, while
/// preserving standalone constants.
pub(crate) fn eval_as_number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Bin(BinExpr {
            op: op!(bin, "-"),
            left,
            right,
            ..
        }) => {
            let l = eval_as_number(&left)?;
            let r = eval_as_number(&right)?;

            return Some(l - r);
        }

        Expr::Call(CallExpr {
            callee: ExprOrSuper::Expr(callee),
            args,
            ..
        }) => {
            for arg in &*args {
                if arg.spread.is_some() || arg.expr.may_have_side_effects() {
                    return None;
                }
            }

            match &**callee {
                Expr::Member(MemberExpr {
                    obj: ExprOrSuper::Expr(obj),
                    prop,
                    computed: false,
                    ..
                }) => {
                    let prop = match &**prop {
                        Expr::Ident(i) => i,
                        _ => return None,
                    };

                    match &**obj {
                        Expr::Ident(obj) if &*obj.sym == "Math" => match &*prop.sym {
                            "cos" => {
                                let v = eval_as_number(&args.first()?.expr)?;

                                return Some(v.cos());
                            }
                            "sin" => {
                                let v = eval_as_number(&args.first()?.expr)?;

                                return Some(v.sin());
                            }

                            "max" => {
                                let mut numbers = vec![];
                                for arg in args {
                                    let v = eval_as_number(&arg.expr)?;
                                    if v.is_infinite() || v.is_nan() {
                                        return None;
                                    }
                                    numbers.push(v);
                                }

                                return Some(
                                    numbers
                                        .into_iter()
                                        .max_by(|a, b| a.partial_cmp(b).unwrap())
                                        .unwrap_or(f64::NEG_INFINITY),
                                );
                            }

                            "min" => {
                                let mut numbers = vec![];
                                for arg in args {
                                    let v = eval_as_number(&arg.expr)?;
                                    if v.is_infinite() || v.is_nan() {
                                        return None;
                                    }
                                    numbers.push(v);
                                }

                                return Some(
                                    numbers
                                        .into_iter()
                                        .min_by(|a, b| a.partial_cmp(b).unwrap())
                                        .unwrap_or(f64::INFINITY),
                                );
                            }

                            "pow" => {
                                if args.len() != 2 {
                                    return None;
                                }
                                let first = eval_as_number(&args[0].expr)?;
                                let second = eval_as_number(&args[1].expr)?;

                                return Some(first.powf(second));
                            }

                            _ => {}
                        },
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        Expr::Member(MemberExpr {
            obj: ExprOrSuper::Expr(obj),
            prop,
            computed: false,
            ..
        }) => {
            let prop = match &**prop {
                Expr::Ident(i) => i,
                _ => return None,
            };

            match &**obj {
                Expr::Ident(obj) if &*obj.sym == "Math" => match &*prop.sym {
                    "PI" => return Some(f64::consts::PI),
                    "E" => return Some(f64::consts::E),
                    "LN10" => return Some(f64::consts::LN_10),
                    _ => {}
                },
                _ => {}
            }
        }

        _ => {
            if let Value::Known(v) = e.as_number() {
                return Some(v);
            }
        }
    }

    None
}

pub(crate) fn always_terminates(s: &Stmt) -> bool {
    match s {
        Stmt::Return(..) | Stmt::Throw(..) | Stmt::Break(..) | Stmt::Continue(..) => true,
        Stmt::If(IfStmt { cons, alt, .. }) => {
            always_terminates(&cons) && alt.as_deref().map(always_terminates).unwrap_or(false)
        }
        Stmt::Block(s) => s.stmts.iter().any(always_terminates),

        _ => false,
    }
}

pub(crate) fn is_ident_used_by<N>(id: Id, node: &N) -> bool
where
    N: for<'aa> VisitWith<UsageFinder<'aa>>,
{
    UsageFinder::find(&Ident::new(id.0, DUMMY_SP.with_ctxt(id.1)), node)
}

#[cfg(test)]
mod tests {
    use super::negate_cost;
    use swc_common::{input::SourceFileInput, FileName};
    use swc_ecma_parser::{lexer::Lexer, Parser};

    fn assert_negate_cost(s: &str, in_bool_ctx: bool, is_ret_val_ignored: bool, expected: isize) {
        testing::run_test2(false, |cm, _| {
            let fm = cm.new_source_file(FileName::Anon, s.to_string());

            let lexer = Lexer::new(
                Default::default(),
                swc_ecma_ast::EsVersion::latest(),
                SourceFileInput::from(&*fm),
                None,
            );

            let mut parser = Parser::new_from(lexer);

            let e = parser
                .parse_expr()
                .expect("failed to parse input as an expression");

            let actual = negate_cost(&e, in_bool_ctx, is_ret_val_ignored).unwrap();

            assert_eq!(
                actual, expected,
                "Expected negation cost of {} to be {}, but got {}",
                s, expected, actual,
            );

            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn logical_1() {
        assert_negate_cost(
            "this[key] && !this.hasOwnProperty(key) || (this[key] = value)",
            false,
            true,
            2,
        );
    }

    #[test]
    #[ignore]
    fn logical_2() {
        assert_negate_cost(
            "(!this[key] || this.hasOwnProperty(key)) && (this[key] = value)",
            false,
            true,
            -2,
        );
    }
}
