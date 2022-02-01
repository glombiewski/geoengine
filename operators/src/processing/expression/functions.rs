use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::{collections::HashMap, ops::RangeInclusive};

pub(super) struct Function {
    pub num_args: RangeInclusive<usize>,
    pub token_fn: fn(usize, Ident) -> TokenStream,
}

lazy_static::lazy_static! {
    pub(super) static ref FUNCTIONS: HashMap<&'static str, Function> = {
        let mut functions = HashMap::new();

        functions.insert("min", Function {
            num_args: 2..=2,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64, b: f64) -> f64 {
                    f64::min(a, b)
                }
            },
        });

        functions.insert("max", Function {
            num_args: 2..=2,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64, b: f64) -> f64 {
                    f64::max(a, b)
                }
            },
        });

        functions.insert("abs", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::abs(a)
                }
            },
        });

        functions.insert("pow", Function {
            num_args: 2..=2,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64, b: f64) -> f64 {
                    f64::powf(a, b)
                }
            },
        });

        functions.insert("sqrt", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::sqrt(a)
                }
            },
        });

        functions.insert("cos", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::cos(a)
                }
            },
        });

        functions.insert("sin", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::sin(a)
                }
            },
        });

        functions.insert("tan", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::tan(a)
                }
            },
        });

        functions.insert("acos", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::acos(a)
                }
            },
        });

        functions.insert("asin", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::asin(a)
                }
            },
        });

        functions.insert("atan", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::atan(a)
                }
            },
        });

        functions.insert("log10", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::log10(a)
                }
            },
        });

        functions.insert("ln", Function {
            num_args: 1..=1,
            token_fn: |_, fn_name| quote! {
                fn #fn_name(a: f64) -> f64 {
                    f64::ln(a)
                }
            },
        });

        functions.insert("pi", Function {
            num_args: 0..=0,
            token_fn: |_, fn_name| quote! {
                fn #fn_name() -> f64 {
                    std::f64::consts::PI
                }
            },
        });

        functions
    };
}
