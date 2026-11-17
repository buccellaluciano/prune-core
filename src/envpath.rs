//! `prune path`: agrega o quita la carpeta de `prune.exe` del PATH de Windows.
//!
//! Es para quien usa P-Rune Core sin instalar (el zip): lo deja a mano desde cualquier carpeta
//! (`prune path add`) y lo saca sin dejar rastro (`prune path remove`). El instalador MSI no
//! lo necesita: del PATH se encarga Windows Installer (ver installer.wxs). Reglas:
//!
//! - Solo se agrega o se quita una entrada, la de esta carpeta. El resto del PATH queda
//!   exactamente como estaba, y se conserva el tipo del valor (`REG_EXPAND_SZ` o `REG_SZ`).
//! - Al PATH del equipo solo va una carpeta de Program Files (ahí escriben solo los
//!   administradores). Una carpeta donde escribe cualquiera, en el PATH de todos, dejaría
//!   que otro programa plante ahí un ejecutable.

use crate::PathCmd;
use crate::ui::{self, tr};
use prune_engine::catalog::Hive;
use prune_engine::registry::{self as reg, RawValue};
use prune_engine::{paths, system};
use serde::Serialize;

const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
/// Una variable de entorno no puede pasar de 32.767 caracteres.
const MAX_LEN: usize = 32_000;

/// Dónde vive un PATH en el registro.
#[derive(Clone, Copy)]
struct Scope {
    hive: Hive,
    key: &'static str,
}

const MACHINE: Scope = Scope { hive: Hive::HKLM, key: r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment" };
const USER: Scope = Scope { hive: Hive::HKCU, key: "Environment" };

// ───────────────────────── Texto del PATH ─────────────────────────

/// Para comparar entradas: sin espacios, comillas ni barra final, y sin importar mayúsculas.
fn norm(entry: &str) -> String {
    entry.trim().trim_matches('"').trim_end_matches(['\\', '/']).to_lowercase()
}

fn has(path: &str, dir: &str) -> bool {
    let dir = norm(dir);
    path.split(';').any(|e| norm(e) == dir)
}

/// El PATH con `dir` al final. None si ya estaba.
fn added(path: &str, dir: &str) -> Option<String> {
    if has(path, dir) {
        return None;
    }
    let base = path.trim_end_matches(';');
    Some(if base.is_empty() { dir.to_string() } else { format!("{base};{dir}") })
}

/// El PATH sin `dir`. None si no estaba.
fn removed(path: &str, dir: &str) -> Option<String> {
    if !has(path, dir) {
        return None;
    }
    let dir = norm(dir);
    Some(path.split(';').filter(|e| norm(e) != dir).collect::<Vec<_>>().join(";"))
}

/// Las entradas que no son `dir`, en orden. Antes de escribir se comprueba que sean las
/// mismas que había: si no, no se escribe nada.
fn others<'a>(path: &'a str, dir: &str) -> Vec<&'a str> {
    let dir = norm(dir);
    path.split(';').filter(|e| !e.trim().is_empty() && norm(e) != dir).collect()
}

// ───────────────────────── Registro ─────────────────────────

fn read(scope: Scope) -> Result<(String, u32), String> {
    match reg::read(scope.hive, scope.key, "Path").map_err(|e| e.to_string())? {
        // Sin PATH propio (pasa con el del usuario): se crea como lo crea Windows.
        None => Ok((String::new(), REG_EXPAND_SZ)),
        Some(v) if v.vtype == REG_SZ || v.vtype == REG_EXPAND_SZ => {
            let text = reg::as_text(&v).ok_or_else(|| tr("El PATH no se pudo leer como texto.").to_string())?;
            Ok((text, v.vtype))
        }
        Some(v) => Err(format!("{} ({})", tr("El PATH tiene un tipo inesperado en el registro: no se toca"), v.vtype)),
    }
}

/// Cambia el PATH de `scope` con `edit` (que devuelve None si no hay nada que cambiar).
/// Devuelve si escribió algo.
fn change(scope: Scope, dir: &str, edit: fn(&str, &str) -> Option<String>) -> Result<bool, String> {
    let (old, vtype) = read(scope)?;
    let Some(new) = edit(&old, dir) else { return Ok(false) };
    if others(&new, dir) != others(&old, dir) {
        return Err(tr("El PATH nuevo no conserva las demás entradas: no se escribió nada.").to_string());
    }
    if new.encode_utf16().count() > MAX_LEN {
        return Err(tr("El PATH quedaría demasiado largo: no se escribió nada.").to_string());
    }
    let bytes: Vec<u8> = new.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    reg::write_raw(scope.hive, scope.key, "Path", &RawValue { vtype, bytes }).map_err(|e| e.to_string())?;
    Ok(true)
}

