//! ES2017: Async / Await
//!
//! This plugin transforms async functions to generator functions.
//!
//! ## Example
//!
//! Input:
//! ```js
//! async function foo() {
//!   await bar();
//! }
//! const foo2 = async () => {
//!   await bar();
//! };
//! ```
//!
//! Output:
//! ```js
//! function foo() {
//!   return _asyncToGenerator(this, null, function* () {
//!     yield bar();
//!   })
//! }
//! const foo2 = () => _asyncToGenerator(this, null, function* () {
//!   yield bar();
//! }
//! ```
//!
//! ## Implementation
//!
//! Implementation based on [@babel/plugin-transform-async-to-generator](https://babel.dev/docs/babel-plugin-transform-async-to-generator) and [esbuild](https://github.com/evanw/esbuild/blob/main/internal/js_parser/js_parser_lower.go#L392).
//!
//!
//! Reference:
//! * Babel docs: <https://babeljs.io/docs/en/babel-plugin-transform-async-to-generator>
//! * Esbuild implementation: <https://github.com/evanw/esbuild/blob/main/internal/js_parser/js_parser_lower.go#L392>
//! * Babel implementation: <https://github.com/babel/babel/blob/main/packages/babel-plugin-transform-async-to-generator>
//! * Babel helper implementation: <https://github.com/babel/babel/blob/main/packages/babel-helper-remap-async-to-generator>
//! * Async / Await TC39 proposal: <https://github.com/tc39/proposal-async-await>
//!

use crate::context::Ctx;
use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    ArrowFunctionExpression, Expression, FormalParameterKind, Function, Statement, YieldExpression,
};
use oxc_ast::NONE;
use oxc_span::{Atom, SPAN};
use oxc_syntax::reference::ReferenceFlags;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

pub struct AsyncToGenerator<'a> {
    _ctx: Ctx<'a>,
}

impl<'a> AsyncToGenerator<'a> {
    pub fn new(ctx: Ctx<'a>) -> Self {
        Self { _ctx: ctx }
    }

    fn get_helper_callee(symbol_id: Option<SymbolId>, ctx: &mut TraverseCtx<'a>) -> Expression<'a> {
        let ident = ctx.create_reference_id(
            SPAN,
            Atom::from("babelHelpers"),
            symbol_id,
            ReferenceFlags::Read,
        );
        let object = ctx.ast.expression_from_identifier_reference(ident);
        let property = ctx.ast.identifier_name(SPAN, Atom::from("asyncToGenerator"));
        Expression::from(ctx.ast.member_expression_static(SPAN, object, property, false))
    }
}

impl<'a> Traverse<'a> for AsyncToGenerator<'a> {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a>) {
        if let Expression::AwaitExpression(await_expr) = expr {
            // Do not transform top-level await
            if ctx.ancestry.ancestors().any(|ancestor| {
                matches!(
                    ancestor,
                    Ancestor::FunctionBody(_) | Ancestor::ArrowFunctionExpressionBody(_)
                )
            }) {
                let yield_expression = YieldExpression {
                    span: SPAN,
                    delegate: false,
                    argument: Some(await_expr.argument.clone_in(ctx.ast.allocator)),
                };
                let expression = ctx.ast.alloc(yield_expression);
                *expr = Expression::YieldExpression(expression);
            }
        }
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut TraverseCtx<'a>) {
        let babel_helpers_id = ctx.scopes().find_binding(ctx.current_scope_id(), "babelHelpers");
        let callee = Self::get_helper_callee(babel_helpers_id, ctx);
        let mut target = func.clone_in(ctx.ast.allocator);
        target.r#async = false;
        target.generator = true;
        target.params = ctx.ast.alloc(ctx.ast.formal_parameters(
            SPAN,
            FormalParameterKind::FormalParameter,
            ctx.ast.vec(),
            NONE,
        ));
        let parameters = {
            let mut items = ctx.ast.vec();
            items.push(ctx.ast.argument_expression(ctx.ast.expression_this(SPAN)));
            items.push(ctx.ast.argument_expression(ctx.ast.expression_null_literal(SPAN)));
            items.push(ctx.ast.argument_expression(ctx.ast.expression_from_function(target)));
            items
        };
        let call = ctx.ast.expression_call(SPAN, callee, NONE, parameters, false);
        let returns = ctx.ast.return_statement(SPAN, Some(call));
        let body = Statement::ReturnStatement(ctx.ast.alloc(returns));
        let body = ctx.ast.function_body(SPAN, ctx.ast.vec(), ctx.ast.vec1(body));
        let body = ctx.ast.alloc(body);
        func.body = Some(body);
    }

    fn exit_arrow_function_expression(
        &mut self,
        arrow: &mut ArrowFunctionExpression<'a>,
        ctx: &mut TraverseCtx<'a>,
    ) {
        let babel_helpers_id = ctx.scopes().find_binding(ctx.current_scope_id(), "babelHelpers");
        let callee = Self::get_helper_callee(babel_helpers_id, ctx);
        let mut target = arrow.clone_in(ctx.ast.allocator);
        target.r#async = false;
        target.params = ctx.ast.alloc(ctx.ast.formal_parameters(
            SPAN,
            FormalParameterKind::FormalParameter,
            ctx.ast.vec(),
            NONE,
        ));
        let parameters = {
            let mut items = ctx.ast.vec();
            items.push(ctx.ast.argument_expression(ctx.ast.expression_this(SPAN)));
            items.push(ctx.ast.argument_expression(ctx.ast.expression_null_literal(SPAN)));
            items.push(ctx.ast.argument_expression(ctx.ast.expression_from_arrow_function(target)));
            items
        };
        let call = ctx.ast.expression_call(SPAN, callee, NONE, parameters, false);
        let returns = ctx.ast.return_statement(SPAN, Some(call));
        let body = Statement::ReturnStatement(ctx.ast.alloc(returns));
        let body = ctx.ast.function_body(SPAN, ctx.ast.vec(), ctx.ast.vec1(body));
        arrow.body = ctx.ast.alloc(body);
    }
}
