//! Apps preinstaladas: ver cuáles hay y quitarlas.

use crate::Ctx;
use crate::ui::{self, tr};
use prune_engine::appx::{self, BloatView, RemoveResult};
use prune_engine::catalog::Risk;

fn tier_title(t: Risk) -> &'static str {
    match t {
        Risk::Safe => tr("Seguras: nadie las extraña"),
        Risk::Moderate => tr("Moderadas: puede que alguien las use"),
        Risk::Advanced => tr("Avanzadas: quitarlas rompe funciones, leé la nota"),
    }
}

pub fn list(ctx: &Ctx) -> Result<(), String> {
    let apps = appx::list()?;
    if ctx.json {
        return ui::json(&apps);
    }
    if apps.is_empty() {
        say!("{} {}", ui::green("✓"), tr("No queda ninguna app del catálogo instalada."));
        return Ok(());
    }
    let width = apps.iter().map(|a| a.id.len()).max().unwrap_or(0);
    for tier in [Risk::Safe, Risk::Moderate, Risk::Advanced] {
        let rows: Vec<&BloatView> = apps.iter().filter(|a| a.tier == tier).collect();
        if rows.is_empty() {
            continue;
        }
        say!("\n{}", ui::bold(tier_title(tier)));
        for a in rows {
            let note = a.note.as_ref().map(|n| ui::dim(&format!("  {n}"))).unwrap_or_default();
            say!("  {}  {}{note}", ui::cyan(&ui::pad(&a.id, width)), a.title);
        }
    }
    say!("\n{} {}", ui::dim(tr("Para quitar:")), ui::cyan("prune bloat remove <app> [<app>…]"));
    Ok(())
}

/// Una app instalada por su id, o por una parte del id o del nombre si no hay dudas.
fn resolve<'a>(apps: &'a [BloatView], query: &str) -> Result<&'a BloatView, String> {
    ui::need_text(query)?;
    if let Some(a) = apps.iter().find(|a| a.id.eq_ignore_ascii_case(query)) {
        return Ok(a);
    }
    let q = ui::fold(query);
    let hits: Vec<&BloatView> = apps.iter().filter(|a| ui::fold(&a.id).contains(&q) || ui::fold(&a.title).contains(&q)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "{}: {query}. {} `prune bloat list`.",
            tr("Esa app no está instalada o no está en el catálogo"),
            tr("Mirá")
        )),
        many => {
            let ids: Vec<&str> = many.iter().take(8).map(|a| a.id.as_str()).collect();
            Err(format!("`{query}` {}: {}", tr("coincide con varias apps"), ids.join(", ")))
        }
    }
}

pub fn remove(ctx: &Ctx, queries: &[String], deprovision: bool) -> Result<(), String> {
    let apps = appx::list()?;
    let mut picked: Vec<&BloatView> = Vec::new();
    for q in queries {
        let a = resolve(&apps, q)?;
        if !picked.iter().any(|p| p.id == a.id) {
            picked.push(a);
        }
    }
    remove_apps(ctx, &picked, deprovision)
}

/// Confirma y quita. Lo usan `bloat remove` y `profile import --bloat`.
pub fn remove_apps(ctx: &Ctx, picked: &[&BloatView], deprovision: bool) -> Result<(), String> {
    let (results, failure) = remove_confirmed(ctx, picked, deprovision)?;
    if ctx.json {
        ui::json(&results)?;
    }
    failure.map_or(Ok(()), Err)
}

