//! Dónde guarda P-Rune Core el historial, y con qué permisos se lee cada uno (ver
//! `journal_dir`). Como administrador comparte el de la app de escritorio, si está instalada:
//! lo que aplica una se puede deshacer desde la otra.

use std::path::PathBuf;

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

/// Carpeta propia de Core en Program Files, para el historial de administrador cuando Core
/// no está instalado ni está la app de escritorio.
const CORE_DIR: &str = "P-Rune Core";

/// La carpeta del historial según con qué permisos corre el programa (sin crear nada).
///
/// Hay dos historiales, y cada uno se lee y se escribe solo con el nivel de permisos con el
/// que se creó:
/// - **Como administrador**, siempre en Program Files, donde solo escriben los
///   administradores: la carpeta de la app de escritorio si está instalada, la de este exe si
///   está instalado, o `Program Files\P-Rune Core\data`.
/// - **Sin administrador**, en `%APPDATA%`.
///
/// Un proceso de administrador nunca lee, importa ni escribe el de `%APPDATA%`: ahí escribe
/// cualquier programa del usuario, y deshacer es volver a escribir lo que el historial dice
/// (DNS, registro, inicio de Windows). Con un historial plantado, deshacer como administrador
/// sería hacerle el trabajo a quien lo plantó.
fn journal_dir_for(elevated: bool, protected: Option<PathBuf>, program_files: Option<PathBuf>, user: Option<PathBuf>) -> Option<PathBuf> {
    if elevated { protected.or_else(|| program_files.map(|pf| pf.join(CORE_DIR).join("data"))) } else { user }
}

/// La carpeta del historial de administrador de esta PC, si ya existe. Es para avisarle a
/// quien mira sin administrador que hay otro historial; no crea nada ni lo lee (la carpeta
/// propia de Core no se puede leer sin administrador: ver `admin_only`).
pub fn admin_journal_dir() -> Option<PathBuf> {
    let dir = journal_dir_for(true, protected_data_dir(), crate::winget::program_files(), None)?;
    dir.is_dir().then_some(dir)
}

/// Deja una carpeta (y lo que tenga adentro) solo para Administradores y SYSTEM.
///
/// En Program Files los usuarios no pueden escribir, pero sí leer, y con solo tener abierto
/// un archivo ajeno ya se le puede trabar a otro: quien abriera `journal.lock` o
/// `journal.json` sin soltarlos dejaría a Core sin poder anotar ningún cambio.
fn admin_only(dir: &std::path::Path) -> Result<(), String> {
    // P = protegida (no hereda de Program Files); acceso total solo a SYSTEM y Administradores.
    set_dacl(dir, "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
}

fn set_dacl(dir: &std::path::Path, sddl: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows::Win32::Security::{ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::core::{BOOL, PCWSTR};

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (sddl, path) = (wide(sddl), wide(&dir.to_string_lossy()));
    unsafe {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None).map_err(|e| e.message())?;
        let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let result = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted).map_err(|e| e.message()).and_then(|()| {
            // Con PROTECTED, lo que hay adentro pasa a heredar de esta carpeta y no de Program Files.
            SetNamedSecurityInfoW(PCWSTR(path.as_ptr()), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, None, None, Some(dacl), None)
                .ok()
                .map_err(|e| e.message())
        });
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        result
    }
}

/// Dónde guardar el historial (ver `journal_dir_for`). Como administrador crea la carpeta si
/// hace falta; si no se puede, es un error: no se usa otra en su lugar.
pub fn journal_dir(elevated: bool) -> Result<PathBuf, String> {
    let dir = journal_dir_for(elevated, protected_data_dir(), crate::winget::program_files(), user_data_dir())
        .ok_or_else(|| t!("journal_save_failed", error = "Program Files / %APPDATA%"))?;
    if !elevated {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| t!("journal_save_failed", error = e))?;
    // La carpeta propia de Core se cierra a los usuarios. La de la app de escritorio no se
    // toca: es de la app. Si no se puede cerrar, el historial funciona igual.
    if desktop_data_dir().is_none() {
        let _ = admin_only(&dir);
    }
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
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Como administrador nunca se usa la carpeta del usuario, ni siquiera si no hay otra.
    #[test]
    fn el_historial_de_admin_nunca_va_en_la_carpeta_del_usuario() {
        let pf = PathBuf::from(r"C:\Program Files");
        let user = PathBuf::from(r"C:\Users\x\AppData\Roaming\com.debloater.app");
        let app = PathBuf::from(r"C:\Program Files\P-Rune\data");
        // Con la app de escritorio (o Core) instalada: su carpeta.
        assert_eq!(journal_dir_for(true, Some(app.clone()), Some(pf.clone()), Some(user.clone())), Some(app.clone()));
        // Sin nada instalado: la propia de Core, también en Program Files.
        assert_eq!(journal_dir_for(true, None, Some(pf.clone()), Some(user.clone())), Some(pf.join("P-Rune Core").join("data")));
        // Si no se puede saber dónde está Program Files, no hay carpeta: no se cae en la del usuario.
        assert_eq!(journal_dir_for(true, None, None, Some(user.clone())), None);
        // Sin administrador, siempre la del usuario, aunque haya una protegida.
        assert_eq!(journal_dir_for(false, Some(app), Some(pf), Some(user.clone())), Some(user));
    }

    /// Cambiar los permisos de una carpeta funciona y lo de adentro sigue usable para quien
    /// queda con acceso. (Con los permisos de verdad, solo administradores, este test no podría
    /// limpiar después: acá se le deja acceso también al dueño.)
    #[test]
    fn los_permisos_de_una_carpeta_se_pueden_cerrar() {
        let dir = std::env::temp_dir().join(format!("debloater-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("antes.txt"), b"x").unwrap();
        set_dacl(&dir, "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)").unwrap();
        std::fs::write(dir.join("despues.txt"), b"x").unwrap();
        assert_eq!(std::fs::read(dir.join("antes.txt")).unwrap(), b"x");
        // Un texto de permisos que no es válido se rechaza sin tocar nada.
        assert!(set_dacl(&dir, "esto no es SDDL").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn carpeta_de_datos_del_usuario() {
        let dir = user_data_dir().expect("Windows siempre tiene %APPDATA%");
        assert!(dir.is_absolute());
        assert_eq!(dir.file_name().unwrap(), APP_ID);
    }
}
