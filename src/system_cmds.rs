//! Lo que no son ajustes del catálogo pero también queda en el historial: Windows Update,
//! lo que arranca con Windows y el proxy. Y los puntos de restauración.
//!
//! Son comandos directos (decís exactamente qué querés y se hace), sin plan previo: cada uno
//! deja una entrada en el historial y se deshace con `prune undo`.

use crate::Ctx;
use crate::ui::{self, tr};
use clap::Subcommand;
use prune_engine::{proxy, restore, startup, wupdate};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct Done {
    changed: bool,
    entry_id: Option<u64>,
    label: Option<String>,
}

/// Cierra un cambio ya hecho: lo anota en el historial (si no se puede, lo vuelve atrás: sin
/// historial no habría cómo deshacerlo) y cuenta qué pasó.
fn finish<C>(ctx: &Ctx, change: Option<C>, label: fn(&C) -> String, record: impl FnOnce(C) -> Result<u64, String>, rollback: impl FnOnce(&C)) -> Result<(), String>
where
    C: Clone,
{
    let Some(change) = change else {
        if ctx.json {
            return ui::json(&Done { changed: false, entry_id: None, label: None });
        }
        anstream::println!("{}", tr("Ya estaba así: no se cambió nada."));
        return Ok(());
    };
    let text = label(&change);
    let entry_id = record(change.clone()).map_err(|e| {
        rollback(&change);
        t!("journal_save_failed", error = e)
    })?;
    if ctx.json {
        return ui::json(&Done { changed: true, entry_id: Some(entry_id), label: Some(text) });
    }
    anstream::println!("{} {text}", ui::green("✓"));
    anstream::println!("{} {}", ui::dim(tr("Para deshacerlo:")), ui::cyan("prune undo"));
    Ok(())
}

fn yes_no(b: bool) -> String {
    if b { ui::green(tr("sí")) } else { ui::dim(tr("no")) }
}

// ───────────────────────── Windows Update ─────────────────────────

#[derive(Subcommand)]
pub enum WuCmd {
    /// Si está en pausa y cuáles son las horas activas
    Status,
    /// Pausa las actualizaciones por una cantidad de días
    Pause {
        #[arg(value_name = "DÍAS")]
        dias: u32,
    },
    /// Reanuda las actualizaciones
    Resume,
    /// Horas activas: Windows no reinicia solo en ese horario
    Hours {
        /// Hora de inicio (0 a 23)
        #[arg(value_name = "INICIO", requires = "fin")]
        inicio: Option<u32>,
        /// Hora de fin (0 a 23)
        #[arg(value_name = "FIN")]
        fin: Option<u32>,
        /// Que las decida Windows según el uso
        #[arg(long, conflicts_with = "inicio")]
        auto: bool,
    },
}

pub fn wu(ctx: &Ctx, cmd: Option<WuCmd>) -> Result<(), String> {
    let change = match cmd.unwrap_or(WuCmd::Status) {
        WuCmd::Status => return wu_status(ctx),
        WuCmd::Pause { dias } => {
            wupdate::pause(dias)?
        }
        WuCmd::Resume => {
            wupdate::resume()?
        }
        WuCmd::Hours { inicio, fin, auto } => {
            match (inicio, fin, auto) {
                (Some(a), Some(b), false) => wupdate::hours(Some((a, b)))?,
                (None, None, true) => wupdate::hours(None)?,
                _ => return Err(tr("Indicá la hora de inicio y la de fin (por ejemplo `prune wu hours 9 23`), o --auto.").to_string()),
            }
        }
    };
    finish(ctx, change, wupdate::label, |c| ctx.engine.record_wu(c), |c| drop(wupdate::restore(c)))
}

