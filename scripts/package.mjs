// Compila P-Rune Core y arma lo que se publica en cada release, en target/dist/:
//   P-Rune-Core_<versión>_x64.msi  instalador: lo copia a Program Files y lo deja en el PATH
//   P-Rune-Core_<versión>_x64.zip  el exe solo, con LICENSE y README; se usa donde se
//                                  descomprima, y `.\prune.exe path add` lo deja en el PATH
//
// Uso: node scripts/package.mjs
// scripts/release.mjs lo llama para publicar.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";

export const ROOT = resolve(import.meta.dirname, "..");

/// La versión del workspace (la de los dos crates), de Cargo.toml.
export function workspaceVersion() {
  const toml = readFileSync(join(ROOT, "Cargo.toml"), "utf8");
  const version = toml.match(/\[workspace\.package\][^[]*?\bversion\s*=\s*"([^"]+)"/)?.[1];
  if (!version) throw new Error("No se encontró la versión en [workspace.package] de Cargo.toml.");
  return version;
}

export const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");

/// candle.exe o light.exe de WiX 3: los que bajó Tauri en esta máquina, si los hay (el MSI de
/// P-Rune se arma con los mismos); si no, los del PATH.
function wix(tool) {
  const cached = join(process.env.LOCALAPPDATA ?? "", "tauri", "WixTools314", `${tool}.exe`);
  return existsSync(cached) ? cached : `${tool}.exe`;
}

/// Arma el MSI con installer.wxs.
function buildMsi(dist, version) {
  // Un MSI solo admite versiones de tres números: 0.6.0-beta.1 se instala como 0.6.0.
  const msiVersion = version.match(/^\d+\.\d+\.\d+/)?.[0];
  if (!msiVersion) throw new Error(`La versión ${version} no empieza con tres números: no sirve para un MSI.`);
  const obj = join(ROOT, "target/wix");
  mkdirSync(obj, { recursive: true });
  const msi = join(dist, `P-Rune-Core_${version}_x64.msi`);
  const define = (name, value) => `-d${name}=${value}`;
  try {
    execFileSync(
      wix("candle"),
      [
        "-nologo",
        "-arch",
        "x64",
        define("Version", msiVersion),
        define("Exe", join(dist, "prune.exe")),
        define("License", join(dist, "LICENSE")),
        define("Readme", join(dist, "README.md")),
        define("Icon", join(ROOT, "assets/icon.ico")),
        "-out",
        join(obj, "installer.wixobj"),
        join(ROOT, "installer.wxs"),
      ],
      { stdio: "inherit" },
    );
    // -spdb: sin el archivo de símbolos del instalador, que no se publica.
    execFileSync(wix("light"), ["-nologo", "-spdb", "-out", msi, join(obj, "installer.wixobj")], { stdio: "inherit" });
  } catch (e) {
    if (e.code === "ENOENT") throw new Error("No se encontró WiX 3 (candle.exe y light.exe): instalalo y dejalo en el PATH.");
    throw e;
  }
  return msi;
}

/// Compila y empaqueta. Devuelve las rutas del MSI y del zip.
export function buildPackage(version = workspaceVersion()) {
  // Rust guarda en el ejecutable rutas absolutas de la máquina que compila (los mensajes de
  // error de cada dependencia: C:\Users\<usuario>\.cargo\registry\...). Se reemplazan por
  // rutas neutras para no publicar el nombre de usuario ni la estructura de carpetas.
  const remap = [[homedir(), "~"], [ROOT, "."]];
  if (process.env.CARGO_HOME) remap.splice(1, 0, [process.env.CARGO_HOME, "~/.cargo"]);
  const rustflags = [process.env.CARGO_ENCODED_RUSTFLAGS, ...remap.map(([from, to]) => `--remap-path-prefix=${from}=${to}`)].filter(Boolean).join("\x1f");
  execFileSync("cargo", ["build", "--release", "--locked"], { cwd: ROOT, stdio: "inherit", env: { ...process.env, CARGO_ENCODED_RUSTFLAGS: rustflags } });
  const built = join(ROOT, "target/release/prune.exe");

  // Comprobación: el ejecutable no puede llevar la carpeta del usuario.
  const text = readFileSync(built).toString("latin1").toLowerCase();
  const leaked = [homedir(), basename(homedir())].filter((s) => s.length > 2 && text.includes(`\\${s.toLowerCase()}\\`));
  if (leaked.length) throw new Error(`prune.exe todavía contiene rutas locales (${leaked.join(", ")}). No se publica.`);
  // Y dice su versión: si no es la que se publica, alguien se olvidó de subirla en Cargo.toml.
  const said = execFileSync(built, ["--version"], { encoding: "utf8" }).trim();
  if (!said.endsWith(` ${version}`)) throw new Error(`prune.exe dice "${said}" pero se publica la ${version}: subí la versión en Cargo.toml.`);

  const dist = join(ROOT, "target/dist");
  rmSync(dist, { recursive: true, force: true });
  mkdirSync(dist, { recursive: true });
  copyFileSync(built, join(dist, "prune.exe"));
  copyFileSync(join(ROOT, "LICENSE"), join(dist, "LICENSE"));
  copyFileSync(join(ROOT, "README.md"), join(dist, "README.md"));

  // El tar que trae Windows (bsdtar) arma zips con -a; el de Git Bash no, por eso va con la ruta.
  const zipName = `P-Rune-Core_${version}_x64.zip`;
  const tar = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
  execFileSync(tar, ["-a", "-c", "-f", zipName, "prune.exe", "LICENSE", "README.md"], { cwd: dist, stdio: "inherit" });

  return { msi: buildMsi(dist, version), zip: join(dist, zipName) };
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const { msi, zip } = buildPackage();
  for (const file of [msi, zip]) {
    console.log(`${sha256(file)}  ${(readFileSync(file).length / 1048576).toFixed(2)} MB  ${file}`);
  }
}
