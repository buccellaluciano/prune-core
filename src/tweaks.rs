//! Ajustes: estado, búsqueda, detalle y aplicar o revertir con plan previo.

use crate::Ctx;
use crate::ui::{self, tr};
use prune_engine::catalog::{self, Restart, Risk};
use prune_engine::engine::{self, ChoiceView, Desired, PlannedChange, TweakState, TweakView};
use prune_engine::i18n;
use serde::Serialize;

// ───────────────────────── Textos chicos ─────────────────────────

fn state_mark(s: TweakState) -> String {
    match s {
        TweakState::Applied => ui::green("●"),
        TweakState::Default => ui::dim("○"),
        TweakState::Partial => ui::yellow("◐"),
        TweakState::Unsupported => ui::dim("–"),
        TweakState::Error => ui::red("!"),
    }
}

pub(crate) fn state_text(s: TweakState) -> &'static str {
    match s {
        TweakState::Applied => tr("aplicado"),
        TweakState::Default => tr("como lo trae Windows"),
        TweakState::Partial => tr("a medias"),
        TweakState::Unsupported => tr("no disponible en esta versión de Windows"),
        TweakState::Error => tr("no se pudo leer"),
    }
}

pub(crate) fn risk_text(r: Risk) -> &'static str {
    match r {
        Risk::Safe => tr("seguro"),
        Risk::Moderate => tr("moderado"),
        Risk::Advanced => tr("avanzado"),
    }
}

pub(crate) fn restart_text(r: Restart) -> Option<&'static str> {
    match r {
        Restart::None => None,
        Restart::Explorer => Some(tr("reiniciar el Explorador")),
        Restart::Logoff => Some(tr("cerrar sesión")),
        Restart::Reboot => Some(tr("reiniciar la PC")),
    }
}

/// Etiquetas a la derecha de cada ajuste: riesgo (si no es seguro) y si pide administrador.
fn badges(v: &TweakView) -> String {
    let mut out = Vec::new();
    match v.risk {
        Risk::Safe => {}
        Risk::Moderate => out.push(ui::yellow(risk_text(v.risk))),
        Risk::Advanced => out.push(ui::red(risk_text(v.risk))),
    }
    if v.requires_admin {
        out.push(ui::dim("admin"));
    }
    out.join(" ")
}

pub(crate) fn category_title(id: &str) -> String {
    catalog::tweaks().category.iter().find(|c| c.id == id).map_or_else(|| id.to_string(), |c| i18n::category(&c.id, &c.title))
}

/// ¿`haystack` contiene `needle`? Sin distinguir mayúsculas ni acentos.
fn contains(haystack: &str, needle: &str) -> bool {
    ui::fold(haystack).contains(&ui::fold(needle))
}

// ───────────────────────── Buscar por id ─────────────────────────

/// Un ajuste por su id, o por una parte del id si no hay dudas de cuál es.
pub fn resolve<'a>(views: &'a [TweakView], query: &str) -> Result<&'a TweakView, String> {
    if let Some(v) = views.iter().find(|v| v.id.eq_ignore_ascii_case(query)) {
        return Ok(v);
    }
    let q = query.to_lowercase();
    let hits: Vec<&TweakView> = views.iter().filter(|v| contains(&v.id, &q)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "{}: {query}. {} `prune list {query}`.",
            tr("No hay ningún ajuste con ese id"),
            tr("Para buscar por texto:")
        )),
        many => {
            let ids: Vec<&str> = many.iter().take(8).map(|v| v.id.as_str()).collect();
            let more = if many.len() > ids.len() { ", …" } else { "" };
            Err(format!("`{query}` {}: {}{more}", tr("coincide con varios ajustes"), ids.join(", ")))
        }
    }
}

fn resolve_choice<'a>(v: &'a TweakView, query: &str) -> Result<&'a ChoiceView, String> {
    if let Some(c) = v.choices.iter().find(|c| c.id.eq_ignore_ascii_case(query)) {
        return Ok(c);
    }
    let q = query.to_lowercase();
    let hits: Vec<&ChoiceView> = v.choices.iter().filter(|c| contains(&c.id, &q) || contains(&c.label, &q)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        _ => Err(format!("{}: {query}. {}", tr("Opción desconocida"), choices_hint(v))),
    }
}

fn choices_hint(v: &TweakView) -> String {
    let opts: Vec<String> = v.choices.iter().map(|c| format!("{}={} ({})", v.id, c.id, c.label)).collect();
    format!("{}: {}", tr("Opciones"), opts.join(", "))
}