/// Lo mismo, sin imprimir el JSON: devuelve el resultado por app y, si alguna falló, el error
/// con el que tiene que terminar el comando.
pub(crate) fn remove_confirmed(ctx: &Ctx, picked: &[&BloatView], deprovision: bool) -> Result<(Vec<RemoveResult>, Option<String>), String> {
    if picked.is_empty() {
        return Ok((Vec::new(), None));
    }
    // Con otra cuenta de administrador se quitarían las apps de esa cuenta, no las de la persona.
    if let Some(user) = &ctx.info.foreign_user {
        return Err(t!("foreign_user_remove", user = user));
    }
    ctx.need_yes_in_json()?;
    if !ctx.json {
        for a in picked {
            let tier = match a.tier {
                Risk::Safe => String::new(),
                Risk::Moderate => format!("  {}", ui::yellow(tr("moderada"))),
                Risk::Advanced => format!("  {}", ui::red(tr("avanzada"))),
            };
            say!("  {}  {}{tier}", ui::cyan(&a.id), a.title);
            if let Some(n) = &a.note {
                say!("    {}", ui::dim(n));
            }
        }
        if deprovision && !ctx.info.elevated {
            say!(
                "{} {}",
                ui::yellow("●"),
                tr("--deprovision pide una terminal de administrador: se quitan solo de tu cuenta.")
            );
        }
    }
    let question = tf!("¿Quitar {n} apps? No se puede deshacer desde P-Rune: se reinstalan desde Microsoft Store.", n = picked.len());
    if !ui::confirm(&question, ctx.yes)? {
        say!("{}", tr("No se quitó nada."));
        return Ok((Vec::new(), None));
    }
    let ids: Vec<String> = picked.iter().map(|a| a.id.clone()).collect();
    let (results, journal) = remove_now(ctx, &ids, deprovision)?;
    let failed = results.iter().filter(|r| !r.ok).count();
    if !ctx.json {
        for r in &results {
            let title = picked.iter().find(|a| a.id == r.id).map_or(r.id.as_str(), |a| a.title.as_str());
            match &r.error {
                None => say!("{} {title}", ui::green("✓")),
                Some(e) => say!("{} {title}: {e}", ui::red("✗")),
            }
        }
    }
    if let Some(warning) = journal {
        note!("{} {warning}", ui::yellow("●"));
    }
    let failure = (failed > 0).then(|| tf!("No se pudieron quitar {n} apps.", n = failed));
    Ok((results, failure))
}

/// Quita ya, sin preguntar ni imprimir (eso es de quien llama: el subcomando o la interfaz
/// interactiva). Devuelve el resultado por app y, si no se pudo anotar en el historial, el aviso.
pub fn remove_now(ctx: &Ctx, ids: &[String], deprovision: bool) -> Result<(Vec<RemoveResult>, Option<String>), String> {
    // Con otra cuenta de administrador se quitarían las apps de esa cuenta, no las de la persona.
    if let Some(user) = &ctx.info.foreign_user {
        return Err(t!("foreign_user_remove", user = user));
    }
    let results = appx::remove(ids, deprovision && ctx.info.elevated)?;
    let removed: Vec<String> = results.iter().flat_map(|r| r.removed.clone()).collect();
    // Queda anotado en el historial: así `prune drift` avisa si Windows las vuelve a instalar.
    let journal = if removed.is_empty() { Ok(()) } else { ctx.engine.record_removed_apps(t!("label_removed_apps", n = removed.len()), removed) };
    Ok((results, journal.err().map(|e| t!("journal_save_failed", error = e))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, title: &str) -> BloatView {
        BloatView { id: id.into(), title: title.into(), tier: Risk::Safe, note: None, packages: vec![] }
    }

    #[test]
    fn busca_por_id_o_por_nombre() {
        let apps = vec![app("Microsoft.BingNews", "Microsoft Noticias"), app("Microsoft.BingWeather", "El Tiempo (MSN)"), app("Clipchamp.Clipchamp", "Clipchamp")];
        assert_eq!(resolve(&apps, "microsoft.bingnews").unwrap().id, "Microsoft.BingNews");
        assert_eq!(resolve(&apps, "tiempo").unwrap().id, "Microsoft.BingWeather");
        assert_eq!(resolve(&apps, "clip").unwrap().id, "Clipchamp.Clipchamp");
        assert!(resolve(&apps, "bing").unwrap_err().contains("Microsoft.BingWeather"));
        assert!(resolve(&apps, "spotify").is_err());
        assert!(resolve(&apps[..1], "").is_err());
    }
}
