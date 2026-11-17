//! DNS: ver qué usa cada conexión y cambiarlo a un proveedor, con la misma red de seguridad
//! que la app de escritorio.
//!
//! Antes de tocar nada se prueban los servidores. Después de cambiar hay que confirmar que
//! internet sigue andando: sin confirmación en `CONFIRM_SECS` segundos, vuelve solo al DNS
//! anterior. El cambio queda en el historial y se deshace con `prune undo`.

use crate::Ctx;
use crate::ui::{self, tr};
use clap::{Subcommand, ValueEnum};
use prune_engine::network::{self, AdapterDns, AdapterView, DnsConfig, DohMode, FamilyView, TestResult};
use serde::Serialize;
use std::io::IsTerminal;
use std::time::Duration;

#[derive(Subcommand)]
pub enum DnsCmd {
    /// Qué DNS usa cada conexión y qué proveedores hay
    Status,
    /// Cambia el DNS a un proveedor, o a servidores escritos a mano
    Set {
        /// Proveedor: cloudflare, cloudflare_malware, google, quad9 o adguard
        #[arg(value_name = "PROVEEDOR", required_unless_present_any = ["servers", "servers6"])]
        proveedor: Option<String>,
        /// Servidores IPv4 a mano, separados por coma (en vez de un proveedor)
        #[arg(long, value_delimiter = ',', value_name = "IP", conflicts_with = "proveedor")]
        servers: Vec<String>,
        /// Servidores IPv6 a mano, separados por coma
        #[arg(long, value_delimiter = ',', value_name = "IP", conflicts_with = "proveedor")]
        servers6: Vec<String>,
        /// Conexión (alcanza con parte del nombre); por defecto, la que sale a internet
        #[arg(short, long, value_name = "CONEXIÓN")]
        adapter: Option<String>,
        /// DNS cifrado (DoH): preferred lo usa si se puede, required solo cifrado, off sin cifrar
        #[arg(long, value_enum, default_value = "preferred", value_name = "MODO")]
        doh: Doh,
    },
    /// Vuelve al DNS automático (el que da el router)
    Reset {
        #[arg(short, long, value_name = "CONEXIÓN")]
        adapter: Option<String>,
    },
    /// Prueba si un proveedor responde, sin cambiar nada
    Test {
        #[arg(value_name = "PROVEEDOR")]
        proveedor: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Doh {
    Preferred,
    Required,
    Off,
}

impl From<Doh> for DohMode {
    fn from(d: Doh) -> Self {
        match d {
            Doh::Preferred => DohMode::Preferred,
            Doh::Required => DohMode::Required,
            Doh::Off => DohMode::Off,
        }
    }
}

pub fn run(ctx: &Ctx, cmd: Option<DnsCmd>) -> Result<(), String> {
    match cmd.unwrap_or(DnsCmd::Status) {
        DnsCmd::Status => status(ctx),
        DnsCmd::Test { proveedor } => {
            let adapters = network::adapters();
            let on = pick_adapters(&adapters, None)?;
            let target = network::validate_request(&request(&on, &servers_of(&proveedor)?, DohMode::Preferred), &adapters)?;
            let results = probe(ctx, &target);
            if ctx.json {
                return ui::json(&results);
            }
            if results.iter().any(|r| r.ok) { Ok(()) } else { Err(tr("Ningún servidor respondió.").to_string()) }
        }
        DnsCmd::Set { proveedor, servers, servers6, adapter, doh } => {
            let (wanted, doh, name) = match proveedor {
                Some(p) => (servers_of(&p)?, doh.into(), provider(&p)?.name.to_string()),
                // A mano no hay forma de saber si el servidor admite DoH: sin cifrar, como en la app.
                None => (Servers { v4: servers.clone(), v6: servers6.clone() }, DohMode::Off, servers.iter().chain(&servers6).cloned().collect::<Vec<_>>().join(", ")),
            };
            change(ctx, adapter.as_deref(), Some(wanted), doh, &name)
        }
        DnsCmd::Reset { adapter } => change(ctx, adapter.as_deref(), None, DohMode::Off, tr("automático")),
    }
}

// ───────────────────────── Armar el pedido ─────────────────────────

struct Servers {
    v4: Vec<String>,
    v6: Vec<String>,
}

fn provider(id: &str) -> Result<&'static network::Provider, String> {
    network::PROVIDERS.iter().find(|p| p.id.eq_ignore_ascii_case(id) || p.name.eq_ignore_ascii_case(id)).ok_or_else(|| {
        let ids: Vec<&str> = network::PROVIDERS.iter().map(|p| p.id).collect();
        tf!("No existe el proveedor {id}. Hay: {list}.", id = id, list = ids.join(", "))
    })
}

fn servers_of(id: &str) -> Result<Servers, String> {
    let p = provider(id)?;
    Ok(Servers { v4: p.v4.iter().map(|s| s.to_string()).collect(), v6: p.v6.iter().map(|s| s.to_string()).collect() })
}

/// Las conexiones a cambiar: la pedida por nombre, o la que sale a internet.
fn pick_adapters<'a>(adapters: &'a [AdapterView], query: Option<&str>) -> Result<Vec<&'a AdapterView>, String> {
    let names = || adapters.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    let Some(q) = query else {
        let primary: Vec<&AdapterView> = adapters.iter().filter(|a| a.primary).collect();
        return if primary.is_empty() { Err(tf!("No se encontró la conexión que sale a internet. Elegí una con --adapter: {names}", names = names())) } else { Ok(primary) };
    };
    let ql = ui::fold(q);
    let exact: Vec<&AdapterView> = adapters.iter().filter(|a| a.name.to_lowercase() == ql).collect();
    let hits = if exact.is_empty() { adapters.iter().filter(|a| ui::fold(&a.name).contains(&ql) || ui::fold(&a.description).contains(&ql)).collect() } else { exact };
    match hits.len() {
        1 => Ok(hits),
        0 => Err(tf!("No hay ninguna conexión llamada {query}. Hay: {names}", query = q, names = names())),
        _ => Err(tf!("`{query}` coincide con varias conexiones: {names}", query = q, names = hits.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// Lo que se le pide al motor para cada conexión. IPv6 solo se cambia donde ya se usa: en
/// una red sin IPv6, ponerle servidores no sirve y las pruebas darían siempre error.
fn request(on: &[&AdapterView], wanted: &Servers, doh: DohMode) -> Vec<AdapterDns> {
    on.iter()
        .map(|a| {
            let uses_v6 = !a.v6.effective.is_empty() || !a.v6.config.servers.is_empty();
            AdapterDns {
                guid: a.guid.clone(),
                name: a.name.clone(),
                v4: DnsConfig { servers: wanted.v4.clone(), doh: if wanted.v4.is_empty() { DohMode::Off } else { doh } },
                v6: if uses_v6 { DnsConfig { servers: wanted.v6.clone(), doh: if wanted.v6.is_empty() { DohMode::Off } else { doh } } } else { a.v6.config.clone() },
            }
        })
        .collect()
}

// ───────────────────────── Probar ─────────────────────────

fn probe(ctx: &Ctx, target: &[AdapterDns]) -> Vec<TestResult> {
    if !ctx.json {
        anstream::eprintln!("{}", ui::dim(tr("Probando los servidores…")));
    }
    let results = network::test(target);
    if !ctx.json {
        for r in &results {
            let kind = if r.doh { ui::dim(tr("  cifrado (DoH)")) } else { String::new() };
            match (r.ok, r.ms) {
                (true, Some(ms)) => anstream::println!("  {} {}  {}{kind}", ui::green("✓"), ui::pad(&r.server, 24), ui::dim(&format!("{ms} ms"))),
                (true, None) => anstream::println!("  {} {}{kind}", ui::green("✓"), ui::pad(&r.server, 24)),
                (false, _) => anstream::println!("  {} {}  {}{kind}", ui::red("✗"), ui::pad(&r.server, 24), ui::dim(tr("no responde"))),
            }
        }
    }
    results
}

// ───────────────────────── Cambiar ─────────────────────────

#[derive(Serialize)]
struct Outcome {
    entry_id: u64,
    kept: bool,
    tests: Vec<TestResult>,
}

/// Pregunta con límite de tiempo: sin respuesta, es que no.
fn ask_within(question: &str, secs: u64) -> bool {
    use std::io::Write;
    anstream::eprint!("{question} {} ", ui::dim(tr("[s/N]")));
    let _ = std::io::stderr().flush();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(line) => ui::is_yes(&line),
        Err(_) => {
            anstream::eprintln!();
            false
        }
    }
}

fn change(ctx: &Ctx, adapter: Option<&str>, wanted: Option<Servers>, doh: DohMode, name: &str) -> Result<(), String> {
    // Admin, dominio, directiva o un cambio anterior sin confirmar: se dice antes de probar.
    network::guard()?;
    ctx.need_yes_in_json()?;
    let adapters = network::adapters();
    let on = pick_adapters(&adapters, adapter)?;
    let reset = wanted.is_none();
    let wanted = wanted.unwrap_or(Servers { v4: vec![], v6: vec![] });
    let target = network::validate_request(&request(&on, &wanted, doh), &adapters)?;

    let tests = if reset { Vec::new() } else { probe(ctx, &target) };
    if !reset && !tests.iter().any(|r| r.ok) {
        return Err(tr("Ningún servidor respondió: no se cambió nada.").to_string());
    }
    let where_ = on.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    if !ui::confirm(&tf!("¿Cambiar el DNS de {adapter} a {dns}?", adapter = where_, dns = name), ctx.yes)? {
        anstream::println!("{}", tr("No se cambió nada."));
        return Ok(());
    }

    let applied = {
        // Dos pedidos a la vez no pueden pasar los dos el control de "hay uno pendiente".
        let _lock = network::APPLY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        network::apply(&target)?
    };
    let entry_id = match ctx.engine.record_network(applied.clone()) {
        Ok(id) => id,
        Err(e) => {
            // Sin historial no hay cómo deshacer: mejor no dejar el cambio.
            let _ = network::restore(&applied.before);
            return Err(t!("journal_save_failed", error = e));
        }
    };
    network::arm(entry_id);

    // Hay que confirmar que internet sigue andando. Con --yes (o sin terminal) no hay a quién
    // preguntarle: se vuelven a probar los servidores con el DNS ya cambiado.
    let kept = if ctx.yes || !std::io::stdin().is_terminal() {
        reset || network::test(&target).iter().any(|r| r.ok)
    } else {
        ask_within(&tf!("DNS cambiado. ¿Internet sigue andando? Sin respuesta en {n} segundos, vuelve al anterior.", n = network::CONFIRM_SECS), network::CONFIRM_SECS)
    };

    if kept {
        network::take(entry_id);
        if ctx.json {
            return ui::json(&Outcome { entry_id, kept, tests });
        }
        anstream::println!("{} {}", ui::green("✓"), network::label(&applied));
        anstream::println!("{} {}", ui::dim(tr("Para deshacerlo:")), ui::cyan("prune undo"));
        return Ok(());
    }
    // Si ya no está pendiente es que otra cosa lo resolvió: no hay nada más que hacer.
    if network::take(entry_id) {
        let report = ctx.engine.undo(entry_id, true, None)?;
        if let Some(f) = report.results.iter().find(|r| !r.ok) {
            return Err(tf!("No se pudo volver al DNS anterior: {error}. Probá `prune undo`.", error = f.error.clone().unwrap_or_default()));
        }
    }
    if ctx.json {
        ui::json(&Outcome { entry_id, kept, tests })?;
    }
    Err(tr("Sin confirmación: se volvió al DNS anterior.").to_string())
}

// ───────────────────────── Estado ─────────────────────────

fn family_text(f: &FamilyView) -> String {
    if f.config.servers.is_empty() {
        let effective = if f.effective.is_empty() { String::new() } else { format!(" ({})", f.effective.join(", ")) };
        return ui::dim(&format!("{}{effective}", tr("automático")));
    }
    let mut out = f.config.servers.join(", ");
    if let Some(p) = f.provider.and_then(|id| network::PROVIDERS.iter().find(|p| p.id == id)) {
        out.push_str(&format!("  {}", ui::cyan(p.name)));
    }
    match f.config.doh {
        DohMode::Off => {}
        DohMode::Preferred => out.push_str(&format!("  {}", ui::green(tr("cifrado")))),
        DohMode::Required => out.push_str(&format!("  {}", ui::green(tr("solo cifrado")))),
    }
    out
}

fn status(ctx: &Ctx) -> Result<(), String> {
    let s = network::status();
    if ctx.json {
        return ui::json(&s);
    }
    for a in &s.adapters {
        let primary = if a.primary { ui::green(tr("  sale a internet")) } else { String::new() };
        anstream::println!("{}  {}{primary}", ui::bold(&a.name), ui::dim(&a.description));
        anstream::println!("  IPv4  {}", family_text(&a.v4));
        anstream::println!("  IPv6  {}", family_text(&a.v6));
    }
    if s.adapters.is_empty() {
        anstream::println!("{}", tr("No hay conexiones de red activas."));
    }
    anstream::println!("\n{}", ui::bold(tr("Proveedores")));
    for p in &s.providers {
        anstream::println!("  {}  {}  {}", ui::cyan(&ui::pad(p.id, 20)), ui::pad(p.name, 22), ui::dim(&p.v4.join(", ")));
    }
    if let Some(d) = &s.domain {
        anstream::println!("{} {}", ui::yellow("●"), tf!("Esta PC está en el dominio {domain}: el DNS lo maneja la organización.", domain = d));
    }
    if s.policy {
        anstream::println!("{} {}", ui::yellow("●"), tr("Una directiva de la organización fija el DNS: puede que no se pueda cambiar."));
    }
    if s.vpn {
        anstream::println!("{} {}", ui::yellow("●"), tr("Hay una VPN activa: mientras esté conectada puede usar su propio DNS."));
    }
    anstream::println!("\n{} {}", ui::dim(tr("Para cambiarlo (pide administrador):")), ui::cyan("prune dns set cloudflare"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, primary: bool, v6: &[&str]) -> AdapterView {
        let family = |servers: &[&str]| FamilyView { config: DnsConfig { servers: vec![], doh: DohMode::Off }, effective: servers.iter().map(|s| s.to_string()).collect(), provider: None };
        AdapterView { guid: format!("{{{name}}}"), name: name.into(), description: format!("Adaptador {name}"), kind: "ethernet", primary, v4: family(&["192.168.0.1"]), v6: family(v6) }
    }

    #[test]
    fn elige_la_conexion_que_sale_a_internet() {
        let adapters = vec![adapter("Ethernet", false, &[]), adapter("Wi-Fi", true, &[]), adapter("Wi-Fi 2", false, &[])];
        assert_eq!(pick_adapters(&adapters, None).unwrap()[0].name, "Wi-Fi");
        // Por nombre: el exacto gana aunque sea parte de otro.
        assert_eq!(pick_adapters(&adapters, Some("wi-fi")).unwrap()[0].name, "Wi-Fi");
        assert_eq!(pick_adapters(&adapters, Some("ether")).unwrap()[0].name, "Ethernet");
        assert!(pick_adapters(&adapters, Some("Wi")).is_err());
        assert!(pick_adapters(&adapters, Some("vpn")).is_err());
        // Sin ninguna principal no se adivina.
        assert!(pick_adapters(&adapters[..1], None).is_err());
    }

    #[test]
    fn proveedores_por_id_o_nombre() {
        assert_eq!(provider("CLOUDFLARE").unwrap().id, "cloudflare");
        assert_eq!(provider("Quad9").unwrap().id, "quad9");
        assert!(provider("opendns").err().unwrap().contains("cloudflare"));
    }

    #[test]
    fn ipv6_solo_donde_ya_se_usa() {
        let with = adapter("Con", true, &["fe80::1"]);
        let without = adapter("Sin", true, &[]);
        let wanted = servers_of("cloudflare").unwrap();
        let req = request(&[&with, &without], &wanted, DohMode::Preferred);
        assert_eq!(req[0].v4.servers, wanted.v4);
        assert_eq!(req[0].v6.servers, wanted.v6);
        assert!(matches!(req[0].v6.doh, DohMode::Preferred));
        // Sin IPv6 en uso, esa parte queda como estaba.
        assert_eq!(req[1].v4.servers, wanted.v4);
        assert!(req[1].v6.servers.is_empty());
        // Volver a automático: sin servidores y sin cifrado.
        let auto = request(&[&with], &Servers { v4: vec![], v6: vec![] }, DohMode::Preferred);
        assert!(auto[0].v4.servers.is_empty() && matches!(auto[0].v4.doh, DohMode::Off));
    }
}
