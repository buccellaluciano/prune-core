//! Limpieza de archivos descartables: temporales y cachés que Windows y las apps vuelven a
//! generar. Lo de las últimas 24 horas y lo que está en uso no se toca (eso lo decide el motor).

use crate::Ctx;
use crate::ui::{self, tr};
use clap::Subcommand;
use prune_engine::cleanup::{self, CleanView, Warn};

#[derive(Subcommand)]
pub enum CleanupCmd {
    /// Calcula cuánto espacio se puede liberar en cada categoría
    Scan,
    /// Borra las categorías elegidas (no se puede deshacer)
    Run {
        /// Categorías según `prune cleanup scan`
        #[arg(value_name = "CATEGORÍA", required_unless_present = "all")]
        categorias: Vec<String>,
        /// Todas las que no tienen advertencia
        #[arg(long, conflicts_with = "categorias")]
        all: bool,
    },
}

fn title(id: &str) -> String {
    match id {
        "temp_user" => tr("Temporales del usuario"),
        "temp_system" => tr("Temporales de Windows"),
        "crash" => tr("Volcados e informes de errores"),
        "shaders" => tr("Caché de sombreadores"),
        "thumbnails" => tr("Caché de miniaturas"),
        "update_cache" => tr("Descargas de Windows Update"),
        "spotify" => tr("Caché de Spotify"),
        "prefetch" => "Prefetch",
        "recycle_bin" => tr("Papelera de reciclaje"),
        other => return other.to_string(),
    }
    .to_string()
}

fn warn_text(w: Warn) -> &'static str {
    match w {
        Warn::SlowerStart => tr("Windows lo vuelve a armar, pero los próximos arranques y aperturas de apps son más lentos. No suele convenir."),
        Warn::Permanent => tr("Vacía la papelera: no se puede recuperar."),
        Warn::CloseApp => tr("Cerrá Spotify antes. Las canciones descargadas sin conexión se vuelven a bajar."),
    }
}

/// Por qué no se puede limpiar una categoría en esta terminal (None si se puede).
fn blocked(ctx: &Ctx, v: &CleanView) -> Option<&'static str> {
    if v.admin && !ctx.info.elevated {
        Some(tr("pide administrador"))
    } else if !v.available {
        Some(tr("no disponible"))
    } else {
        None
    }
}

pub fn run(ctx: &Ctx, cmd: Option<CleanupCmd>) -> Result<(), String> {
    let foreign = ctx.info.foreign_user.is_some();
    if !ctx.json {
        anstream::eprintln!("{}", ui::dim(tr("Calculando el espacio…")));
    }
    let views = cleanup::scan(ctx.info.elevated, foreign);
    let (ids, all) = match cmd.unwrap_or(CleanupCmd::Scan) {
        CleanupCmd::Scan => return scan(ctx, &views),
        CleanupCmd::Run { categorias, all } => (categorias, all),
    };

    let mut picked: Vec<&CleanView> = Vec::new();
    if all {
        // Lo que tiene advertencia hay que pedirlo por nombre.
        picked.extend(views.iter().filter(|v| v.warn.is_none() && blocked(ctx, v).is_none() && v.bytes > 0));
    } else {
        for id in &ids {
            let v = views
                .iter()
                .find(|v| v.id.eq_ignore_ascii_case(id))
                .ok_or_else(|| tf!("No existe la categoría {id}. Mirá `prune cleanup scan`.", id = id))?;
            if let Some(why) = blocked(ctx, v) {
                return Err(format!("{}: {why}", title(&v.id)));
            }
            if !picked.iter().any(|p| p.id == v.id) {
                picked.push(v);
            }
        }
    }
    let total: u64 = picked.iter().map(|v| v.bytes).sum();
    if picked.is_empty() || total == 0 {
        anstream::println!("{}", tr("No hay nada para borrar."));
        return Ok(());
    }
    ctx.need_yes_in_json()?;
    if !ctx.json {
        for v in &picked {
            anstream::println!("  {}  {}", ui::pad(&title(&v.id), 34), ui::size(v.bytes));
            if let Some(w) = v.warn {
                anstream::println!("    {} {}", ui::yellow("●"), warn_text(w));
            }
        }
    }
    if !ui::confirm(&tf!("¿Borrar {size}? No se puede deshacer.", size = ui::size(total)), ctx.yes)? {
        anstream::println!("{}", tr("No se borró nada."));
        return Ok(());
    }
    let ids: Vec<String> = picked.iter().map(|v| v.id.clone()).collect();
    let results = cleanup::run(&ids, ctx.info.elevated, foreign);
    if ctx.json {
        return ui::json(&results);
    }
    let freed: u64 = results.iter().map(|r| r.freed).sum();
    let skipped: u64 = results.iter().map(|r| r.skipped).sum();
    for r in results.iter().filter(|r| r.error.is_some()) {
        anstream::println!("{} {}: {}", ui::red("✗"), title(&r.id), r.error.as_deref().unwrap_or_default());
    }
    anstream::println!("{} {}", ui::green("✓"), tf!("Se liberaron {size}.", size = ui::size(freed)));
    if skipped > 0 {
        anstream::println!("{}", ui::dim(&tf!("Archivos en uso que quedaron sin borrar: {n}", n = skipped)));
    }
    if results.iter().any(|r| r.error.is_some()) {
        return Err(tr("Algunas categorías no se pudieron limpiar.").to_string());
    }
    Ok(())
}

fn scan(ctx: &Ctx, views: &[CleanView]) -> Result<(), String> {
    if ctx.json {
        return ui::json(&views);
    }
    let width = views.iter().map(|v| v.id.len()).max().unwrap_or(0);
    for v in views {
        let size = match blocked(ctx, v) {
            Some(why) => ui::dim(&ui::pad(why, 18)),
            None if v.bytes == 0 => ui::dim(&ui::pad(tr("vacío"), 18)),
            None => ui::pad(&ui::size(v.bytes), 18),
        };
        let warn = if v.warn.is_some() { ui::yellow(" !") } else { String::new() };
        anstream::println!("  {}  {size} {}{warn}", ui::cyan(&ui::pad(&v.id, width)), title(&v.id));
    }
    let total: u64 = views.iter().filter(|v| v.warn.is_none() && blocked(ctx, v).is_none()).map(|v| v.bytes).sum();
    anstream::println!("\n{}", tf!("Se pueden liberar {size} sin tocar lo que tiene advertencia (!).", size = ui::size(total)));
    for v in views.iter().filter(|v| v.warn.is_some()) {
        if let Some(w) = v.warn {
            anstream::println!("  {} {}: {}", ui::yellow("!"), title(&v.id), ui::dim(warn_text(w)));
        }
    }
    anstream::println!("{} {}", ui::dim(tr("Para limpiar:")), ui::cyan("prune cleanup run --all"));
    Ok(())
}
