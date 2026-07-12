use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

fn has_named_field(input: &DeriveInput, name: &str) -> bool {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .any(|f| f.ident.as_ref().is_some_and(|ident| ident == name)),
            _ => false,
        },
        _ => false,
    }
}

/// Derives `engine::scene::SceneClock` by pointing its one required method,
/// `clock_mut`, at the struct's `clock` field.
///
/// Emits `crate::engine::scene::{SceneClock, Clock}` paths, so it only works
/// inside the two bin trees that declare `mod engine;` — which is every user
/// this repo-internal crate will ever have.
#[proc_macro_derive(SceneClock)]
pub fn derive_scene_clock(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    if !has_named_field(&input, "clock") {
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

/// Derives `engine::scene::SceneOrbitalBodies`. Unlike the other scene
/// derives, a missing `orbital_bodies` field is not an error: the impl then
/// returns the empty slice - how a scene that tracks no orbital bodies
/// still satisfies the trait. Same crate-internal-paths caveat as
/// [`derive_scene_clock`].
#[proc_macro_derive(SceneOrbitalBodies)]
pub fn derive_scene_orbital_bodies(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let body = if has_named_field(&input, "orbital_bodies") {
        quote! { &mut self.orbital_bodies }
    } else {
        quote! { &mut [] }
    };

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics crate::engine::scene::SceneOrbitalBodies for #name #ty_generics #where_clause {
            fn orbital_bodies_mut(
                &mut self,
            ) -> &mut [crate::engine::scene::orbital_body::OrbitalBody] {
                #body
            }
        }
    }
    .into()
}

/// Derives `engine::scene::SceneKinematicBodies`; the exact
/// missing-field-is-empty twin of [`derive_scene_orbital_bodies`] for the
/// `kinematic_bodies` field.
#[proc_macro_derive(SceneKinematicBodies)]
pub fn derive_scene_kinematic_bodies(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let body = if has_named_field(&input, "kinematic_bodies") {
        quote! { &mut self.kinematic_bodies }
    } else {
        quote! { &mut [] }
    };

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics crate::engine::scene::SceneKinematicBodies for #name #ty_generics #where_clause {
            fn kinematic_bodies_mut(
                &mut self,
            ) -> &mut [crate::engine::scene::kinematic_body::KinematicBody] {
                #body
            }
        }
    }
    .into()
}

/// Derives `engine::camera::ScenePtzCamera` by pointing its three accessors
/// at the struct's `camera` and `camera_target` fields. Same
/// crate-internal-paths caveat as [`derive_scene_clock`].
#[proc_macro_derive(ScenePtzCamera)]
pub fn derive_scene_ptz_camera(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    if !has_named_field(&input, "camera") || !has_named_field(&input, "camera_target") {
        return syn::Error::new_spanned(
            &input.ident,
            "#[derive(ScenePtzCamera)] requires a struct with named `camera: PtzCamera` and \
             `camera_target: CameraTarget` fields",
        )
        .to_compile_error()
        .into();
    }

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics crate::engine::camera::ScenePtzCamera for #name #ty_generics #where_clause {
            fn camera(&self) -> &crate::engine::camera::PtzCamera {
                &self.camera
            }

            fn camera_mut(&mut self) -> &mut crate::engine::camera::PtzCamera {
                &mut self.camera
            }

            fn camera_target(&self) -> &crate::engine::scene::CameraTarget {
                &self.camera_target
            }
        }
    }
    .into()
}
