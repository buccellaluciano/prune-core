//! Programas: instalar, actualizar y desinstalar con winget lo que está en el catálogo.
//!
//! Igual que en la app de escritorio, solo se usan ids del catálogo (nunca un id escrito a
//! mano) y antes de instalar se puede ver de dónde baja el instalador y su SHA-256.

use crate::Ctx;
use crate::ui::{self, tr};
use clap::Subcommand;
use prune_engine::winget::{self, Action, AppStatus, AppsOverview, Phase};
use prune_engine::{i18n, vt};
use std::io::{IsTerminal, Write};

#[derive(Subcommand)]
pub enum AppsCmd {
    /// Programas del catálogo: cuáles están instalados y cuáles tienen actualización
    List {
        /// Texto a buscar en el id, el nombre o la descripción
        buscar: Option<String>,
        /// Solo los instalados
        #[arg(short, long)]
        installed: bool,
        /// Solo los que tienen actualización
        #[arg(short, long)]
        updates: bool,
    },
    /// Instala programas con winget
    Install {
        #[arg(required = true, value_name = "PROGRAMA")]
        programas: Vec<String>,
    },
    /// Actualiza programas instalados
    Upgrade {
        #[arg(value_name = "PROGRAMA", required_unless_present = "all")]
        programas: Vec<String>,
        /// Todos los del catálogo que tengan actualización
        #[arg(long, conflicts_with = "programas")]
        all: bool,
    },
    /// Desinstala programas
    Uninstall {
        #[arg(required = true, value_name = "PROGRAMA")]
        programas: Vec<String>,
    },
    /// De dónde baja el instalador y su SHA-256, para revisarlo antes de instalar
    Check {
        #[arg(value_name = "PROGRAMA")]
        programa: String,
    },
}

/// Estado de winget, o el motivo por el que no se puede usar.
fn overview(ctx: &Ctx) -> Result<AppsOverview, String> {
    if !ctx.json {
        anstream::eprintln!("{}", ui::dim(tr("Consultando winget…")));
    }
    let o = winget::status();
    if o.winget.is_none() {
        return Err(o.error.unwrap_or_else(|| tr("winget no está disponible en esta PC.").to_string()));
    }
    Ok(o)
}

/// Un programa por su id, o por una parte del id o del nombre si no hay dudas.
fn resolve<'a>(apps: &'a [AppStatus], query: &str) -> Result<&'a AppStatus, String> {
    if let Some(a) = apps.iter().find(|a| a.id.eq_ignore_ascii_case(query) || a.title.eq_ignore_ascii_case(query)) {
        return Ok(a);
    }
    let q = ui::fold(query);
    let hits: Vec<&AppStatus> = apps.iter().filter(|a| ui::fold(&a.id).contains(&q) || ui::fold(&a.title).contains(&q)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(tf!("Ese programa no está en el catálogo: {query}. Mirá `prune apps list`.", query = query)),
        many => {
            let ids: Vec<&str> = many.iter().take(8).map(|a| a.id.as_str()).collect();
            Err(tf!("`{query}` coincide con varios: {names}", query = query, names = ids.join(", ")))
        }
    }
}

fn phase_text(p: Phase) -> &'static str {
    match p {
        Phase::Download => tr("descargando"),
        Phase::Hash => tr("verificando"),
        Phase::Install => tr("instalando"),
        Phase::Uninstall => tr("desinstalando"),
        Phase::Done => tr("listo"),
    }
}

pub fn run(ctx: &Ctx, cmd: Option<AppsCmd>) -> Result<(), String> {
    match cmd.unwrap_or(AppsCmd::List { buscar: None, installed: false, updates: false }) {
        AppsCmd::List { buscar, installed, updates } => list(ctx, buscar.as_deref(), installed, updates),
        AppsCmd::Install { programas } => act(ctx, &programas, false, Action::Install),
        AppsCmd::Upgrade { programas, all } => act(ctx, &programas, all, Action::Upgrade),
        AppsCmd::Uninstall { programas } => act(ctx, &programas, false, Action::Uninstall),
        AppsCmd::Check { programa } => check(ctx, &programa),
    }
}

fn list(ctx: &Ctx, search: Option<&str>, installed: bool, updates: bool) -> Result<(), String> {
    let o = overview(ctx)?;
    let q = search.map(ui::fold);
    let shown: Vec<&AppStatus> = o
        .apps
        .iter()
        .filter(|a| !installed || a.installed.is_some())
        .filter(|a| !updates || a.available.is_some())
        .filter(|a| q.as_ref().is_none_or(|q| ui::fold(&a.id).contains(q) || ui::fold(&a.title).contains(q) || ui::fold(&a.description).contains(q)))
        .collect();
    if ctx.json {
        return ui::json(&shown);
    }
    if shown.is_empty() {
        anstream::println!("{}", tr("Ningún programa coincide."));
        return Ok(());
    }
    let width = shown.iter().map(|a| a.id.len()).max().unwrap_or(0);
    for g in &o.groups {
        let rows: Vec<&&AppStatus> = shown.iter().filter(|a| a.group == g.id).collect();
        if rows.is_empty() {
            continue;
        }
        anstream::println!("\n{}", ui::bold(&i18n::app_group(&g.id, &g.title)));
        for a in rows {
            let (mark, version) = match (&a.installed, &a.available) {
                (Some(v), Some(new)) => (ui::yellow("↑"), ui::yellow(&format!("  {v} → {new}"))),
                (Some(v), None) => (ui::green("●"), ui::dim(&format!("  {v}"))),
                (None, _) => (ui::dim("○"), String::new()),
            };
            anstream::println!("  {mark} {}  {}{version}", ui::cyan(&ui::pad(&a.id, width)), a.title);
        }
    }
    anstream::println!("\n{} {} {}  {} {}  {} {}", ui::dim(tr("Referencias:")), ui::green("●"), ui::dim(tr("instalado")), ui::yellow("↑"), ui::dim(tr("hay actualización")), ui::dim("○"), ui::dim(tr("sin instalar")));
    Ok(())
}

