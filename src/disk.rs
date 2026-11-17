//! Disco: qué ocupa lugar. Solo lectura.

use crate::Ctx;
use crate::ui::{self, tr};
use clap::Subcommand;
use prune_engine::disk::{self, DirView, Summary};
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::time::Duration;

#[derive(Subcommand)]
pub enum DiskCmd {
    /// Unidades con su espacio libre
    Drives,
    /// Recorre una unidad y muestra qué carpetas ocupan más
    Scan {
        /// Letra de la unidad (por defecto, C)
        #[arg(value_name = "UNIDAD", default_value = "C")]
        unidad: String,
        /// Cuántas carpetas mostrar
        #[arg(short = 'n', long, default_value_t = 15, value_name = "N")]
        limit: usize,
    },
}

pub fn run(ctx: &Ctx, cmd: Option<DiskCmd>) -> Result<(), String> {
    match cmd.unwrap_or(DiskCmd::Drives) {
        DiskCmd::Drives => drives(ctx),
        DiskCmd::Scan { unidad, limit } => scan(ctx, &unidad, limit),
    }
}

/// Barra de `width` caracteres llena en proporción a `part` de `total`.
fn bar(part: u64, total: u64, width: usize) -> String {
    let filled = if total == 0 { 0 } else { ((part as f64 / total as f64) * width as f64).round() as usize }.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn drives(ctx: &Ctx) -> Result<(), String> {
    let list = disk::drives();
    if ctx.json {
        return ui::json(&list);
    }
    for d in &list {
        let used = d.total.saturating_sub(d.free);
        let pct = if d.total == 0 { 0 } else { used * 100 / d.total };
        let gauge = bar(used, d.total, 20);
        let gauge = if pct >= 90 { ui::red(&gauge) } else if pct >= 75 { ui::yellow(&gauge) } else { ui::green(&gauge) };
        let label = if d.label.is_empty() { String::new() } else { format!("  {}", d.label) };
        anstream::println!("  {}  {gauge}  {}{}", ui::bold(&format!("{}:", d.letter)), tf!("{free} libres de {total}", free = ui::size(d.free), total = ui::size(d.total)), ui::dim(&label));
    }
    anstream::println!("\n{} {}", ui::dim(tr("Para ver qué ocupa lugar:")), ui::cyan("prune disk scan C"));
    Ok(())
}

#[derive(Serialize)]
struct Scan {
    summary: Summary,
    root: DirView,
}

fn scan(ctx: &Ctx, letter: &str, limit: usize) -> Result<(), String> {
    disk::start(letter)?;
    let live = !ctx.json && std::io::stderr().is_terminal();
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let s = disk::status();
        if live {
            let text = tf!("{files} archivos, {size}", files = s.files, size = ui::size(s.bytes));
            anstream::eprint!("\r{}", ui::dim(&ui::pad(&text, 50)));
            let _ = std::io::stderr().flush();
        }
        if let Some(e) = s.error {
            return Err(e);
        }
        if s.done || !s.running {
            break;
        }
    }
    if live {
        anstream::eprint!("\r{}\r", " ".repeat(52));
    }
    let summary = disk::summary()?;
    let root = disk::dir(0)?;
    if ctx.json {
        return ui::json(&Scan { summary, root });
    }

    anstream::println!("{}  {}", ui::bold(&summary.root), tf!("{size} en {files} archivos", size = ui::size(summary.size), files = summary.files));
    if summary.denied_dirs > 0 {
        anstream::println!("{}", ui::dim(&tf!("Carpetas que no se pudieron leer: {n} (con una terminal de administrador se ven más).", n = summary.denied_dirs)));
    }
    anstream::println!("\n{}", ui::bold(tr("Carpetas de la raíz")));
    for e in root.children.iter().take(limit) {
        anstream::println!("  {}  {}  {}", ui::pad(&ui::size(e.size), 9), ui::cyan(&bar(e.size, summary.size, 16)), e.name);
    }
    if root.own > 0 {
        anstream::println!("  {}  {}  {}", ui::pad(&ui::size(root.own), 9), ui::dim(&bar(root.own, summary.size, 16)), ui::dim(tr("(archivos sueltos)")));
    }
    if !summary.hotspots.is_empty() {
        anstream::println!("\n{}", ui::bold(tr("Carpetas que más ocupan")));
        for h in summary.hotspots.iter().take(limit) {
            anstream::println!("  {}  {}", ui::pad(&ui::size(h.size), 9), h.path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barras_proporcionales() {
        assert_eq!(bar(0, 100, 4), "░░░░");
        assert_eq!(bar(50, 100, 4), "██░░");
        assert_eq!(bar(100, 100, 4), "████");
        // Sin total, o con más que el total, no se rompe.
        assert_eq!(bar(5, 0, 4), "░░░░");
        assert_eq!(bar(500, 100, 4), "████");
    }
}
