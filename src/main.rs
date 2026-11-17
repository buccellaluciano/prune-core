//! P-Rune Core: P-Rune desde la consola, en un solo exe. Es otro producto sobre el mismo
//! motor (`prune-engine`) que la app de escritorio: mismo catálogo, mismo plan antes de
//! aplicar y, si las dos están en la misma PC, mismo historial.

// `t!("clave")`: mensaje traducido del motor (ver `prune_engine::i18n`).
#[macro_use]
extern crate prune_engine;

/// Texto de la CLI con valores: `tf!("Quedan {n}", n = 3)`. El texto es la clave en
/// `locales/<idioma>.json` (ver `ui`).
macro_rules! tf {
    ($es:literal, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::ui::fmt($es, &[$((stringify!($name), &$val as &dyn ::std::fmt::Display)),+])
    };
}

mod apps;
mod bloat;
mod cleanup;
mod disk;
mod dns;
mod envpath;
mod journal;
mod profile;
mod system_cmds;
mod tui;
mod tweaks;
mod ui;

use clap::{ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use prune_engine::engine::Engine;
use prune_engine::system::SystemInfo;
use prune_engine::{i18n, paths, system};
use std::io::IsTerminal;
use std::path::PathBuf;
use ui::tr;

#[derive(Parser)]
#[command(
    name = "prune",
    bin_name = "prune",
    version,
    about = "P-Rune Core: debloater y ajustes para Windows 11, desde la consola.
Sin comando abre la interfaz interactiva.",
    after_help = "Ejemplos:\n  prune list telemetria           busca ajustes\n  prune apply taskbar.align_left  muestra el plan y pregunta antes de tocar nada\n  prune undo                      deshace lo último",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Args)]
