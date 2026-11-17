//! Dónde guarda P-Rune sus datos. Lo usan todas las interfaces, así comparten el historial:
//! lo que aplica una se puede deshacer desde la otra.

use std::path::{Path, PathBuf};

/// Nombre de la carpeta de la app en `%APPDATA%`. Es el `identifier` de la app de escritorio
/// (P-Rune, en su `tauri.conf.json`): tiene que ser el mismo para que las dos, cuando no
/// están instaladas, guarden el historial en el mismo lugar.
pub const APP_ID: &str = "com.debloater.app";

/// `%APPDATA%\com.debloater.app` según Windows (no según variables de entorno, que se
/// pueden cambiar). Es la misma carpeta que Tauri llama `app_data_dir`.
pub fn user_data_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(|s| PathBuf::from(s).join(APP_ID))
    }
}

/// `<instalación>\data` si este exe está instalado en Program Files; None si no (en
/// desarrollo o como exe suelto). No crea la carpeta ni mira si se puede escribir.
pub fn installed_data_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let pf = crate::winget::program_files()?.to_string_lossy().to_ascii_lowercase();
    if !exe_dir.to_string_lossy().to_ascii_lowercase().starts_with(&format!("{pf}\\")) {
        return None;
    }
    Some(exe_dir.join("data"))
}

/// Carpeta de instalación por defecto de la app de escritorio dentro de Program Files, y su
/// ejecutable (`productName` y `mainBinaryName` de su `tauri.conf.json`).
const DESKTOP_DIR: &str = "P-Rune";
const DESKTOP_EXE: &str = "P-Rune.exe";

/// `<P-Rune>\data` si la app de escritorio está instalada en su carpeta por defecto.
///
/// P-Rune Core es otro programa, pero si en la misma PC está la app, las dos tienen que ver
/// el mismo historial: si no, lo que aplica una no se podría deshacer desde la otra. Solo se
/// mira en Program Files (ahí escriben solo los administradores) y no en el registro ni en
/// otra carpeta: una instalación en otro lugar no se detecta y Core usa su propio historial.
pub fn desktop_data_dir() -> Option<PathBuf> {
    let dir = crate::winget::program_files()?.join(DESKTOP_DIR);
    dir.join(DESKTOP_EXE).is_file().then(|| dir.join("data"))
}

/// La carpeta protegida del historial, si en esta PC hay una: la de la app de escritorio si
/// está instalada (así Core, instalado o suelto, comparte su historial) y, si no, la de este
/// exe si está en Program Files. Para la app instalada en su carpeta, las dos son la misma.
pub fn protected_data_dir() -> Option<PathBuf> {
    desktop_data_dir().or_else(installed_data_dir)
}

/// Dónde guardar el historial.
///
/// Instalada en Program Files y corriendo como admin, va en `<instalación>\data`: ahí solo
/// escriben los administradores, así que ningún programa del usuario puede manipularlo.
/// La primera vez se trae el historial que hubiera en `%APPDATA%` (igual se valida contra el
/// catálogo antes de deshacer). En desarrollo, sin admin, queda en `%APPDATA%`.
pub fn journal_dir(user_dir: &Path) -> PathBuf {
    let protected = (|| {
        if !crate::system::is_elevated() {
            return None;
        }
        let dir = protected_data_dir()?;
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })();
    let Some(dir) = protected else { return user_dir.to_path_buf() };
    let new = dir.join("journal.json");
    // La 0.1.0 se llamaba "Prune" y se instalaba en Program Files\Prune: su historial
    // (solo escribible por admins, igual que este) se trae a la carpeta nueva.
    let legacy = crate::winget::program_files().map(|pf| pf.join("Prune").join("data").join("journal.json"));
    if !new.exists() {
        if let Some(legacy) = legacy.filter(|p| p.exists() && *p != new) {
            if std::fs::copy(&legacy, &new).is_ok() {
                let _ = std::fs::rename(&legacy, legacy.with_file_name("journal.migrado.json"));
            }
        }
    }
    let old = user_dir.join("journal.json");
    if !new.exists() && old.exists() && std::fs::copy(&old, &new).is_ok() {
        let _ = std::fs::rename(&old, user_dir.join("journal.migrado.json"));
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carpeta_de_datos_del_usuario() {
        let dir = user_data_dir().expect("Windows siempre tiene %APPDATA%");
        assert!(dir.is_absolute());
        assert_eq!(dir.file_name().unwrap(), APP_ID);
    }
}
