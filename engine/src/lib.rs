//! Motor de P-Rune, sin interfaz: catálogo, tweaks, historial y acceso a Windows.
//!
//! Lo usa P-Rune Core (`src/`) y es el mismo que tiene la app de escritorio, P-Rune. Acá no hay nada
//! de Tauri: cada interfaz decide cómo mostrar y cómo pedir confirmación.

/// Mensaje traducido: `t!("clave")` o `t!("clave", nombre = valor, ...)` (ver `i18n`).
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::msg($key)
    };
    ($key:literal, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::fmt($key, &[$((stringify!($name), &$val as &dyn ::std::fmt::Display)),+])
    };
}

pub mod appx;
pub mod catalog;
pub mod cleanup;
pub mod com;
pub mod disk;
pub mod engine;
pub mod http;
pub mod i18n;
pub mod icons;
pub mod network;
pub mod paths;
pub mod power;
pub mod profile;
pub mod proxy;
pub mod registry;
pub mod restore;
pub mod sign;
pub mod startup;
pub mod system;
pub mod user;
pub mod verify;
pub mod vt;
pub mod winget;
pub mod wmo;
pub mod wupdate;