struct Global {
    /// Salida en JSON, para scripts
    #[arg(long, global = true)]
    json: bool,
    /// No pregunta antes de aplicar
    #[arg(short = 'y', long, global = true)]
    yes: bool,
    /// Idioma: es, en, pt-BR, fr, de, it, zh-CN, ja o ru (por defecto, el de Windows; también con PRUNE_LANG)
    #[arg(long, global = true, value_name = "IDIOMA", env = "PRUNE_LANG", hide_env = true)]
    lang: Option<String>,
    /// Muestra esta ayuda
    #[arg(short = 'h', long, global = true, action = ArgAction::Help)]
    help: Option<bool>,
    /// Muestra la versión
    #[arg(short = 'V', long, action = ArgAction::Version)]
    version: Option<bool>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Resumen del sistema y de lo que está aplicado
    Status,
    /// Lista los ajustes con su estado real
    List {
        /// Texto a buscar en el id, el título, la descripción o las etiquetas
        buscar: Option<String>,
        /// Solo esta categoría (id o parte del nombre)
        #[arg(short, long, value_name = "CATEGORÍA")]
        category: Option<String>,
        /// Solo los que están aplicados
        #[arg(short, long)]
        applied: bool,
    },
    /// Detalle de un ajuste: qué valores del registro toca
    Show {
        /// Id del ajuste (alcanza con una parte, si no es ambigua)
        id: String,
    },
    /// Aplica ajustes: muestra el plan y pide confirmación
    Apply {
        /// Ids de los ajustes; los que tienen opciones van como id=opción
        #[arg(required = true, value_name = "AJUSTE")]
        ajustes: Vec<String>,
        /// Solo muestra el plan, no cambia nada
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Vuelve ajustes a como los trae Windows
    Revert {
        /// Ids de los ajustes
        #[arg(required = true, value_name = "AJUSTE")]
        ajustes: Vec<String>,
        /// Solo muestra el plan, no cambia nada
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Historial de cambios (el mismo que ve la app de escritorio)
    History {
        /// Cuántas entradas mostrar
        #[arg(short = 'n', long, default_value_t = 15, value_name = "N")]
        limit: usize,
    },
    /// Deshace una entrada del historial (por defecto, la última)
    Undo {
        /// Número de la entrada según `prune history`, o su id
        entrada: Option<u64>,
        /// Deshace aunque esos valores hayan cambiado después
        #[arg(long)]
        force: bool,
    },
    /// Ajustes que aplicaste y que Windows u otro programa volvió a cambiar
    Drift,
    /// Apps preinstaladas que se pueden quitar
    Bloat {
        #[command(subcommand)]
        cmd: BloatCmd,
    },
    /// Guardar la configuración en un archivo o traerla de otra PC
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// Programas: instalar, actualizar y desinstalar con winget
    Apps {
        #[command(subcommand)]
        cmd: Option<apps::AppsCmd>,
    },
    /// Lo que arranca con Windows
    Startup {
        #[command(subcommand)]
        cmd: Option<system_cmds::StartupCmd>,
    },
    /// Limpieza de temporales y cachés
    Cleanup {
        #[command(subcommand)]
        cmd: Option<cleanup::CleanupCmd>,
    },
    /// Qué ocupa lugar en el disco
    Disk {
        #[command(subcommand)]
        cmd: Option<disk::DiskCmd>,
    },
    /// DNS de cada conexión, con proveedores y cifrado (DoH)
    Dns {
        #[command(subcommand)]
        cmd: Option<dns::DnsCmd>,
    },
    /// Proxy de tu usuario
    Proxy {
        #[command(subcommand)]
        cmd: Option<system_cmds::ProxyCmd>,
    },
    /// Windows Update: pausar y horas activas
    Wu {
        #[command(subcommand)]
        cmd: Option<system_cmds::WuCmd>,
    },
    /// Puntos de restauración del sistema
    Restore {
        #[command(subcommand)]
        cmd: Option<system_cmds::RestoreCmd>,
    },
    /// Agrega o quita `prune` del PATH, para escribirlo desde cualquier carpeta
    Path {
        #[command(subcommand)]
        cmd: PathCmd,
    },
}

#[derive(Subcommand)]
enum BloatCmd {
    /// Apps del catálogo que esta PC tiene instaladas
    List,
    /// Quita apps (no se puede deshacer desde P-Rune)
    Remove {
        /// Ids de las apps según `prune bloat list`
        #[arg(required = true, value_name = "APP")]
        apps: Vec<String>,
        /// Además evita que se instalen en las cuentas nuevas (requiere administrador)
        #[arg(long)]
        deprovision: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// Escribe en un archivo los ajustes aplicados, las apps quitadas y los programas
    Export {
        #[arg(value_name = "ARCHIVO")]
        archivo: PathBuf,
    },
    /// Aplica los ajustes de un archivo exportado, después de mostrar el plan
    Import {
        #[arg(value_name = "ARCHIVO")]
        archivo: PathBuf,
        /// Solo muestra qué haría
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// También quita las apps de Windows que el archivo indica
        #[arg(long)]
        bloat: bool,
    },
}

#[derive(Subcommand)]
pub enum PathCmd {
    /// Agrega la carpeta de prune al PATH (del equipo si está instalado y hay administrador; si no, del usuario)
    Add,
    /// La quita del PATH
    Remove,
    /// Dice si está en el PATH
    Status,
}

/// Lo que comparten todos los comandos.
pub struct Ctx {
    pub json: bool,
    pub yes: bool,
    pub info: SystemInfo,
    pub engine: Engine,
    /// Como administrador, una carpeta de Program Files; sin administrador, `%APPDATA%`. Son
    /// dos historiales distintos y nunca se mezclan (ver `paths::journal_dir`).
    pub journal_dir: PathBuf,
}

impl Ctx {
    fn new(global: &Global) -> Result<Self, String> {
        let info = system::info();
        let journal_dir = paths::journal_dir(info.elevated)?;
        Ok(Self { json: global.json, yes: global.yes, engine: Engine::new(journal_dir.clone()), journal_dir, info })
    }

    /// En JSON no hay a quién preguntarle: los comandos que cambian algo piden `--yes`.
    pub fn need_yes_in_json(&self) -> Result<(), String> {
        if self.json && !self.yes {
            return Err(tr("Con --json no se pregunta: agregá --yes.").to_string());
        }
        Ok(())
    }
}

/// Idioma de Windows para esta persona, entre los que tiene P-Rune.
fn system_lang() -> &'static str {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) } as usize;
    let locale = String::from_utf16_lossy(&buf[..n.saturating_sub(1)]).to_ascii_lowercase();
    let prefix = locale.split(['-', '_']).next().unwrap_or_default();
    match prefix {
        "es" => "es",
        "pt" => "pt-BR",
        "fr" => "fr",
        "de" => "de",
        "it" => "it",
        "zh" => "zh-CN",
        "ja" => "ja",
        "ru" => "ru",
        _ => "en",
    }
}

fn set_language(asked: Option<&str>) -> Result<(), String> {
    let lang = match asked {
        // Se acepta sin importar mayúsculas: "pt-br", "ZH-cn".
        Some(a) => i18n::LANGS.iter().copied().find(|l| l.eq_ignore_ascii_case(a)).ok_or_else(|| {
            format!("{}: {a} ({})", tr("Idioma no disponible"), i18n::LANGS.join(", "))
        })?,
        None => system_lang(),
    };
    ui::set_lang(lang);
    i18n::set(lang)
}

/// El idioma pedido con `--lang` o `PRUNE_LANG`. Se mira antes de que clap lea los
/// argumentos, porque la ayuda (que arma clap) también tiene que salir en ese idioma.
fn asked_lang(args: &[String]) -> Option<String> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--lang" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--lang=") {
            return Some(v.to_string());
        }
    }
    std::env::var("PRUNE_LANG").ok().filter(|v| !v.is_empty())
}

