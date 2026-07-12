use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Derives `engine::scene::SceneClock` by pointing its one required method,
/// `clock_mut`, at the struct's `clock` field.
///
/// Emits `crate::engine::scene::{SceneClock, Clock}` paths, so it only works
/// inside the two bin trees that declare `mod engine;` — which is every user
/// this repo-internal crate will ever have.
#[proc_macro_derive(SceneClock)]
pub fn derive_scene_clock(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let has_clock_field = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .any(|f| f.ident.as_ref().is_some_and(|ident| ident == "clock")),
            _ => false,
        },
        _ => false,
    };
    if !has_clock_field {
        return syn::Error::new_spanned(
            &input.ident,
            "#[derive(SceneClock)] requires a struct with a named `clock: Clock` field",
        )
        .to_compile_error()
        .into();
    }

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics crate::engine::scene::SceneClock for #name #ty_generics #where_clause {
            fn clock_mut(&mut self) -> &mut crate::engine::scene::Clock {
                &mut self.clock
            }
        }
    }
    .into()
}
