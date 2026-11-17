//! Salida en la terminal: colores, columnas, confirmaciones y los textos propios de la CLI.
//!
//! Idiomas: el español es la fuente y va escrito en el código; `tr` busca ese mismo texto en
//! `locales/<idioma>.json`. Si un idioma no lo tiene, sale en inglés, y si tampoco,
//! en español. Los tests comprueban que cada idioma tenga todos los textos del código y de la
//! ayuda, con los mismos `{valores}`. El catálogo y los mensajes del motor los traduce
//! `prune_engine::i18n`.

use anstyle::{AnsiColor, Style};
use prune_engine::i18n::LANGS;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Las traducciones, en el orden de `LANGS` (el español no tiene archivo: es la clave).
const SOURCES: [&str; 9] = [
    "{}",
    include_str!("../locales/en.json"),
    include_str!("../locales/pt-BR.json"),
    include_str!("../locales/fr.json"),
    include_str!("../locales/de.json"),
    include_str!("../locales/it.json"),
    include_str!("../locales/zh-CN.json"),
    include_str!("../locales/ja.json"),
    include_str!("../locales/ru.json"),
];
const EN: usize = 1;

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn tables() -> &'static [HashMap<String, String>] {
    static T: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
    T.get_or_init(|| {
        SOURCES.iter().zip(LANGS).map(|(s, l)| serde_json::from_str(s).unwrap_or_else(|e| panic!("locales/{l}.json inválido: {e}"))).collect()
    })
}

/// Idioma de los textos de la CLI (uno de `LANGS`; cualquier otro deja el que estaba).
pub fn set_lang(lang: &str) {
    if let Some(i) = LANGS.iter().position(|l| *l == lang) {
        CURRENT.store(i, Ordering::Relaxed);
    }
}

fn lookup(es: &str) -> Option<&'static str> {
    let t = tables();
    let cur = CURRENT.load(Ordering::Relaxed);
    t[cur].get(es).or_else(|| if cur == 0 { None } else { t[EN].get(es) }).map(String::as_str)
}

/// Texto de la CLI en el idioma elegido. `es` es el texto en español, tal cual.
pub fn tr(es: &'static str) -> &'static str {
    lookup(es).unwrap_or(es)
}

/// Como `tr`, para textos que no están escritos en el código (la ayuda de clap).
pub fn tr_dyn(es: &str) -> String {
    lookup(es).unwrap_or(es).to_string()
}

/// Texto con valores: cada `{nombre}` se reemplaza por su valor. Se usa con `tf!`.
pub fn fmt(es: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut s = tr(es).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

fn paint(style: Style, s: &str) -> String {
    format!("{style}{s}{style:#}")
}

pub fn green(s: &str) -> String {
    paint(AnsiColor::Green.on_default(), s)
}

pub fn yellow(s: &str) -> String {
    paint(AnsiColor::Yellow.on_default(), s)
}

pub fn red(s: &str) -> String {
    paint(AnsiColor::Red.on_default(), s)
}

pub fn cyan(s: &str) -> String {
    paint(AnsiColor::Cyan.on_default(), s)
}

pub fn dim(s: &str) -> String {
    paint(Style::new().dimmed(), s)
}

pub fn bold(s: &str) -> String {
    paint(Style::new().bold(), s)
}

/// Cuántas columnas ocupa un texto en la terminal: los acentos cuentan uno y los caracteres
/// anchos (chino, japonés), dos.
pub fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Para comparar al buscar: en minúsculas y sin acentos, así "telemetria" encuentra
/// "Telemetría" y "telemetrie" encuentra "Télémétrie".
pub fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Rellena con espacios hasta `width` columnas (ver `width`).
pub fn pad(s: &str, width: usize) -> String {
    let len = self::width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(len)))
}

/// Hace cuánto pasó algo (`at` en segundos desde 1970).
pub fn ago(at: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let secs = now.saturating_sub(at);
    match secs {
        0..60 => tr("recién").to_string(),
        60..3600 => tf!("hace {n} min", n = secs / 60),
        3600..86_400 => tf!("hace {n} h", n = secs / 3600),
        _ => tf!("hace {n} d", n = secs / 86_400),
    }
}

/// Tamaño legible: 1,5 GB, 320 MB, 12 KB.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

pub fn json<T: Serialize>(value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    anstream::println!("{text}");
    Ok(())
}

/// Lo que cuenta como "sí": en español e inglés siempre, y además en el idioma elegido.
pub fn is_yes(answer: &str) -> bool {
    let a = answer.trim().to_lowercase();
    if a.is_empty() {
        return false;
    }
    let yes = tr("sí").to_lowercase();
    matches!(a.as_str(), "s" | "si" | "sí" | "y" | "yes") || a == yes || yes.chars().next().is_some_and(|c| a == c.to_string())
}

/// Pregunta de sí o no. Con `yes` (`--yes`) no pregunta. Si nadie puede contestar (la entrada
/// no es una terminal), no se asume nada: es un error.
pub fn confirm(question: &str, yes: bool) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(tr("Hace falta confirmar y no hay una terminal para preguntar: agregá --yes.").to_string());
    }
    anstream::eprint!("{question} {} ", dim(tr("[s/N]")));
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(is_yes(&line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rellena_por_columnas() {
        assert_eq!(pad("días", 6), "días  ");
        assert_eq!(pad("largo", 3), "largo");
        // Cada carácter japonés ocupa dos columnas.
        assert_eq!(width("はい"), 4);
        assert_eq!(pad("はい", 6), "はい  ");
    }

    #[test]
    fn buscar_no_distingue_acentos() {
        assert_eq!(fold("Telemetría"), "telemetria");
        assert_eq!(fold("Télémétrie"), "telemetrie");
        assert_eq!(fold("Búsqueda ÑANDÚ"), "busqueda nandu");
        assert_eq!(fold("遥测"), "遥测");
    }

    #[test]
    fn tamanos_legibles() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(320 * 1024 * 1024), "320 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
    }

    #[test]
    fn respuestas_que_cuentan_como_si() {
        for a in ["s", "S", "si", "sí", "y", "YES", " s\n"] {
            assert!(is_yes(a), "{a:?}");
        }
        for a in ["", "n", "no", "x", "\n"] {
            assert!(!is_yes(a), "{a:?}");
        }
    }
}