fn wu_status(ctx: &Ctx) -> Result<(), String> {
    let s = wupdate::status();
    if ctx.json {
        return ui::json(&s);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match s.paused_until.filter(|t| *t > now) {
        Some(until) => {
            let left = until - now;
            let text = if left >= 86_400 { tf!("En pausa: quedan {n} días.", n = left / 86_400) } else { tf!("En pausa: quedan {n} horas.", n = (left / 3600).max(1)) };
            anstream::println!("{} {text}", ui::yellow("●"));
        }
        None => anstream::println!("{} {}", ui::green("●"), tr("Las actualizaciones no están en pausa.")),
    }
    anstream::println!("{}", ui::dim(&tf!("Se pueden pausar hasta {n} días.", n = s.max_pause_days)));
    match (s.active_start, s.active_end, s.smart_hours) {
        (_, _, Some(true)) => anstream::println!("{}", tr("Horas activas: las decide Windows según el uso.")),
        (Some(a), Some(b), _) => anstream::println!("{}", tf!("Horas activas: de {a} a {b} h.", a = a, b = b)),
        _ => {}
    }
    if s.pause_policy || s.hours_policy {
        anstream::println!("{} {}", ui::yellow("●"), tr("Una directiva de la organización controla parte de esto: puede que no se pueda cambiar."));
    }
    if !s.elevated {
        anstream::println!("{}", ui::dim(tr("Cambiarlo pide una terminal de administrador.")));
    }
    Ok(())
}

// ───────────────────────── Inicio de Windows ─────────────────────────

#[derive(Subcommand)]
pub enum StartupCmd {
    /// Lo que arranca con Windows
    List,
    /// Deja de arrancar con Windows (no lo desinstala)
    Disable {
        /// Nombre según `prune startup list` (alcanza con una parte)
        #[arg(value_name = "NOMBRE")]
        nombre: String,
    },
    /// Vuelve a arrancar con Windows
    Enable {
        #[arg(value_name = "NOMBRE")]
        nombre: String,
    },
}

fn source_text(s: startup::Source) -> &'static str {
    match s {
        startup::Source::RunUser => tr("tu usuario"),
        startup::Source::RunMachine => tr("todo el equipo"),
        startup::Source::RunMachine32 => tr("todo el equipo (32 bits)"),
        startup::Source::FolderUser => tr("carpeta Inicio"),
        startup::Source::FolderMachine => tr("carpeta Inicio de todos"),
        startup::Source::Task => tr("tarea programada"),
    }
}

fn find_startup<'a>(items: &'a [startup::StartupItem], query: &str) -> Result<&'a startup::StartupItem, String> {
    let q = ui::fold(query);
    let exact: Vec<&startup::StartupItem> = items.iter().filter(|i| i.name.to_lowercase() == q || i.title.to_lowercase() == q).collect();
    let hits = if exact.is_empty() { items.iter().filter(|i| ui::fold(&i.name).contains(&q) || ui::fold(&i.title).contains(&q)).collect() } else { exact };
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(tf!("Nada que arranque con Windows se llama así: {query}. Mirá `prune startup list`.", query = query)),
        many => {
            let names: Vec<String> = many.iter().take(8).map(|i| format!("{} ({})", i.name, source_text(i.source))).collect();
            Err(tf!("`{query}` coincide con varios: {names}", query = query, names = names.join(", ")))
        }
    }
}

pub fn startup(ctx: &Ctx, cmd: Option<StartupCmd>) -> Result<(), String> {
    let status = startup::status();
    let (name, enabled) = match cmd.unwrap_or(StartupCmd::List) {
        StartupCmd::List => return startup_list(ctx, &status),
        StartupCmd::Disable { nombre } => (nombre, false),
        StartupCmd::Enable { nombre } => (nombre, true),
    };
    let item = find_startup(&status.items, &name)?;
    let change = startup::set(item.source, &item.name, enabled)?;
    finish(ctx, change, startup::label, |c| ctx.engine.record_startup(c), |c| drop(startup::restore(c)))
}

fn startup_list(ctx: &Ctx, status: &startup::StartupStatus) -> Result<(), String> {
    if ctx.json {
        return ui::json(status);
    }
    if status.items.is_empty() {
        anstream::println!("{}", tr("Nada arranca con Windows."));
        return Ok(());
    }
    let width = status.items.iter().map(|i| ui::width(&i.title)).max().unwrap_or(0).min(40);
    for i in &status.items {
        let mark = if i.enabled { ui::green("●") } else { ui::dim("○") };
        let mut notes = vec![source_text(i.source).to_string()];
        if let Some(p) = &i.publisher {
            notes.push(p.clone());
        }
        if i.system {
            notes.push(tr("de Windows").to_string());
        }
        if !i.exists {
            notes.push(tr("el archivo ya no existe").to_string());
        }
        let title = if i.enabled { ui::pad(&i.title, width) } else { ui::dim(&ui::pad(&i.title, width)) };
        anstream::println!("  {mark} {title}  {}", ui::dim(&notes.join(" · ")));
    }
    anstream::println!("\n{} {}", ui::dim(tr("Para que algo deje de arrancar:")), ui::cyan(&format!("prune startup disable <{}>", tr("NOMBRE").to_lowercase())));
    Ok(())
}

// ───────────────────────── Proxy ─────────────────────────

