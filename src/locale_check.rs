//! Comprueba que cada idioma de la CLI tenga todos los textos: los del código (`tr("...")` y
//! `tf!("...")`) y los de la ayuda (que arma clap con los comentarios de cada comando).
//!
//! Cuando agregues o cambies un texto, este test dice qué falta en cada `locales/<idioma>.json`.
//! Para ver la lista completa de textos en español:
//!   $env:PRUNE_KEYS_OUT = "claves.json"; cargo test -p prune-core volcar -- --ignored

use crate::ui;
use clap::CommandFactory;
use prune_engine::i18n::LANGS;
use std::collections::BTreeSet;

/// Todo el código de la CLI menos este archivo (que nombra `tr(` como texto, no como llamada).
const CODE: &[&str] = &[
    include_str!("apps.rs"),
    include_str!("bloat.rs"),
    include_str!("cleanup.rs"),
    include_str!("disk.rs"),
    include_str!("dns.rs"),
    include_str!("envpath.rs"),
    include_str!("journal.rs"),
    include_str!("main.rs"),
    include_str!("profile.rs"),
    include_str!("system_cmds.rs"),
    include_str!("tui.rs"),
    include_str!("tui_sections.rs"),
    include_str!("tweaks.rs"),
    include_str!("ui.rs"),
];

/// Los textos escritos directamente después de `marker` (`tr(` o `tf!(`), sin los comentarios.
fn literals_after(src: &str, marker: &str, out: &mut BTreeSet<String>) {
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let bytes = code.as_bytes();
    let mut from = 0;
    while let Some(i) = code[from..].find(marker) {
        let start = from + i;
        from = start + marker.len();
        // Que no sea el final de otro nombre (`attr(`, `str(`).
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        let Some(body) = code[from..].trim_start().strip_prefix('"') else { continue };
        let mut text = String::new();
        let mut chars = body.chars();
        loop {
            match chars.next() {
                Some('"') => {
                    out.insert(text);
                    break;
                }
                Some('\\') => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some(c) => text.push(c),
                    None => break,
                },
                Some(c) => text.push(c),
                None => break,
            }
        }
    }
}

/// Los textos de la ayuda: descripción de cada comando y de cada opción, y los nombres de
/// los valores escritos a mano (los que van en mayúsculas).
fn help_texts(cmd: &clap::Command, out: &mut BTreeSet<String>) {
    for text in [cmd.get_about(), cmd.get_after_help()].into_iter().flatten() {
        out.insert(text.to_string());
    }
    for arg in cmd.get_arguments() {
        if let Some(help) = arg.get_help() {
            out.insert(help.to_string());
        }
        // Solo de lo que recibe un valor: en un interruptor (--yes) el nombre nunca se ve.
        if arg.get_action().takes_values() {
            for name in arg.get_value_names().into_iter().flatten() {
                if !name.chars().any(char::is_lowercase) {
                    out.insert(name.to_string());
                }
            }
        }
    }
    for sub in cmd.get_subcommands() {
        help_texts(sub, out);
    }
}

fn all_texts() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for src in CODE {
        literals_after(src, "tr(", &mut out);
        literals_after(src, "tf!(", &mut out);
    }
    help_texts(&crate::Cli::command(), &mut out);
    out
}

/// Los `{valores}` de un texto.
fn placeholders(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}') {
            let name = &rest[..close];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Cuántos espacios tiene un texto al principio y al final.
fn edges(text: &str) -> (usize, usize) {
    (text.len() - text.trim_start().len(), text.len() - text.trim_end().len())
}

#[test]
fn el_lector_de_textos_encuentra_las_llamadas() {
    let mut out = BTreeSet::new();
    let src = "let a = tr(\"uno\");\n// tr(\"comentario\")\nlet b = ui::tr(\n    \"dos \\\"con comillas\\\"\",\n);\nlet c = attr(\"no\");\nlet d = tr(variable);";
    literals_after(src, "tr(", &mut out);
    assert_eq!(out.into_iter().collect::<Vec<_>>(), ["dos \"con comillas\"", "uno"]);
    assert_eq!(placeholders("Quedan {n} de {total}. {} {no vale}").into_iter().collect::<Vec<_>>(), ["n", "total"]);
}

#[test]
fn cada_idioma_tiene_todos_los_textos() {
    let texts = all_texts();
    assert!(texts.len() > 200, "se encontraron muy pocos textos ({}): ¿cambió cómo se leen?", texts.len());
    let mut problems = Vec::new();
    for (lang, table) in LANGS.iter().zip(ui::tables()).skip(1) {
        for es in &texts {
            match table.get(es) {
                None => problems.push(format!("{lang}: falta {es:?}")),
                Some(t) if t.trim().is_empty() => problems.push(format!("{lang}: está vacío {es:?}")),
                Some(t) if placeholders(t) != placeholders(es) => problems.push(format!("{lang}: no tiene los mismos {{valores}} {es:?} -> {t:?}")),
                // Los espacios de los bordes separan una palabra de la siguiente en pantalla.
                Some(t) if edges(t) != edges(es) => problems.push(format!("{lang}: no tiene los mismos espacios al principio y al final {es:?} -> {t:?}")),
                Some(_) => {}
            }
        }
        // Lo que ya no se usa se saca, así los archivos no juntan textos viejos.
        for es in table.keys().filter(|k| !texts.contains(*k)) {
            problems.push(format!("{lang}: sobra {es:?}"));
        }
    }
    let shown: Vec<&String> = problems.iter().take(60).collect();
    assert!(problems.is_empty(), "{} problemas en locales/ (se muestran {}):\n{}", problems.len(), shown.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// Escribe en `PRUNE_KEYS_OUT` todos los textos en español, como lista JSON.
#[test]
#[ignore = "herramienta: vuelca la lista de textos"]
fn volcar_textos() {
    let path = std::env::var("PRUNE_KEYS_OUT").expect("definí PRUNE_KEYS_OUT con el archivo de salida");
    let texts: Vec<String> = all_texts().into_iter().collect();
    std::fs::write(&path, serde_json::to_string_pretty(&texts).unwrap()).unwrap();
    println!("{} textos en {path}", texts.len());
}
