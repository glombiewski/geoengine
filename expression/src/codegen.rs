use super::error::{self, ExpressionParserError};
use crate::functions::Function;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use snafu::ensure;
use std::{collections::BTreeSet, fmt::Debug, hash::Hash};

type Result<T, E = ExpressionParserError> = std::result::Result<T, E>;

// TODO: prefix for variables and functions

/// An expression as an abstract syntax tree.
/// Allows genering Rust code.
#[derive(Debug, Clone)]
pub struct ExpressionAst {
    /// This name is the generated function name after generating code.
    name: Identifier,
    root: AstNode,
    parameters: Vec<Parameter>,
    out_type: DataType,
    functions: BTreeSet<AstFunction>,
}

impl ExpressionAst {
    pub fn new(
        name: Identifier,
        parameters: Vec<Parameter>,
        out_type: DataType,
        functions: BTreeSet<AstFunction>,
        root: AstNode,
    ) -> Result<ExpressionAst> {
        ensure!(!name.as_ref().is_empty(), error::EmptyExpressionName);

        Ok(Self {
            name,
            root,
            parameters,
            out_type,
            functions,
        })
    }

    pub fn code(&self) -> String {
        self.to_token_stream().to_string()
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }
}

impl ToTokens for ExpressionAst {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        for function in &self.functions {
            function.to_tokens(tokens);
        }

        let fn_name = &self.name;
        let params: Vec<TokenStream> = self
            .parameters
            .iter()
            .map(|p| {
                let param = p.identifier();
                let dtype = p.data_type();
                quote! { #param: Option<#dtype> }
            })
            .collect();
        let content = &self.root;

        let dtype = self.out_type;

        tokens.extend(quote! {
            #[no_mangle]
            pub extern "Rust" fn #fn_name (#(#params),*) -> Option<#dtype> {
                #content
            }
        });
    }
}

#[derive(Debug, Clone)]
pub enum AstNode {
    Constant(f64),
    NoData,
    Variable {
        name: Identifier,
        data_type: DataType,
    },
    // Operation {
    //     left: Box<AstNode>,
    //     op: AstOperator,
    //     right: Box<AstNode>,
    // },
    Function {
        function: Function,
        args: Vec<AstNode>,
    },
    Branch {
        condition_branches: Vec<Branch>,
        else_branch: Box<AstNode>,
    },
    AssignmentsAndExpression {
        assignments: Vec<Assignment>,
        expression: Box<AstNode>,
    },
}

impl AstNode {
    pub fn data_type(&self) -> DataType {
        match self {
            // - only support number constants
            // - no data is a number for now
            Self::Constant(_) | Self::NoData => DataType::Number,

            Self::Variable { data_type, .. } => *data_type,

            Self::Function { function, .. } => function.output_type(),

            // we have to check beforehand that all branches have the same type
            Self::Branch { else_branch, .. } => else_branch.data_type(),

            Self::AssignmentsAndExpression { expression, .. } => expression.data_type(),
        }
    }

    ///// Outputs the required variables for this node.
    // TODO: reverse to input available variables?
    // TODO: speed-up by caching intermediate results?
    // fn required_vars<'s>(&'s self, vars: &mut HashSet<&'s Identifier>) {
    //     match self {
    //         AstNode::Constant(_) | AstNode::NoData => {}
    //         AstNode::Variable { name, .. } => {
    //             vars.insert(name);
    //         }
    //         // AstNode::Operation { left, op: _, right } => {
    //         //     left.required_vars(vars);
    //         //     right.required_vars(vars);
    //         // }
    //         AstNode::Function { args, .. } => {
    //             for arg in args {
    //                 arg.required_vars(vars);
    //             }
    //         }
    //         AstNode::Branch {
    //             condition_branches,
    //             else_branch,
    //         } => {
    //             for branch in condition_branches {
    //                 branch.body.required_vars(vars);
    //             }
    //             else_branch.required_vars(vars);
    //         }
    //         AstNode::AssignmentsAndExpression {
    //             assignments,
    //             expression,
    //         } => {
    //             let mut candidate_vars = HashSet::new();
    //             expression.required_vars(&mut candidate_vars);

    //             let exclusion_vars: HashSet<&Identifier> = assignments
    //                 .iter()
    //                 .map(|assignment| &assignment.identifier)
    //                 .collect();

    //             // only output variables that were not covered by the assignments
    //             vars.extend(candidate_vars.difference(&exclusion_vars));
    //         }
    //     }
    // }
}