fn act(ctx: &Ctx, queries: &[String], all: bool, action: Action) -> Result<(), String> {
    let o = overview(ctx)?;
    let mut picked: Vec<&AppStatus> = Vec::new();
    if all {
        picked.extend(o.apps.iter().filter(|a| a.installed.is_some() && a.available.is_some()));
    }
    for q in queries {
        let a = resolve(&o.apps, q)?;
        // Lo que no aplica se dice antes de empezar, no a mitad de camino.
        match action {
            Action::Install if a.installed.is_some() => return Err(tf!("{name} ya está instalado. Para actualizarlo: `prune apps upgrade`.", name = a.title)),
            Action::Upgrade | Action::Uninstall if a.installed.is_none() => return Err(tf!("{name} no está instalado.", name = a.title)),
            Action::Upgrade if a.available.is_none() => return Err(tf!("{name} ya está en la última versión.", name = a.title)),
            _ => {}
        }
        if !picked.iter().any(|p| p.id == a.id) {
            picked.push(a);
        }
    }
    if picked.is_empty() {
        anstream::println!("{}", tr("No hay nada para actualizar."));
        return Ok(());
    }
    ctx.need_yes_in_json()?;
    if !ctx.json {
        for a in &picked {
            anstream::println!("  {}  {}", ui::cyan(&a.id), a.title);
        }
    }
    let question = match action {
        Action::Install => tf!("¿Instalar {n} programas con winget?", n = picked.len()),
        Action::Upgrade => tf!("¿Actualizar {n} programas con winget?", n = picked.len()),
        Action::Uninstall => tf!("¿Desinstalar {n} programas?", n = picked.len()),
    };
    if !ui::confirm(&question, ctx.yes)? {
        anstream::println!("{}", tr("No se cambió nada."));
        return Ok(());
    }

    let live = !ctx.json && std::io::stderr().is_terminal();
    let mut results = Vec::new();
    for a in &picked {
        let mut last = String::new();
        let r = winget::act(&a.id, action, |p| {
            // Un solo renglón que se va pisando: fase y porcentaje.
            let Some(phase) = p.phase else { return };
            let text = match p.fraction {
                Some(f) => format!("{} {:.0}%", phase_text(phase), f * 100.0),
                None => phase_text(phase).to_string(),
            };
            if live && text != last {
                anstream::eprint!("\r  {}  {}", a.title, ui::dim(&ui::pad(&text, 24)));
                let _ = std::io::stderr().flush();
                last = text;
            }
        });
        if live {
            anstream::eprint!("\r{}\r", " ".repeat(ui::width(&a.title) + 30));
        }
        if !ctx.json {
            match (&r.ok, &r.message) {
                (true, _) => anstream::println!("{} {}", ui::green("✓"), a.title),
                (false, m) => anstream::println!("{} {}: {}", ui::red("✗"), a.title, m.as_deref().unwrap_or_default()),
            }
        }
        results.push(r);
    }
    if ctx.json {
        ui::json(&results)?;
    }
    let failed = results.iter().filter(|r| !r.ok).count();
    if failed > 0 {
        return Err(tf!("Fallaron {n} programas.", n = failed));
    }
    Ok(())
}

fn check(ctx: &Ctx, query: &str) -> Result<(), String> {
    let o = overview(ctx)?;
    let a = resolve(&o.apps, query)?;
    let info = winget::installer_info(&a.id)?;
    if ctx.json {
        return ui::json(&info);
    }
    anstream::println!("{}  {}", ui::bold(&a.title), ui::cyan(&a.id));
    if let Some(v) = &info.version {
        anstream::println!("{}  {v}", ui::pad(tr("Versión:"), 12));
    }
    if let Some(u) = &info.url {
        anstream::println!("{}  {u}", ui::pad(tr("Instalador:"), 12));
    }
    if let Some(h) = &info.sha256 {
        anstream::println!("{}  {h}", ui::pad("SHA-256:", 12));
        if let Ok(url) = vt::report_url(h) {
            anstream::println!("{}  {url}", ui::pad("VirusTotal:", 12));
        }
    }
    anstream::println!("{}", ui::dim(tr("winget comprueba ese SHA-256 al instalar: si el archivo no coincide, no lo instala.")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, title: &str) -> AppStatus {
        AppStatus { id: id.into(), title: title.into(), group: "g".into(), description: String::new(), installed: None, available: None }
    }

    #[test]
    fn busca_por_id_o_por_nombre() {
        let apps = vec![app("Mozilla.Firefox", "Firefox"), app("Mozilla.Thunderbird", "Thunderbird"), app("7zip.7zip", "7-Zip")];
        assert_eq!(resolve(&apps, "mozilla.firefox").unwrap().id, "Mozilla.Firefox");
        assert_eq!(resolve(&apps, "firefox").unwrap().id, "Mozilla.Firefox");
        assert_eq!(resolve(&apps, "7-zip").unwrap().id, "7zip.7zip");
        assert_eq!(resolve(&apps, "thunder").unwrap().id, "Mozilla.Thunderbird");
        assert!(resolve(&apps, "mozilla").unwrap_err().contains("Mozilla.Thunderbird"));
        assert!(resolve(&apps, "chrome").is_err());
    }
}