#[derive(Subcommand)]
pub enum ProxyCmd {
    /// Cómo está configurado el proxy de tu usuario
    Status,
    /// Prende o apaga el servidor proxy ya configurado
    Manual { estado: Switch },
    /// Prende o apaga el script de configuración (PAC)
    Pac { estado: Switch },
    /// Prende o apaga "Detectar la configuración automáticamente"
    Auto { estado: Switch },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Switch {
    On,
    Off,
}

pub fn proxy(ctx: &Ctx, cmd: Option<ProxyCmd>) -> Result<(), String> {
    let (kind, state) = match cmd.unwrap_or(ProxyCmd::Status) {
        ProxyCmd::Status => return proxy_status(ctx),
        ProxyCmd::Manual { estado } => (proxy::Kind::Manual, estado),
        ProxyCmd::Pac { estado } => (proxy::Kind::Pac, estado),
        ProxyCmd::Auto { estado } => (proxy::Kind::AutoDetect, estado),
    };
    let change = proxy::set(kind, matches!(state, Switch::On))?;
    finish(ctx, change, proxy::label, |c| ctx.engine.record_proxy(c), |c| drop(proxy::restore(c)))
}

fn proxy_status(ctx: &Ctx) -> Result<(), String> {
    let s = proxy::status()?;
    if ctx.json {
        return ui::json(&s);
    }
    let detail = |text: &str| if text.is_empty() { String::new() } else { format!("  {}", ui::dim(text)) };
    anstream::println!("{}  {}{}", ui::pad(tr("Servidor proxy:"), 28), yes_no(s.manual), detail(&s.server));
    anstream::println!("{}  {}{}", ui::pad(tr("Script de configuración:"), 28), yes_no(s.pac), detail(&s.pac_url));
    anstream::println!("{}  {}", ui::pad(tr("Detectar automáticamente:"), 28), yes_no(s.auto_detect));
    if let Some(w) = &s.winhttp {
        anstream::println!("{}  {w}", ui::pad(tr("Proxy del sistema (WinHTTP):"), 28));
    }
    if s.policy {
        anstream::println!("{} {}", ui::yellow("●"), tr("Una directiva de la organización controla el proxy: puede que no se pueda cambiar."));
    }
    if let Some(user) = &s.foreign_user {
        anstream::println!("{} {}", ui::yellow("●"), tf!("Esta terminal corre con otra cuenta que la de la sesión de {user}.", user = user));
    }
    Ok(())
}

// ───────────────────────── Puntos de restauración ─────────────────────────

#[derive(Subcommand)]
pub enum RestoreCmd {
    /// Si la protección del sistema está activada y cuál fue el último punto
    Status,
    /// Crea un punto de restauración ahora
    Create,
}

pub fn restore(ctx: &Ctx, cmd: Option<RestoreCmd>) -> Result<(), String> {
    match cmd.unwrap_or(RestoreCmd::Status) {
        RestoreCmd::Status => {
            let s = restore::status();
            if ctx.json {
                return ui::json(&s);
            }
            if s.enabled {
                anstream::println!("{} {}", ui::green("●"), tr("La protección del sistema está activada."));
            } else {
                anstream::println!("{} {}", ui::yellow("●"), tr("La protección del sistema está desactivada: no se pueden crear puntos de restauración."));
            }
            if s.policy {
                anstream::println!("{} {}", ui::yellow("●"), tr("Una directiva de la organización la desactiva."));
            }
            if !s.readable {
                anstream::println!("{}", ui::dim(tr("Para ver los puntos que hay hace falta una terminal de administrador.")));
            } else {
                anstream::println!("{}", tf!("Puntos de restauración: {n}", n = s.count));
                if let Some(last) = &s.last {
                    anstream::println!("{} {}  {}", tr("Último:"), last.description, ui::dim(&ui::ago(last.at)));
                }
            }
            Ok(())
        }
        RestoreCmd::Create => {
            if !ctx.info.elevated {
                return Err(t!("requires_admin"));
            }
            if !ctx.json {
                anstream::eprintln!("{}", ui::dim(tr("Creando el punto de restauración (puede tardar un minuto)…")));
            }
            let created = restore::create(&t!("restore_description"))?;
            if ctx.json {
                return ui::json(&created);
            }
            if created.reused {
                anstream::println!("{} {}", ui::green("✓"), tf!("Ya había uno reciente ({when}): Windows no crea otro tan seguido.", when = ui::ago(created.at)));
            } else {
                anstream::println!("{} {}", ui::green("✓"), tr("Punto de restauración creado."));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use startup::{Source, StartupItem};

    fn item(source: Source, name: &str, title: &str) -> StartupItem {
        StartupItem { source, name: name.into(), title: title.into(), command: String::new(), path: None, exists: true, publisher: None, system: false, enabled: true }
    }

    #[test]
    fn busca_lo_que_arranca_por_nombre() {
        let items = vec![item(Source::RunUser, "Spotify", "Spotify"), item(Source::RunUser, "SpotifyHelper", "Spotify Web Helper"), item(Source::Task, "OneDrive", "Microsoft OneDrive")];
        // El nombre exacto gana aunque sea parte de otro.
        assert_eq!(find_startup(&items, "spotify").unwrap().name, "Spotify");
        assert_eq!(find_startup(&items, "onedrive").unwrap().name, "OneDrive");
        assert_eq!(find_startup(&items, "helper").unwrap().name, "SpotifyHelper");
        assert!(find_startup(&items, "spot").is_err());
        assert!(find_startup(&items, "steam").is_err());
    }
}
