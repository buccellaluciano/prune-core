//! Historial: ver, deshacer y detectar lo que Windows volvió atrás.

use crate::Ctx;
use crate::ui::{self, tr};
use prune_engine::engine::{Engine, JournalEntry};
use prune_engine::{appx, paths, power, proxy, registry, startup, wupdate};

/// Se puede deshacer: no se deshizo ya y no es solo "apps quitadas" (esas no vuelven desde acá).
pub(crate) fn undoable(e: &JournalEntry) -> bool {
    let special = e.power.is_some() || e.network.is_some() || e.proxy.is_some() || e.startup.is_some() || e.wu.is_some();
    !e.undone && (!e.changes.is_empty() || special)
}

/// Lo que deshacer esta entrada va a escribir, armado con lo que la entrada guarda de verdad
/// y no con su título: el título se puede leer como un cambio propio aunque la entrada
/// lleve otra cosa. Es lo que se muestra antes de confirmar.
pub(crate) fn undo_plan(e: &JournalEntry) -> Vec<String> {
    let mut out = Vec::new();
    for c in &e.changes {
        let name = if c.name.is_empty() { t!("default_value") } else { c.name.clone() };
        out.push(format!("{}\\{}\\{name} → {}", registry::hive_label(c.hive), c.path, registry::describe(c.before.as_ref())));
    }
    if let Some(p) = &e.power {
        let plan = power::parse(&p.before).map_or_else(|| p.before.clone(), |g| power::name(&g));
        out.push(format!("{} → {plan}", t!("power_title")));
    }
    if let Some(n) = &e.network {
        let servers = |list: &[String]| if list.is_empty() { tr("automático").to_string() } else { list.join(", ") };
        for a in &n.before {
            out.push(format!("DNS {} → IPv4 {} · IPv6 {}", a.name, servers(&a.v4.servers), servers(&a.v6.servers)));
        }
    }
    // Para estos tres, lo que la entrada hizo: deshacer es lo contrario.
    for done in [e.proxy.as_ref().map(proxy::label), e.startup.as_ref().map(startup::label), e.wu.as_ref().map(wupdate::label)].into_iter().flatten() {
        out.push(format!("↩ {done}"));
    }
    out
}

/// Sin administrador se ve el historial propio. Si además hay uno de administrador (el de la
/// app de escritorio, o el de Core en una terminal elevada), se avisa que existe.
fn admin_history_note(ctx: &Ctx) -> Option<String> {
    if ctx.info.elevated {
        return None;
    }
    let n = Engine::new(paths::admin_journal_dir()?).history().len();
    (n > 0).then(|| tf!("Hay además un historial de administrador con {n} entradas: abrí una terminal de administrador para verlo.", n = n))
}

pub fn history(ctx: &Ctx, limit: usize) -> Result<(), String> {
    let entries = ctx.engine.history();
    if ctx.json {
        return ui::json(&entries.iter().take(limit).collect::<Vec<_>>());
    }
    // Si no se pudo leer, se dice: "está vacío" a secas sería mentira.
    if let Some(w) = ctx.engine.take_warning() {
        anstream::println!("{} {w}", ui::yellow("●"));
    }
    let note = admin_history_note(ctx);
    if entries.is_empty() {
        anstream::println!("{}", tr("El historial está vacío."));
        if let Some(n) = &note {
            anstream::println!("{}", ui::dim(n));
        }
        return Ok(());
    }
    let when: Vec<String> = entries.iter().take(limit).map(|e| ui::ago(e.at)).collect();
    let width = when.iter().map(|w| ui::width(w)).max().unwrap_or(0);
    for (i, (e, when)) in entries.iter().zip(&when).enumerate() {
        let n = ui::pad(&format!("{}", i + 1), 3);
        let note = if e.undone {
            ui::dim(tr(" (deshecho)"))
        } else if !undoable(e) {
            ui::dim(tr(" (no se puede deshacer)"))
        } else {
            String::new()
        };
        let label = if e.undone { ui::dim(&e.label) } else { e.label.clone() };
        anstream::println!("{} {}  {label}{note}", ui::cyan(&n), ui::dim(&ui::pad(when, width)));
    }
    if entries.len() > limit {
        anstream::println!("{}", ui::dim(&format!("… {} {}", entries.len() - limit, tr("más (-n para ver más)"))));
    }
    anstream::println!("\n{} {}", ui::dim(tr("Para deshacer una entrada:")), ui::cyan("prune undo <N>"));
    if let Some(n) = &note {
        anstream::println!("{}", ui::dim(n));
    }
    Ok(())
}

/// Un número chico es la posición en `prune history`; uno grande, el id de la entrada.
fn pick(entries: &[JournalEntry], which: Option<u64>) -> Result<&JournalEntry, String> {
    match which {
        None => entries
            .iter()
            .find(|e| undoable(e))
            .ok_or_else(|| tr("No hay nada para deshacer.").to_string()),
        Some(n) if n >= 1 && (n as usize) <= entries.len() => Ok(&entries[n as usize - 1]),
        Some(id) => entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("{}: {id}. {} `prune history`.", tr("No existe esa entrada"), tr("Mirá"))),
    }
}

