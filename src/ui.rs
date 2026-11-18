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

// ───────────────────────── Colores y texto que no es nuestro ─────────────────────────
//
// Mucho de lo que se imprime viene del sistema: nombres de programas que arrancan con
// Windows, valores del registro, datos del proxy. Un programa sin permisos puede poner ahí
// secuencias de escape que, en la terminal de un administrador, mueven el cursor, borran
// renglones o reescriben el plan que está por confirmar.
//
// Por eso los colores no se escriben como secuencias de escape sino con marcas propias
// (caracteres de uso privado), y todo lo que se imprime pasa por `render` (lo hacen `say!`,
// `note!` y `ask!`): ahí se saca cualquier carácter de control que traiga el texto y recién
// después las marcas se cambian por los colores de verdad.

const GREEN: char = '\u{E000}';
const YELLOW: char = '\u{E001}';
const RED: char = '\u{E002}';
const CYAN: char = '\u{E003}';
const DIM: char = '\u{E004}';
const BOLD: char = '\u{E005}';
const RESET: char = '\u{E00F}';

fn style_of(mark: char) -> Option<Style> {
    Some(match mark {
        GREEN => AnsiColor::Green.on_default(),
        YELLOW => AnsiColor::Yellow.on_default(),
        RED => AnsiColor::Red.on_default(),
        CYAN => AnsiColor::Cyan.on_default(),
        DIM => Style::new().dimmed(),
        BOLD => Style::new().bold(),
        _ => return None,
    })
}

fn paint(mark: char, s: &str) -> String {
    format!("{mark}{s}{RESET}")
}

/// Caracteres que no se le pasan a la terminal: los de control (ESC incluido; el salto de
/// renglón es el único que queda) y los que invierten el sentido del texto.
fn unsafe_char(c: char) -> bool {
    c != '\n' && (c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
}

/// Deja un texto listo para la terminal: sin caracteres de control y con los colores puestos.
pub fn render(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut open: Option<Style> = None;
    let close = |out: &mut String, open: &mut Option<Style>| {
        if let Some(style) = open.take() {
            out.push_str(&format!("{style:#}"));
        }
    };
    for c in text.chars() {
        if c == RESET {
            close(&mut out, &mut open);
        } else if let Some(style) = style_of(c) {
            close(&mut out, &mut open);
            out.push_str(&format!("{style}"));
            open = Some(style);
        } else if !unsafe_char(c) {
            out.push(c);
        }
    }
    close(&mut out, &mut open);
    out
}

/// Para un dato que va en un solo renglón (un nombre, un valor): además de lo que saca
/// `render`, los saltos de renglón pasan a ser espacios. Si no, un nombre con un salto adentro
/// podría hacerse pasar por un renglón más de la salida.
pub fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\n' || unsafe_char(c) { ' ' } else { c }).collect()
}

/// Un nombre o id escrito por la persona no puede venir vacío: vacío coincide con todo, y con
/// un solo candidato se terminaría actuando sobre algo que nadie nombró (pasa con una variable
/// vacía en un script).
pub fn need_text(query: &str) -> Result<(), String> {
    if query.trim().is_empty() { Err(tr("Falta el nombre o el id.").to_string()) } else { Ok(()) }
}

pub fn green(s: &str) -> String {
    paint(GREEN, s)
}

pub fn yellow(s: &str) -> String {
    paint(YELLOW, s)
}

pub fn red(s: &str) -> String {
    paint(RED, s)
}

pub fn cyan(s: &str) -> String {
    paint(CYAN, s)
}

pub fn dim(s: &str) -> String {
    paint(DIM, s)
}

pub fn bold(s: &str) -> String {
    paint(BOLD, s)
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
    // serde ya escapa los caracteres de control dentro de cada texto: va tal cual.
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
    ask!("{question} {} ", dim(tr("[s/N]")));
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

    /// Lo que viene del sistema no puede mandarle órdenes a la terminal.
    #[test]
    fn las_secuencias_de_escape_ajenas_no_pasan() {
        // Borrar la pantalla, subir el cursor, cambiar el título, un retorno de carro.
        let evil = "Steam\u{1b}[2J\u{1b}[1A\u{1b}]0;hola\u{7}\rMicrosoft\u{9b}31m";
        let shown = render(&format!("{} {evil}", green("●")));
        assert!(!shown.contains(evil));
        // Lo único de control que queda es lo nuestro: el color del punto y su cierre.
        assert_eq!(shown.matches('\u{1b}').count(), 2, "{shown:?}");
        assert!(!shown.contains('\r') && !shown.contains('\u{7}') && !shown.contains('\u{9b}'));
        assert!(shown.contains("Steam[2J") && shown.contains("Microsoft"));
        // El texto que invierte el sentido de lectura tampoco.
        assert_eq!(render("a\u{202e}b"), "ab");
        // Los saltos de renglón nuestros sí.
        assert_eq!(render("uno\ndos"), "uno\ndos");
    }

    #[test]
    fn un_dato_de_un_renglon_no_puede_fingir_otro_renglon() {
        assert_eq!(clean("Steam\n● Microsoft Windows"), "Steam ● Microsoft Windows");
        assert_eq!(clean("a\tb\u{1b}[0m"), "a b [0m");
        assert_eq!(clean("Conexión de área local"), "Conexión de área local");
    }

    #[test]
    fn los_colores_propios_se_ponen_y_se_cierran() {
        let shown = render(&format!("{} y {}", bold("uno"), red("dos")));
        assert_eq!(shown.matches('\u{1b}').count(), 4, "{shown:?}");
        assert!(shown.ends_with("\u{1b}[0m"), "{shown:?}");
        // Sin colores no se agrega nada.
        assert_eq!(render("texto común"), "texto común");
    }

    #[test]
    fn un_nombre_vacio_no_es_un_nombre() {
        assert!(need_text("").is_err());
        assert!(need_text("   ").is_err());
        assert!(need_text("steam").is_ok());
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
