// configure default clippy lints
#![deny(clippy::correctness)]
#![warn(clippy::complexity, clippy::style, clippy::perf, clippy::pedantic)]
// disable some pedantic lints
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::default_trait_access,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::non_ascii_literal,
    clippy::option_if_let_else,
    clippy::result_large_err, // TODO: investigate this
    clippy::similar_names,
    clippy::single_match_else,
    clippy::type_repetition_in_bounds,
    clippy::wildcard_imports
)]
// enable some restriction lints
#![warn(
    clippy::dbg_macro,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unimplemented,
    clippy::unwrap_used
)]

mod codegen;
mod compiled;
mod dependencies;
pub mod error;
mod functions;
mod parser;
mod util;

pub use codegen::{DataType, ExpressionAst, Parameter};
pub use compiled::LinkedExpression;
pub use dependencies::ExpressionDependencies;
pub use functions::FUNCTION_PREFIX;
pub use parser::ExpressionParser;

pub use geoengine_expression_deps::*;

/// Checks if a string is a valid variable name
pub fn is_allowed_variable_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(FUNCTION_PREFIX)
}