fn contains(scope: Scope, dir: &str) -> bool {
    read(scope).is_ok_and(|(path, _)| has(&path, dir))
}

/// Avisa a los programas abiertos (el Explorador, sobre todo) que cambió el entorno, así
/// las terminales que se abran desde ahora ya ven el PATH nuevo.
fn broadcast() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
    let name: Vec<u16> = "Environment\0".encode_utf16().collect();
    unsafe {
        let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(name.as_ptr() as isize), SMTO_ABORTIFHUNG, 2000, None);
    }
}

// ───────────────────────── Comandos ─────────────────────────

/// La carpeta donde está este exe.
fn own_dir() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or_else(|| tr("No se pudo saber en qué carpeta está prune.").to_string())?;
    Ok(dir.to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

#[derive(Serialize)]
struct Status {
    dir: String,
    machine: bool,
    user: bool,
    /// Esta terminal ya lo tiene (las abiertas antes del cambio, no).
    session: bool,
    changed: bool,
}

fn status_of(dir: String, changed: bool) -> Status {
    let session = std::env::var("PATH").is_ok_and(|p| has(&p, &dir));
    Status { machine: contains(MACHINE, &dir), user: contains(USER, &dir), session, changed, dir }
}

fn report(json: bool, s: &Status, headline: String) -> Result<(), String> {
    if json {
        return ui::json(s);
    }
    let yes_no = |b: bool| if b { ui::green(tr("sí")) } else { ui::dim(tr("no")) };
    anstream::println!("{headline}");
    // Las etiquetas se alinean según el idioma, no con espacios escritos a mano.
    let labels = [tr("carpeta:"), tr("PATH del equipo:"), tr("PATH de tu usuario:")];
    let width = labels.iter().map(|l| ui::width(l)).max().unwrap_or(0);
    anstream::println!("  {}  {}", ui::dim(&ui::pad(labels[0], width)), s.dir);
    anstream::println!("  {}  {}", ui::dim(&ui::pad(labels[1], width)), yes_no(s.machine));
    anstream::println!("  {}  {}", ui::dim(&ui::pad(labels[2], width)), yes_no(s.user));
    if (s.machine || s.user) && !s.session {
        anstream::println!("{}", ui::dim(tr("Esta terminal se abrió antes del cambio: abrí una nueva para escribir `prune` a secas.")));
    }
    Ok(())
}

pub fn run(cmd: &PathCmd, json: bool) -> Result<(), String> {
    let dir = own_dir()?;
    let elevated = system::is_elevated();
    match cmd {
        PathCmd::Status => {
            let s = status_of(dir, false);
            let headline = if s.machine || s.user { tr("`prune` está en el PATH.") } else { tr("`prune` no está en el PATH.") };
            report(json, &s, headline.to_string())
        }
        PathCmd::Add => {
            // Al PATH de todo el equipo solo va una instalación en Program Files.
            let machine = elevated && paths::installed_data_dir().is_some();
            // Si ya está en alguno de los dos (el instalador lo pone en el del equipo), no
            // hace falta repetirlo en el otro.
            let already = contains(MACHINE, &dir) || contains(USER, &dir);
            let changed = !already && change(if machine { MACHINE } else { USER }, &dir, added)?;
            if changed {
                broadcast();
            }
            let headline = match (changed, machine) {
                (false, _) => tr("Ya estaba en el PATH: no se cambió nada."),
                (true, true) => tr("Agregado al PATH del equipo."),
                (true, false) => tr("Agregado al PATH de tu usuario."),
            };
            report(json, &status_of(dir, changed), format!("{} {headline}", ui::green("✓")))
        }
        PathCmd::Remove => {
            let mut changed = change(USER, &dir, removed)?;
            if elevated {
                changed |= change(MACHINE, &dir, removed)?;
            }
            if changed {
                broadcast();
            }
            let s = status_of(dir, changed);
            if s.machine {
                report(json, &s, String::new())?;
                return Err(tr("Sigue en el PATH del equipo: para quitarlo de ahí hace falta una terminal de administrador.").to_string());
            }
            let headline = if changed { tr("Quitado del PATH.") } else { tr("No estaba en el PATH: no se cambió nada.") };
            report(json, &s, format!("{} {headline}", ui::green("✓")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Program Files\P-Rune";

    #[test]
    fn agrega_al_final_y_una_sola_vez() {
        assert_eq!(added(r"C:\Windows;C:\Windows\System32", DIR).unwrap(), r"C:\Windows;C:\Windows\System32;C:\Program Files\P-Rune");
        assert_eq!(added("", DIR).unwrap(), DIR);
        // Un punto y coma colgando no deja una entrada vacía en el medio.
        assert_eq!(added(r"C:\Windows;", DIR).unwrap(), r"C:\Windows;C:\Program Files\P-Rune");
        // Ya está, aunque esté escrito distinto.
        for same in [r"C:\Windows;C:\Program Files\P-Rune", r"c:\program files\p-rune\;C:\Windows", r#"C:\Windows;"C:\Program Files\P-Rune""#, r"C:\Windows; C:\Program Files\P-Rune "] {
            assert!(added(same, DIR).is_none(), "{same}");
        }
        // Una carpeta que empieza igual no es la misma.
        assert!(added(r"C:\Program Files\P-Rune2", DIR).is_some());
    }

    #[test]
    fn quita_solo_lo_propio() {
        assert_eq!(removed(r"C:\Windows;C:\Program Files\P-Rune;%USERPROFILE%\bin", DIR).unwrap(), r"C:\Windows;%USERPROFILE%\bin");
        assert_eq!(removed(r"C:\Program Files\P-Rune\", DIR).unwrap(), "");
        // Si quedó repetida, se van todas.
        assert_eq!(removed(r"C:\Program Files\P-Rune;C:\Windows;c:\program files\p-rune", DIR).unwrap(), r"C:\Windows");
        assert!(removed(r"C:\Windows;C:\Program Files\P-Rune2", DIR).is_none());
    }

    #[test]
    fn el_resto_queda_tal_cual() {
        // Variables sin expandir, espacios y mayúsculas raras: no se tocan.
        let before = r"%SystemRoot%\system32;C:\Con Espacios\Bin;c:\MiXtO\";
        let after = added(before, DIR).unwrap();
        assert!(after.starts_with(before.trim_end_matches(';')));
        assert_eq!(removed(&after, DIR).unwrap(), before);
        assert_eq!(others(&after, DIR), others(before, DIR));
    }

    /// Ida y vuelta contra el registro de verdad, en la clave de prueba propia (no en el
    /// entorno): se conserva el tipo del valor y lo demás queda igual.
    #[test]
    fn ida_y_vuelta_en_el_registro() {
        let scope = Scope { hive: Hive::HKCU, key: r"Software\DebloaterTest\EnvPath" };
        let _ = reg::delete_tree(scope.hive, scope.key);

        // Sin valor: se crea como REG_EXPAND_SZ.
        assert!(change(scope, DIR, added).unwrap());
        assert_eq!(read(scope).unwrap(), (DIR.to_string(), REG_EXPAND_SZ));
        assert!(!change(scope, DIR, added).unwrap(), "agregar dos veces no cambia nada");
        assert!(change(scope, DIR, removed).unwrap());
        assert!(!change(scope, DIR, removed).unwrap(), "quitar dos veces no cambia nada");

        // Con un REG_SZ que ya tenía cosas: mismo tipo, mismas entradas.
        let before = r"%SystemRoot%\system32;C:\Otra Cosa";
        let bytes: Vec<u8> = before.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        reg::write_raw(scope.hive, scope.key, "Path", &RawValue { vtype: REG_SZ, bytes }).unwrap();
        assert!(change(scope, DIR, added).unwrap());
        assert!(contains(scope, DIR));
        assert_eq!(read(scope).unwrap(), (format!("{before};{DIR}"), REG_SZ));
        assert!(change(scope, DIR, removed).unwrap());
        assert_eq!(read(scope).unwrap(), (before.to_string(), REG_SZ));

        // Un tipo que no es texto no se toca.
        reg::write_raw(scope.hive, scope.key, "Path", &RawValue { vtype: 4, bytes: vec![1, 0, 0, 0] }).unwrap();
        assert!(change(scope, DIR, added).is_err());
        assert_eq!(reg::read(scope.hive, scope.key, "Path").unwrap().unwrap().bytes, vec![1, 0, 0, 0]);

        reg::delete_tree(scope.hive, scope.key).unwrap();
    }
}