/// `id` o `id=opción` → lo que se le pide al motor.
fn desired_for(views: &[TweakView], arg: &str, on: bool) -> Result<Desired, String> {
    let (id, choice) = match arg.split_once('=') {
        Some((id, c)) => (id, Some(c)),
        None => (arg, None),
    };
    let v = resolve(views, id)?;
    if v.state == TweakState::Unsupported {
        return Err(format!("{}: {}", v.id, state_text(v.state)));
    }
    if v.choices.is_empty() {
        if choice.is_some() {
            return Err(format!("{}: {}", v.id, tr("este ajuste no tiene opciones")));
        }
        return Ok(Desired { id: v.id.clone(), applied: on, choice: None });
    }
    let chosen = if on {
        let c = choice.ok_or_else(|| format!("{}: {}. {}", v.id, tr("hay que elegir una opción"), choices_hint(v)))?;
        resolve_choice(v, c)?
    } else {
        // Revertir un ajuste con opciones es volver a la opción de Windows.
        v.choices
            .iter()
            .find(|c| c.default)
            .ok_or_else(|| format!("{}: {}", v.id, tr("no tiene una opción por defecto a la que volver")))?
    };
    Ok(Desired { id: v.id.clone(), applied: on, choice: Some(chosen.id.clone()) })
}

// ───────────────────────── status ─────────────────────────

#[derive(Serialize)]
struct Status<'a> {
    version: &'static str,
    system: &'a prune_engine::system::SystemInfo,
    journal: String,
    tweaks_applied: usize,
    tweaks_available: usize,
    history_entries: usize,
    drifted: Vec<String>,
}

pub fn status(ctx: &Ctx) -> Result<(), String> {
    let views = engine::list(ctx.info.build);
    let available: Vec<&TweakView> = views.iter().filter(|v| v.state != TweakState::Unsupported).collect();
    let applied = available.iter().filter(|v| v.state == TweakState::Applied).count();
    let entries = ctx.engine.history().len();
    // Solo ajustes: revisar las apps que volvieron tarda más (eso lo hace `prune drift`).
    let drifted: Vec<String> = ctx.engine.drift(ctx.info.build, &[]).into_iter().map(|d| d.id).collect();
    if ctx.json {
        return ui::json(&Status {
            version: env!("CARGO_PKG_VERSION"),
            system: &ctx.info,
            journal: ctx.journal_dir.join("journal.json").display().to_string(),
            tweaks_applied: applied,
            tweaks_available: available.len(),
            history_entries: entries,
            drifted,
        });
    }

    let i = &ctx.info;
    anstream::println!("{} {}", ui::bold("P-Rune Core"), env!("CARGO_PKG_VERSION"));
    anstream::println!("{} {} ({}.{})", i.product, i.display_version, i.build, i.ubr);
    if i.elevated {
        anstream::println!("{} {}", ui::green("●"), tr("Terminal de administrador: se puede aplicar todo."));
    } else {
        anstream::println!(
            "{} {}",
            ui::yellow("●"),
            tr("Sin administrador: se puede mirar todo; los ajustes marcados `admin` piden una terminal de administrador.")
        );
    }
    if let Some(user) = &i.foreign_user {
        anstream::println!("{} {}", ui::yellow("●"), tf!("Esta terminal corre con otra cuenta que la de la sesión de {user}.", user = user));
    }
    anstream::println!();
    anstream::println!("{}", tf!("Ajustes aplicados: {n} de {total}", n = applied, total = available.len()));
    for c in &catalog::tweaks().category {
        let in_cat: Vec<&&TweakView> = available.iter().filter(|v| v.category == c.id).collect();
        let on = in_cat.iter().filter(|v| v.state == TweakState::Applied).count();
        if !in_cat.is_empty() {
            let count = format!("{on}/{}", in_cat.len());
            let count = if on > 0 { ui::green(&ui::pad(&count, 6)) } else { ui::dim(&ui::pad(&count, 6)) };
            anstream::println!("  {count} {}", i18n::category(&c.id, &c.title));
        }
    }
    anstream::println!();
    anstream::println!("{} {entries}", tr("Entradas en el historial:"));
    // Si el historial no se pudo leer, se dice: "0 entradas" a secas sería mentira.
    if let Some(w) = ctx.engine.take_warning() {
        anstream::println!("{} {w}", ui::yellow("●"));
    }
    if !drifted.is_empty() {
        anstream::println!(
            "{} {} {} {}",
            ui::yellow("●"),
            drifted.len(),
            tr("ajustes que aplicaste ya no están como los dejaste:"),
            ui::cyan("prune drift")
        );
    }
    Ok(())
}

