//! Exportar la configuración a un archivo e importarla en otra PC.

use crate::ui::{self, tr};
use crate::{Ctx, bloat, tweaks};
use prune_engine::engine::{self, Desired};
use prune_engine::{appx, profile};
use std::path::Path;

pub fn export(ctx: &Ctx, file: &Path) -> Result<(), String> {
    if file.exists() {
        // Con --json no hay a quién preguntarle si lo pisa.
        ctx.need_yes_in_json()?;
        let question = tf!("{file} ya existe. ¿Reemplazarlo?", file = file.display());
        if !ui::confirm(&question, ctx.yes)? {
            anstream::println!("{}", tr("No se escribió nada."));
            return Ok(());
        }
    }
    if !ctx.json {
        // Armar el perfil consulta winget y tarda unos segundos.
        anstream::eprintln!("{}", ui::dim(tr("Revisando ajustes, apps y programas…")));
    }
    profile::export_to(&ctx.engine, file)?;
    if ctx.json {
        return ui::json(&serde_json::json!({ "file": file.display().to_string() }));
    }
    anstream::println!("{} {}", ui::green("✓"), file.display());
    Ok(())
}

pub fn import(ctx: &Ctx, file: &Path, dry_run: bool, with_bloat: bool) -> Result<(), String> {
    let preview = profile::import_from(file)?;
    if ctx.json && dry_run {
        return ui::json(&preview);
    }
    let pending: Vec<&profile::ImportTweak> = preview.tweaks.iter().filter(|t| !t.current).collect();
    if !ctx.json {
        let already = preview.tweaks.len() - pending.len();
        anstream::println!("{}: {}", ui::bold(&preview.file), tf!("{n} ajustes para aplicar, {m} que ya están así", n = pending.len(), m = already));
        if preview.skipped > 0 {
            anstream::println!(
                "{} {} {}",
                ui::yellow("●"),
                preview.skipped,
                tr("entradas del archivo no existen en esta versión o no sirven en esta PC: se ignoran.")
            );
        }
        anstream::println!();
    }

    let views = engine::list(ctx.info.build);
    let desired: Vec<Desired> = pending.iter().map(|t| Desired { id: t.id.clone(), applied: t.applied, choice: t.choice.clone() }).collect();
    if !desired.is_empty() {
        tweaks::run_plan(ctx, &views, desired, dry_run)?;
    }

    if !preview.bloat.is_empty() {
        if with_bloat && !dry_run {
            let installed = appx::list()?;
            let picked: Vec<&appx::BloatView> = installed.iter().filter(|a| preview.bloat.iter().any(|b| b.id == a.id)).collect();
            if !ctx.json {
                anstream::println!("\n{}", ui::bold(tr("Apps de Windows a quitar")));
            }
            bloat::remove_apps(ctx, &picked, false)?;
        } else if !ctx.json {
            anstream::println!("\n{}", ui::bold(tr("Apps de Windows que el archivo quita")));
            for b in &preview.bloat {
                anstream::println!("  {}  {}", ui::cyan(&b.id), b.title);
            }
            if !with_bloat {
                anstream::println!("{}", ui::dim(tr("No se tocan. Para quitarlas también, agregá --bloat.")));
            }
        }
    }

    let missing: Vec<&profile::ImportApp> = preview.apps.iter().filter(|a| !a.installed).collect();
    if !missing.is_empty() && !ctx.json {
        anstream::println!("\n{}", ui::bold(tr("Programas del archivo que esta PC no tiene")));
        for a in &missing {
            anstream::println!("  {}  {}", ui::cyan(&a.id), a.title);
        }
        anstream::println!(
            "{}",
            ui::dim(tr("La terminal todavía no instala programas: usá la app de escritorio o `winget install <id>`."))
        );
    }
    Ok(())
}
