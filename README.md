<p align="center">
  <img src="assets/prune.svg" width="96" alt="P-Rune Core" />
</p>

<h1 align="center">P-Rune Core</h1>

<p align="center">
  Debloater y ajustes para <b>Windows 11</b> desde la consola.<br />
  Un solo exe de 2 MB, sin ventana y sin nada corriendo de fondo. En 9 idiomas.
</p>

P-Rune Core es [P-Rune](https://github.com/buccellaluciano/prune) para quien vive en la
terminal: scripts, sesiones remotas, máquinas recién instaladas. Usa el mismo motor que la app
de escritorio, así que tiene el mismo catálogo, muestra el plan antes de aplicar y deshace
cada cambio con el valor exacto que había.

```
prune                            interfaz interactiva en la propia consola
prune list telemetria            busca ajustes
prune apply taskbar.align_left   muestra el plan y pregunta antes de tocar nada
prune undo                       deshace lo último
```

> [!WARNING]
> P-Rune Core modifica el registro de Windows, quita apps preinstaladas e instala programas.
> Revisá el plan antes de aplicar cada cambio. Se distribuye **sin garantía** (ver
> [LICENSE](LICENSE)): usalo bajo tu responsabilidad. Quitar apps no se puede deshacer desde
> P-Rune Core; se reinstalan desde Microsoft Store.

## Tenerlo

Hay dos formas, las dos en [Releases](https://github.com/buccellaluciano/prune-core/releases):

**Instalador** (`P-Rune-Core_<versión>_x64.msi`). Lo copia a `Program Files\P-Rune Core` y lo
deja en el PATH del equipo: abrís una terminal nueva y escribís `prune`. No tiene asistente:
doble clic, Windows pide permiso de administrador y listo.

```powershell
msiexec /i P-Rune-Core_0.1.1_x64.msi /qn              # sin ventanas, para desplegarlo
msiexec /i P-Rune-Core_0.1.1_x64.msi ADDTOPATH=0      # sin tocar el PATH
```

Se quita desde "Aplicaciones instaladas", y con eso sale también del PATH.

**Sin instalar** (`P-Rune-Core_<versión>_x64.zip`). Descomprimilo donde quieras y usalo ahí
mismo. Para escribir `prune` desde cualquier carpeta:

```powershell
.\prune.exe path add     # agrega esa carpeta al PATH de tu usuario; abrí una terminal nueva
prune
```

`prune path remove` lo saca del PATH y `prune path status` dice si está. Como el PATH apunta a
la carpeta donde esté el exe, dejalo en un lugar fijo antes de agregarlo.

- Las dos funcionan igual: lo único que cambia es dónde queda el exe y a qué PATH va.
- El SHA-256 de cada archivo está en las notas del release, para verificar la descarga.
- Ninguno de los dos está firmado con un certificado de código: Windows SmartScreen puede
  mostrar "editor desconocido".

## Usarlo

```bash
prune                              # interfaz interactiva: marcar, ver el plan, aplicar
prune status                       # resumen: sistema, qué está aplicado, qué cambió
prune list telemetria              # busca ajustes por texto (-c para una categoría)
prune show search.taskbar_mode     # qué valores del registro toca un ajuste
prune apply taskbar.align_left     # muestra el plan y pregunta antes de tocar nada
prune apply search.taskbar_mode=icon   # los ajustes con opciones van como id=opción
prune revert taskbar.align_left    # vuelve a como lo trae Windows
prune history                      # historial; `prune undo` deshace lo último
prune drift                        # lo que aplicaste y Windows volvió a cambiar
prune bloat list                   # apps preinstaladas; `prune bloat remove <app>`
prune profile export mi-pc.json    # y `prune profile import mi-pc.json` en otra PC

prune apps list                    # programas del catálogo; `prune apps install firefox`
prune startup                      # lo que arranca con Windows; `prune startup disable steam`
prune cleanup                      # cuánto se puede liberar; `prune cleanup run --all`
prune disk scan C                  # qué carpetas ocupan más
prune dns                          # DNS de cada conexión; `prune dns set cloudflare`
prune proxy                        # proxy de tu usuario; `prune proxy auto off`
prune wu                           # Windows Update; `prune wu pause 7`, `prune wu hours 9 23`
prune restore create               # punto de restauración
```

`prune --help` y `prune <comando> --help` muestran todo, en tu idioma.

- **Rápido de escribir:** alcanza con una parte del id si no es ambigua (`prune apply
  align_left`), y la búsqueda no distingue mayúsculas ni acentos.
- **Para scripts:** `--json` da la salida en JSON, `-y` no pregunta y `-n` / `--dry-run`
  muestra el plan sin cambiar nada. Si algo falla, sale con código distinto de cero.
- **Interactivo:** sin comando abre una interfaz en la misma terminal. Flechas para moverse,
  espacio para marcar, `/` para buscar, Enter para ver el plan y aplicar, `u` para deshacer lo
  último, `?` para las teclas. Con `1`, `2` y `3` se pasa entre ajustes, apps de Windows e
  historial.
- **Administrador:** no lo pide al abrir. Mirar no lo necesita; los ajustes marcados `admin`
  sí: abrí una terminal de administrador o usá `sudo prune ...`. Si falta, avisa antes de
  cambiar nada.
- **Todo queda anotado:** Windows Update, inicio de Windows, proxy y DNS son comandos
  directos (se hacen al momento), pero quedan en el historial como los ajustes y se deshacen
  con `prune undo`. Limpiar archivos y quitar apps no se deshace: por eso piden confirmación.
- **DNS con red de seguridad:** primero prueba los servidores y, después de cambiar, hay que
  confirmar que internet sigue andando; sin confirmación en 15 segundos, vuelve solo al DNS
  anterior.
- **Idiomas:** español, inglés, portugués, francés, alemán, italiano, chino, japonés y ruso,
  ayuda incluida (`--lang` o `PRUNE_LANG`; por defecto, el de Windows).

## El historial

Hay dos historiales, y cada uno se lee y se escribe solo con los permisos con los que se creó:

- **En una terminal de administrador**, el historial vive en Program Files, donde solo
  escriben los administradores: `Program Files\P-Rune Core\data`.
- **Sin administrador**, en `%APPDATA%\com.debloater.app`. Ahí quedan los ajustes de tu
  usuario que aplicaste sin elevar.

Lo que aplicaste como administrador se deshace desde una terminal de administrador, y lo que
aplicaste sin serlo, desde una común. `prune history` sin administrador avisa si además hay
un historial de administrador.

Están separados a propósito. Deshacer es volver a escribir lo que el historial dice, y en
`%APPDATA%` escribe cualquier programa tuyo: si Core lo leyera como administrador, un
programa sin permisos podría dejarle anotado qué escribir.

### Con P-Rune en la misma PC

Si la app de escritorio está instalada (en su carpeta por defecto, `Program Files\P-Rune`),
Core usa su historial cuando corre como administrador: lo que aplicás con una se puede
deshacer desde la otra.

## Seguridad

- **Nada se aplica a ciegas:** el estado de cada ajuste se lee del registro real, y antes de
  cambiar algo se muestra el plan, valor por valor.
- **Deshacer exacto y a la vista:** el historial guarda el valor que había, y antes de
  deshacer se muestra qué se va a escribir, armado con el contenido de la entrada y no con su
  título. Cada entrada se compara además con el catálogo.
- **El historial de administrador no sale de Program Files:** un proceso de administrador
  nunca lee ni importa un historial guardado donde escribe cualquier programa (ver
  [El historial](#el-historial)).
- **Catálogo cerrado:** solo se tocan los ajustes, las apps y los programas del catálogo que
  viene adentro del exe; nunca un id o una ruta escritos a mano.
- **El PATH se edita con cuidado:** `prune path` agrega o quita solo su propia entrada y deja
  el resto como estaba. Al PATH de todo el equipo solo va una carpeta de Program Files.
- DLLs solo desde `System32`.

Para reportar una vulnerabilidad, ver [SECURITY.md](SECURITY.md).

## Compilarlo

Hace falta [Rust](https://rustup.rs/) en Windows.

```bash
cargo build --release          # queda en target/release/prune.exe
cargo test --workspace
node scripts/package.mjs       # además arma el MSI y el zip en target/dist, con sus SHA-256
```

- El perfil de release optimiza el tamaño en vez de la velocidad: el exe pesa un tercio menos
  y responde igual, porque lo que tarda es leer el registro.
- El MSI se arma con [WiX 3](https://wixtoolset.org/) (`candle.exe` y `light.exe` en el PATH).
- Para publicar una versión: subila en `Cargo.toml`, escribí las notas en
  `docs/releases/<versión>.md`, commiteá, pusheá y corré
  `node scripts/release.mjs --notes docs/releases/<versión>.md --publish`.

## Estructura

| Ruta | Qué hay |
|---|---|
| `src/` | La consola: comandos, interfaz interactiva e idiomas |
| `src/main.rs` | Definición de los comandos y de la ayuda |
| `src/tui.rs`, `src/tui_sections.rs` | La interfaz interactiva |
| `src/envpath.rs` | `prune path`: agregar y quitar del PATH |
| `locales/` | Traducciones (el español va en el código y es la clave) |
| `engine/` | El motor: catálogo, ajustes, historial y acceso a Windows. Sin interfaz |
| `engine/catalog/` | Ajustes, apps removibles y programas, en TOML |
| `engine/src/engine.rs` | Estado, plan, aplicar, historial y deshacer |
| `installer.wxs` | El instalador MSI: copia el exe y lo agrega al PATH |
| `scripts/package.mjs` | Compila y arma el MSI y el zip |
| `scripts/release.mjs` | Publica una versión en GitHub |

Si agregás o cambiás un texto, `cargo test` dice qué le falta a cada idioma en `locales/`.

## Origen

P-Rune Core nace de [P-Rune](https://github.com/buccellaluciano/prune), la app de escritorio.
El motor (`engine/`) es el mismo código; este repo lo lleva por su cuenta para que la versión
de consola se pueda compilar, publicar y versionar sola.

## Licencia

[MIT](LICENSE).