// ───────────────────────── list / show ─────────────────────────

pub fn list(ctx: &Ctx, search: Option<&str>, category: Option<&str>, only_applied: bool) -> Result<(), String> {
    let views = engine::list(ctx.info.build);
    let search = search.map(str::to_lowercase);
    let category = category.map(str::to_lowercase);
    let shown: Vec<&TweakView> = views
        .iter()
        .filter(|v| !only_applied || v.state == TweakState::Applied)
        .filter(|v| category.as_ref().is_none_or(|c| v.category.eq_ignore_ascii_case(c) || contains(&category_title(&v.category), c)))
        .filter(|v| {
            search.as_ref().is_none_or(|q| {
                contains(&v.id, q) || contains(&v.title, q) || contains(&v.description, q) || v.tags.iter().any(|t| contains(t, q))
            })
        })
        .collect();
    if ctx.json {
        return ui::json(&shown);
    }
    if shown.is_empty() {
        anstream::println!("{}", tr("Ningún ajuste coincide."));
        return Ok(());
    }
    let width = shown.iter().map(|v| v.id.len()).max().unwrap_or(0);
    // En el orden del catálogo; lo que no tenga categoría conocida, al final.
    let mut cats: Vec<String> = catalog::tweaks().category.iter().map(|c| c.id.clone()).collect();
    for v in &shown {
        if !cats.contains(&v.category) {
            cats.push(v.category.clone());
        }
    }
    for cat in cats {
        let rows: Vec<&&TweakView> = shown.iter().filter(|v| v.category == cat).collect();
        if rows.is_empty() {
            continue;
        }
        anstream::println!("\n{}", ui::bold(&category_title(&cat)));
        for v in rows {
            // Con opciones, lo que importa es cuál está puesta.
            let current = v.choice.as_ref().and_then(|id| v.choices.iter().find(|c| &c.id == id)).map(|c| format!(" = {}", c.label));
            let title = format!("{}{}", v.title, current.unwrap_or_default());
            let title = if v.state == TweakState::Unsupported { ui::dim(&title) } else { title };
            anstream::println!("  {} {}  {title}  {}", state_mark(v.state), ui::cyan(&ui::pad(&v.id, width)), badges(v));
        }
    }
    anstream::println!(
        "\n{} {} {}  {} {}  {} {}  {} {}",
        ui::dim(tr("Referencias:")),
        ui::green("●"),
        ui::dim(tr("aplicado")),
        ui::dim("○"),
        ui::dim(tr("como lo trae Windows")),
        ui::yellow("◐"),
        ui::dim(tr("a medias")),
        ui::dim("–"),
        ui::dim(tr("no disponible")),
    );
    Ok(())
}

pub fn show(ctx: &Ctx, id: &str) -> Result<(), String> {
    let views = engine::list(ctx.info.build);
    let v = resolve(&views, id)?;
    if ctx.json {
        return ui::json(v);
    }
    anstream::println!("{}  {}", ui::bold(&v.title), ui::cyan(&v.id));
    anstream::println!("{}", v.description);
    anstream::println!();
    anstream::println!("{} {} {}", tr("Estado:"), state_mark(v.state), state_text(v.state));
    anstream::println!("{} {}", tr("Riesgo:"), risk_text(v.risk));
    if v.requires_admin {
        anstream::println!("{}", tr("Requiere una terminal de administrador."));
    }
    if v.ucpd {
        anstream::println!("{}", tr("Windows protege este valor (UCPD): puede que no deje cambiarlo."));
    }
    if let Some(r) = restart_text(v.restart) {
        anstream::println!("{}", tf!("Para que se vea hace falta: {r}.", r = r));
    }
    if !v.choices.is_empty() {
        anstream::println!("\n{}", tr("Opciones:"));
        for c in &v.choices {
            let mark = if v.choice.as_deref() == Some(c.id.as_str()) { ui::green("●") } else { ui::dim("○") };
            let default = if c.default { ui::dim(tr(" (la de Windows)")) } else { String::new() };
            anstream::println!("  {mark} {}  {}{default}", ui::cyan(&format!("{}={}", v.id, c.id)), c.label);
        }
    }
    anstream::println!("\n{}", tr("Qué toca:"));
    for op in &v.ops {
        anstream::println!("  {}\\{}", op.key, ui::bold(&op.name));
        // Las etiquetas se alinean según el idioma, no con espacios escritos a mano.
        let labels = [tr("ahora:"), tr("aplicado:"), tr("revertido:")];
        let width = labels.iter().map(|l| ui::width(l)).max().unwrap_or(0);
        anstream::println!("    {} {}", ui::dim(&ui::pad(labels[0], width)), op.current);
        anstream::println!("    {} {}", ui::dim(&ui::pad(labels[1], width)), op.on);
        if !op.off.is_empty() {
            anstream::println!("    {} {}", ui::dim(&ui::pad(labels[2], width)), op.off);
        }
    }
    Ok(())
}