/// La ayuda la arma clap con los textos en español del código: acá se pasan al idioma
/// elegido (descripciones, títulos y nombres de los valores) para todos los comandos.
fn localize(cmd: clap::Command) -> clap::Command {
    let subs: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
    let about = cmd.get_about().map(|t| ui::tr_dyn(&t.to_string()));
    let after = cmd.get_after_help().map(|t| ui::tr_dyn(&t.to_string()));
    let mut cmd = cmd
        .help_template(format!("{{about-with-newline}}\n{} {{usage}}\n\n{{all-args}}{{after-help}}", tr("Uso:")))
        .subcommand_help_heading(tr("Comandos"))
        .subcommand_value_name(tr("COMANDO"))
        .mut_args(|a| {
            let help = a.get_help().map(|t| ui::tr_dyn(&t.to_string()));
            let names: Option<Vec<String>> = a.get_value_names().filter(|_| a.get_action().takes_values()).map(|v| v.iter().map(|n| ui::tr_dyn(n)).collect());
            let mut a = if a.is_positional() { a.help_heading(tr("Argumentos")) } else { a.help_heading(tr("Opciones")) };
            if let Some(h) = help {
                a = a.help(h);
            }
            // Solo los nombres escritos a mano (en mayúsculas): los que clap saca del nombre
            // del campo quedan como están.
            match names {
                Some(n) if n.iter().all(|x| !x.chars().any(char::is_lowercase)) => a.value_names(n),
                _ => a,
            }
        });
    if let Some(t) = about {
        cmd = cmd.about(t);
    }
    if let Some(t) = after {
        cmd = cmd.after_help(t);
    }
    for name in subs {
        cmd = cmd.mut_subcommand(name, localize);
    }
    cmd
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    set_language(asked_lang(&args).as_deref())?;
    let matches = localize(Cli::command()).get_matches();
    let cli = Cli::from_arg_matches(&matches).map_err(|e| e.to_string())?;
    // No necesita el historial ni el catálogo: mejor que no cree ni toque nada más (con
    // `path remove`, la idea es justamente no dejar rastro).
    if let Some(Cmd::Path { cmd }) = &cli.cmd {
        return envpath::run(cmd, cli.global.json);
    }
    let ctx = Ctx::new(&cli.global)?;
    match cli.cmd {
        // Sin argumentos y con alguien mirando: la interfaz interactiva. Si la salida va a
        // un archivo o a otro programa (o se pidió JSON), el resumen.
        None if !ctx.json && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => tui::run(&ctx),
        None => {
            tweaks::status(&ctx)?;
            if !ctx.json {
                anstream::println!("\n{}", ui::dim(tr("`prune --help` muestra los comandos.")));
            }
            Ok(())
        }
        Some(Cmd::Status) => tweaks::status(&ctx),
        Some(Cmd::List { buscar, category, applied }) => tweaks::list(&ctx, buscar.as_deref(), category.as_deref(), applied),
        Some(Cmd::Show { id }) => tweaks::show(&ctx, &id),
        Some(Cmd::Apply { ajustes, dry_run }) => tweaks::apply(&ctx, &ajustes, true, dry_run),
        Some(Cmd::Revert { ajustes, dry_run }) => tweaks::apply(&ctx, &ajustes, false, dry_run),
        Some(Cmd::History { limit }) => journal::history(&ctx, limit),
        Some(Cmd::Undo { entrada, force }) => journal::undo(&ctx, entrada, force),
        Some(Cmd::Drift) => journal::drift(&ctx),
        Some(Cmd::Bloat { cmd: BloatCmd::List }) => bloat::list(&ctx),
        Some(Cmd::Bloat { cmd: BloatCmd::Remove { apps, deprovision } }) => bloat::remove(&ctx, &apps, deprovision),
        Some(Cmd::Profile { cmd: ProfileCmd::Export { archivo } }) => profile::export(&ctx, &archivo),
        Some(Cmd::Profile { cmd: ProfileCmd::Import { archivo, dry_run, bloat } }) => profile::import(&ctx, &archivo, dry_run, bloat),
        Some(Cmd::Apps { cmd }) => apps::run(&ctx, cmd),
        Some(Cmd::Startup { cmd }) => system_cmds::startup(&ctx, cmd),
        Some(Cmd::Cleanup { cmd }) => cleanup::run(&ctx, cmd),
        Some(Cmd::Disk { cmd }) => disk::run(&ctx, cmd),
        Some(Cmd::Dns { cmd }) => dns::run(&ctx, cmd),
        Some(Cmd::Proxy { cmd }) => system_cmds::proxy(&ctx, cmd),
        Some(Cmd::Wu { cmd }) => system_cmds::wu(&ctx, cmd),
        Some(Cmd::Restore { cmd }) => system_cmds::restore(&ctx, cmd),
        Some(Cmd::Path { .. }) => unreachable!("se resuelve antes de armar el contexto"),
    }
}

fn main() {
    // Como en la app de escritorio: las DLLs que se carguen de acá en más solo se buscan en
    // System32, nunca en la carpeta del exe ni en el directorio actual. Las que Windows carga
    // al abrir el exe las cubre /DEPENDENTLOADFLAG (build.rs).
    unsafe {
        use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }
    if let Err(e) = run() {
        anstream::eprintln!("{} {e}", ui::red("error:"));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod locale_check;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idioma_pedido_antes_de_leer_los_argumentos() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(asked_lang(&args(&["prune", "list", "--lang", "fr"])).as_deref(), Some("fr"));
        assert_eq!(asked_lang(&args(&["prune", "--lang=ja", "status"])).as_deref(), Some("ja"));
    }

    #[test]
    fn la_definicion_de_comandos_es_valida() {
        localize(Cli::command()).debug_assert();
    }

    #[test]
    fn idioma_del_sistema_es_uno_de_los_disponibles() {
        assert!(i18n::LANGS.contains(&system_lang()));
    }
}