pub fn undo(ctx: &Ctx, which: Option<u64>, force: bool) -> Result<(), String> {
    let entries = ctx.engine.history();
    let entry = pick(&entries, which)?;
    ctx.need_yes_in_json()?;
    if !ctx.json {
        anstream::println!("{}  {}", ui::bold(&entry.label), ui::dim(&ui::ago(entry.at)));
        anstream::println!("{}", tr("Deshacer esta entrada va a escribir:"));
        for line in undo_plan(entry) {
            anstream::println!("  {line}");
        }
    }
    if !ui::confirm(tr("¿Deshacer esta entrada?"), ctx.yes)? {
        anstream::println!("{}", tr("No se cambió nada."));
        return Ok(());
    }
    let report = ctx.engine.undo(entry.id, force, ctx.info.foreign_user.as_deref())?;
    let failed = report.results.iter().filter(|r| !r.ok).count();
    let conflicted = !report.conflicts.is_empty() && !force;
    if ctx.json {
        ui::json(&report)?;
    } else {
        if conflicted {
            anstream::println!("{} {}", ui::yellow("●"), tr("Estos valores cambiaron después de esa entrada:"));
            for c in &report.conflicts {
                anstream::println!("  {c}");
            }
        }
        for r in report.results.iter().filter(|r| !r.ok) {
            anstream::println!("{} {}: {}", ui::red("✗"), r.id, r.error.as_deref().unwrap_or_default());
        }
        if let Some(w) = &report.warning {
            anstream::println!("{} {w}", ui::yellow("●"));
        }
    }
    if conflicted {
        return Err(tr("No se tocó nada. Para deshacer igual (pisando esos valores), repetí con --force.")
        .to_string());
    }
    if failed > 0 {
        return Err(tf!("No se pudieron restaurar {n} valores.", n = failed));
    }
    if !ctx.json {
        anstream::println!("{} {}", ui::green("✓"), tr("Deshecho."));
    }
    Ok(())
}

pub fn drift(ctx: &Ctx) -> Result<(), String> {
    let installed: Vec<(String, String)> = appx::list().unwrap_or_default().into_iter().map(|b| (b.id, b.title)).collect();
    let drifted = ctx.engine.drift(ctx.info.build, &installed);
    if ctx.json {
        return ui::json(&drifted);
    }
    if drifted.is_empty() {
        anstream::println!("{} {}", ui::green("✓"), tr("Todo sigue como lo dejaste."));
        return Ok(());
    }
    anstream::println!("{}", tr("Cambiaron desde que los aplicaste:"));
    let width = drifted.iter().map(|d| d.id.len()).max().unwrap_or(0);
    for d in &drifted {
        let kind = if d.kind == "bloat" { tr("app que volvió") } else { tr("ajuste") };
        anstream::println!("  {}  {}  {}", ui::cyan(&ui::pad(&d.id, width)), d.title, ui::dim(&format!("{kind}, {}", ui::ago(d.at))));
    }
    let tweaks: Vec<String> = drifted
        .iter()
        .filter(|d| d.kind == "tweak")
        .map(|d| match &d.choice {
            Some(c) => format!("{}={c}", d.id),
            None => d.id.clone(),
        })
        .collect();
    if !tweaks.is_empty() {
        anstream::println!("\n{} {}", ui::dim(tr("Para volver a aplicarlos:")), ui::cyan(&format!("prune apply {}", tweaks.join(" "))));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, json_extra: &str) -> JournalEntry {
        serde_json::from_str(&format!(r#"{{"id":{id},"at":{},"label":"x","changes":[]{json_extra}}}"#, id / 1000)).unwrap()
    }

    /// Lo que se confirma al deshacer sale de lo que la entrada guarda, no de su título.
    #[test]
    fn el_plan_de_deshacer_sale_del_contenido() {
        let e: JournalEntry = serde_json::from_str(
            r#"{"id":1000,"at":1,"label":"Un título cualquiera",
                "changes":[{"tweak_id":"taskbar.align_left","hive":"HKCU","path":"Software\\Prueba","name":"Valor","before":{"vtype":4,"bytes":[1,0,0,0]}},
                           {"tweak_id":"taskbar.align_left","hive":"HKLM","path":"SOFTWARE\\Otra","name":"Nuevo","before":null}],
                "network":{"before":[{"guid":"{g}","name":"Wi-Fi","v4":{"servers":["6.6.6.6"]},"v6":{"servers":[]}}],"after":[]}}"#,
        )
        .unwrap();
        let plan = undo_plan(&e);
        assert_eq!(plan.len(), 3, "{plan:?}");
        assert!(plan[0].starts_with(r"HKCU\Software\Prueba\Valor → "), "{plan:?}");
        assert!(plan[1].starts_with(r"HKLM\SOFTWARE\Otra\Nuevo → "), "{plan:?}");
        // Un cambio de DNS escondido detrás de otro título queda a la vista, con el servidor.
        assert!(plan[2].contains("Wi-Fi") && plan[2].contains("6.6.6.6"), "{plan:?}");
        // Y una entrada que solo quitó apps no escribe nada.
        assert!(undo_plan(&entry(1, r#","removed_apps":["A_1"]"#)).is_empty());
    }

    #[test]
    fn elige_la_ultima_que_se_puede_deshacer() {
        // Como las devuelve `Engine::history`: la más nueva primero.
        let entries = vec![
            entry(3000, r#","removed_apps":["A_1"]"#),
            entry(2000, r#","undone":true,"power":{"before":"a","after":"b"}"#),
            entry(1000, r#","power":{"before":"a","after":"b"}"#),
        ];
        assert_eq!(pick(&entries, None).unwrap().id, 1000);
        // Por posición (como en `prune history`) o por id.
        assert_eq!(pick(&entries, Some(1)).unwrap().id, 3000);
        assert_eq!(pick(&entries, Some(2000)).unwrap().id, 2000);
        assert!(pick(&entries, Some(99)).is_err());
        assert!(pick(&entries[..2], None).is_err());
    }
}