// ───────────────────────── apply / revert ─────────────────────────

#[derive(Serialize)]
struct Outcome {
    plan: Vec<PlannedChange>,
    /// None en `--dry-run` o si no había nada que cambiar.
    report: Option<engine::ApplyReport>,
}

pub fn apply(ctx: &Ctx, args: &[String], on: bool, dry_run: bool) -> Result<(), String> {
    let views = engine::list(ctx.info.build);
    let mut desired: Vec<Desired> = Vec::new();
    for a in args {
        let d = desired_for(&views, a, on)?;
        // El mismo ajuste dos veces: vale lo último que se pidió.
        desired.retain(|x| x.id != d.id);
        desired.push(d);
    }
    run_plan(ctx, &views, desired, dry_run)
}

/// Muestra el plan, confirma y aplica. Lo usan `apply`, `revert` y `profile import`.
pub fn run_plan(ctx: &Ctx, views: &[TweakView], desired: Vec<Desired>, dry_run: bool) -> Result<(), String> {
    let plan = engine::plan(&desired, ctx.info.build)?;
    let changing = |id: &str| plan.iter().any(|p| p.tweak_id == id && !p.noop);
    // Lo que ya está como se pidió no se toca (ni ensucia el historial).
    let desired: Vec<Desired> = desired.into_iter().filter(|d| changing(&d.id)).collect();
    let view = |id: &str| views.iter().find(|v| v.id == id);

    if !ctx.json {
        let mut seen: Vec<&str> = Vec::new();
        for p in &plan {
            if !seen.contains(&p.tweak_id.as_str()) {
                seen.push(&p.tweak_id);
                let title = view(&p.tweak_id).map_or(p.tweak_id.as_str(), |v| v.title.as_str());
                anstream::println!("{}  {}", ui::bold(title), ui::cyan(&p.tweak_id));
            }
            if p.noop {
                anstream::println!("  {}", ui::dim(&format!("{}\\{}: {} ({})", p.key, p.name, p.from, tr("ya está así"))));
            } else {
                anstream::println!("  {}\\{}: {} {} {}", p.key, p.name, ui::dim(&p.from), ui::dim("→"), ui::green(&p.to));
            }
        }
    }

    if desired.is_empty() {
        if ctx.json {
            return ui::json(&Outcome { plan, report: None });
        }
        anstream::println!("\n{}", tr("Ya está todo así: no hay nada que cambiar."));
        return Ok(());
    }

    let need_admin: Vec<&str> =
        desired.iter().filter(|d| view(&d.id).is_some_and(|v| v.requires_admin)).map(|d| d.id.as_str()).collect();
    let blocked = if !need_admin.is_empty() && !ctx.info.elevated {
        Some(format!(
            "{}: {}. {}",
            tr("Piden una terminal de administrador"),
            need_admin.join(", "),
            tr("Abrí una como administrador o repetí el comando con `sudo`.")
        ))
    } else {
        None
    };
    let restart = desired.iter().filter_map(|d| view(&d.id)).map(|v| v.restart).max().unwrap_or_default();

    if dry_run {
        if ctx.json {
            return ui::json(&Outcome { plan, report: None });
        }
        anstream::println!();
        if let Some(why) = &blocked {
            anstream::println!("{} {why}", ui::yellow("●"));
        }
        if let Some(r) = restart_text(restart) {
            anstream::println!("{}", tf!("Para que se vea hace falta: {r}.", r = r));
        }
        anstream::println!("{}", ui::dim(tr("Vista previa: no se cambió nada.")));
        return Ok(());
    }
    if let Some(why) = blocked {
        return Err(why);
    }
    ctx.need_yes_in_json()?;
    if !ctx.json {
        anstream::println!();
    }
    let question = tf!("¿Aplicar {n} ajustes?", n = desired.len());
    if !ui::confirm(&question, ctx.yes)? {
        anstream::println!("{}", tr("No se cambió nada."));
        return Ok(());
    }

    let report = ctx.engine.apply(&desired, ctx.info.elevated, ctx.info.build, ctx.info.foreign_user.as_deref());
    let failed = report.results.iter().filter(|r| !r.ok).count();
    if ctx.json {
        ui::json(&Outcome { plan, report: Some(report) })?;
    } else {
        for r in &report.results {
            let title = view(&r.id).map_or(r.id.as_str(), |v| v.title.as_str());
            match &r.error {
                None => anstream::println!("{} {title}", ui::green("✓")),
                Some(e) => anstream::println!("{} {title}: {e}", ui::red("✗")),
            }
        }
        if let Some(w) = &report.warning {
            anstream::println!("{} {w}", ui::yellow("●"));
        }
        if let Some(r) = restart_text(report.restart) {
            anstream::println!("{}", tf!("Para que se vea hace falta: {r}.", r = r));
        }
        if report.entry_id.is_some() {
            anstream::println!("{} {}", ui::dim(tr("Para deshacerlo:")), ui::cyan("prune undo"));
        }
    }
    if failed > 0 {
        return Err(tf!("No se pudieron aplicar {n} cambios.", n = failed));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: &str, choices: &[(&str, &str, bool)]) -> TweakView {
        TweakView {
            id: id.into(),
            category: "test".into(),
            title: id.to_uppercase(),
            description: String::new(),
            risk: Risk::Safe,
            restart: Restart::None,
            tags: vec![],
            requires_admin: false,
            per_user: true,
            ucpd: false,
            state: TweakState::Default,
            ops: vec![],
            choices: choices.iter().map(|(id, label, default)| ChoiceView { id: (*id).into(), label: (*label).into(), default: *default }).collect(),
            choice: None,
        }
    }

    fn views() -> Vec<TweakView> {
        vec![
            view("taskbar.align_left", &[]),
            view("taskbar.end_task", &[]),
            view("search.taskbar_mode", &[("hidden", "Oculta", false), ("icon", "Ícono", false), ("box", "Cuadro", true)]),
        ]
    }

    #[test]
    fn busca_por_id_o_por_una_parte_sin_dudas() {
        let v = views();
        assert_eq!(resolve(&v, "taskbar.align_left").unwrap().id, "taskbar.align_left");
        assert_eq!(resolve(&v, "ALIGN").unwrap().id, "taskbar.align_left");
        // Ambiguo: nombra los candidatos en vez de elegir uno.
        let e = resolve(&v, "taskbar").unwrap_err();
        assert!(e.contains("taskbar.align_left") && e.contains("taskbar.end_task") && e.contains("search.taskbar_mode"));
        assert!(resolve(&v, "no-existe").is_err());
    }

    #[test]
    fn el_id_exacto_gana_aunque_sea_parte_de_otro() {
        let v = vec![view("a.b", &[]), view("a.b.c", &[])];
        assert_eq!(resolve(&v, "a.b").unwrap().id, "a.b");
    }

    #[test]
    fn ajustes_simples() {
        let v = views();
        let d = desired_for(&v, "align", true).unwrap();
        assert_eq!((d.id.as_str(), d.applied, d.choice), ("taskbar.align_left", true, None));
        assert!(!desired_for(&v, "align", false).unwrap().applied);
        assert!(desired_for(&v, "align=x", true).is_err());
    }

    #[test]
    fn ajustes_con_opciones() {
        let v = views();
        // Sin opción no se adivina: el error dice cuáles hay.
        let e = desired_for(&v, "search.taskbar_mode", true).unwrap_err();
        assert!(e.contains("search.taskbar_mode=hidden"));
        assert_eq!(desired_for(&v, "search.taskbar_mode=icon", true).unwrap().choice.as_deref(), Some("icon"));
        // Por el nombre que se ve en pantalla también.
        assert_eq!(desired_for(&v, "search.taskbar_mode=oculta", true).unwrap().choice.as_deref(), Some("hidden"));
        assert!(desired_for(&v, "search.taskbar_mode=nada", true).is_err());
        // Revertir vuelve a la opción de Windows.
        assert_eq!(desired_for(&v, "search.taskbar_mode", false).unwrap().choice.as_deref(), Some("box"));
    }

    #[test]
    fn lo_no_disponible_no_se_aplica() {
        let mut v = views();
        v[0].state = TweakState::Unsupported;
        assert!(desired_for(&v, "taskbar.align_left", true).is_err());
    }
}