impl ToTokens for AstNode {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let new_tokens = match self {
            Self::Constant(n) => quote! { Some(#n) },
            Self::NoData => quote! { None },
            Self::Variable { name, .. } => quote! { #name },
            // Self::Operation { left, op, right } => {
            //     quote! { apply(#left, #right, #op) }
            // }
            Self::Function { function, args } => {
                let fn_name = function.name();
                quote! { #fn_name(#(#args),*) }
            }
            AstNode::Branch {
                condition_branches,
                else_branch: default_branch,
            } => {
                let mut new_tokens = TokenStream::new();
                for (i, branch) in condition_branches.iter().enumerate() {
                    let condition = &branch.condition;
                    let body = &branch.body;

                    new_tokens.extend(if i == 0 {
                        // first
                        quote! {
                            if #condition {
                                #body
                            }
                        }
                    } else {
                        // middle
                        quote! {
                            else if #condition {
                                #body
                            }
                        }
                    });
                }

                new_tokens.extend(quote! {
                    else {
                        #default_branch
                    }
                });

                new_tokens
            }
            Self::AssignmentsAndExpression {
                assignments,
                expression,
            } => {
                quote! {
                    #(#assignments)*
                    #expression
                }
            }
        };

        tokens.extend(new_tokens);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Identifier(String);

impl ToTokens for Identifier {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let identifier = format_ident!("{}", self.0);
        tokens.extend(quote! { #identifier });
    }
}

impl From<String> for Identifier {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Identifier {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<&String> for Identifier {
    fn from(s: &String) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Identifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        std::fmt::Display::fmt(&self.0, f)
    }
}

// #[derive(Debug, Clone)]
// pub enum AstOperator {
//     Add,
//     Subtract,
//     Multiply,
//     Divide,
// }

// impl ToTokens for AstOperator {
//     fn to_tokens(&self, tokens: &mut TokenStream) {
//         let new_tokens = match self {
//             AstOperator::Add => quote! { std::ops::Add::add },
//             AstOperator::Subtract => quote! { std::ops::Sub::sub },
//             AstOperator::Multiply => quote! { std::ops::Mul::mul },
//             AstOperator::Divide => quote! { std::ops::Div::div },
//         };

//         tokens.extend(new_tokens);
//     }
// }

#[derive(Debug, Clone)]
pub struct Branch {
    pub condition: BooleanExpression,
    pub body: AstNode,
}

#[derive(Debug, Clone)]
pub enum BooleanExpression {
    Constant(bool),
    Comparison {
        left: Box<AstNode>,
        op: BooleanComparator,
        right: Box<AstNode>,
    },
    Operation {
        left: Box<BooleanExpression>,
        op: BooleanOperator,
        right: Box<BooleanExpression>,
    },
}

impl ToTokens for BooleanExpression {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let new_tokens = match self {
            Self::Constant(b) => quote! { #b },
            Self::Comparison { left, op, right } => quote! { ((#left) #op (#right)) },
            Self::Operation { left, op, right } => quote! { ( (#left) #op (#right) ) },
        };

        tokens.extend(new_tokens);
    }
}

#[derive(Debug, Clone)]
pub enum BooleanComparator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

impl ToTokens for BooleanComparator {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let new_tokens = match self {
            Self::Equal => quote! { == },
            Self::NotEqual => quote! { != },
            Self::LessThan => quote! { < },
            Self::LessThanOrEqual => quote! { <= },
            Self::GreaterThan => quote! { > },
            Self::GreaterThanOrEqual => quote! { >= },
        };

        tokens.extend(new_tokens);
    }
}

#[derive(Debug, Clone)]
pub enum BooleanOperator {
    And,
    Or,
}

impl ToTokens for BooleanOperator {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let new_tokens = match self {
            Self::And => quote! { && },
            Self::Or => quote! { || },
        };

        tokens.extend(new_tokens);
    }
}

#[derive(Debug, Clone)]
pub struct Assignment {
    pub identifier: Identifier,
    pub expression: AstNode,
}

impl ToTokens for Assignment {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            identifier,
            expression,
        } = self;
        let new_tokens = quote! {
            let #identifier = #expression;
        };

        tokens.extend(new_tokens);
    }
}

// TODO: make parameters case insensitive
#[derive(Debug, Clone)]
pub enum Parameter {
    Number(Identifier),
    MultiPoint(Identifier),
    MultiLineString(Identifier),
    MultiPolygon(Identifier),
}

impl AsRef<str> for Parameter {
    fn as_ref(&self) -> &str {
        match self {
            Self::Number(identifier)
            | Self::MultiPoint(identifier)
            | Self::MultiLineString(identifier)
            | Self::MultiPolygon(identifier) => identifier.as_ref(),
        }
    }
}

impl Parameter {
    pub fn identifier(&self) -> &Identifier {
        match self {
            Self::Number(identifier)
            | Self::MultiPoint(identifier)
            | Self::MultiLineString(identifier)
            | Self::MultiPolygon(identifier) => identifier,
        }
    }

    pub fn data_type(&self) -> DataType {
        match self {
            Self::Number(_) => DataType::Number,
            Self::MultiPoint(_) => DataType::MultiPoint,
            Self::MultiLineString(_) => DataType::MultiLineString,
            Self::MultiPolygon(_) => DataType::MultiPolygon,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataType {
    Number,
    MultiPoint,
    MultiLineString,
    MultiPolygon,
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let s = match self {
            Self::Number => "number",
            Self::MultiPoint => "geometry (multipoint)",
            Self::MultiLineString => "geometry (multilinestring)",
            Self::MultiPolygon => "geometry (multipolygon)",
        };

        write!(f, "{s}")
    }
}

impl ToTokens for DataType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Number => quote! { f64 },
            Self::MultiPoint => {
                quote! { geo::geometry::MultiPoint<geo::Point<f64>> }
            }
            Self::MultiLineString => {
                quote! { geo::geometry::MultiLineString<geo::LineString<f64>> }
            }
            Self::MultiPolygon => {
                quote! { geo::geometry::MultiPolygon<geo::Polygon<f64>> }
            }
        });
    }
}

impl DataType {
    pub fn group_name(&self) -> &str {
        match self {
            Self::Number => "number",
            Self::MultiPoint | Self::MultiLineString | Self::MultiPolygon => "geometry",
        }
    }

    /// A unique short name without spaces, etc.
    pub fn call_name_suffix(self) -> char {
        match self {
            Self::Number => 'n',
            Self::MultiPoint => 'p',
            Self::MultiLineString => 'l',
            Self::MultiPolygon => 'q',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AstFunction {
    pub function: Function,
}

impl ToTokens for AstFunction {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let function = &self.function;
        tokens.extend(quote! {
            #[inline]
            #function
        });
    }
}
