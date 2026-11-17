// Arma una versión de P-Rune Core y, con --publish, crea el release en GitHub:
//   1. compila y empaqueta el MSI y el zip (scripts/package.mjs),
//   2. escribe el texto del release: las notas, las descargas y los SHA-256,
//   3. con --publish, crea el tag v<versión> y sube los dos archivos.
//
// Uso:
//   node scripts/release.mjs --notes docs/releases/0.1.0.md               # solo arma todo
//   node scripts/release.mjs --notes docs/releases/0.1.0.md --publish     # además publica
//   ... --publish --prerelease    # como versión preliminar (no queda como "Latest")
//
// La versión es la de Cargo.toml. Antes de publicar: todo commiteado y pusheado, porque el
// tag se crea sobre la rama principal de GitHub.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { ROOT, buildPackage, sha256, workspaceVersion } from "./package.mjs";

const REPO = "buccellaluciano/prune-core";

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const fail = (msg) => {
  console.error(`\n✗ ${msg}`);
  process.exit(1);
};

const notesFile = opt("--notes");
if (!notesFile || !existsSync(notesFile)) fail("Falta --notes <archivo.md> con las notas de la versión.");
const publish = args.includes("--publish");
const prerelease = args.includes("--prerelease");
const version = workspaceVersion();

if (publish) {
  // Lo que se publica tiene que ser lo que está en GitHub.
  const git = (...a) => execFileSync("git", a, { cwd: ROOT, encoding: "utf8" }).trim();
  if (git("status", "--porcelain")) fail("Hay cambios sin commitear: commiteá antes de publicar.");
  git("fetch", "--quiet", "origin");
  if (git("rev-parse", "HEAD") !== git("rev-parse", "@{upstream}")) fail("La rama local no coincide con la de GitHub: pusheá antes de publicar.");
}

console.log(`Armando P-Rune Core ${version}…`);
const { msi, zip } = buildPackage(version);
const files = [msi, zip];
const names = files.map((f) => basename(f));
const hashes = files.map((f) => `${sha256(f)}  ${basename(f)}`);

const notes = readFileSync(notesFile, "utf8").trim();
const body = `${notes}

## Descargas

| Archivo | Para qué |
|---|---|
| **\`${names[0]}\`** | Instalador. Lo deja listo para escribir \`prune\` en cualquier terminal. |
| \`${names[1]}\` | Sin instalar: el exe solo. \`.\\prune.exe path add\` lo deja en el PATH. |

> [!WARNING]
> P-Rune Core modifica el registro de Windows, quita apps preinstaladas e instala programas. Revisá el
> plan antes de aplicar cada cambio. Se distribuye sin garantía.
>
> Los archivos no están firmados con un certificado de código: Windows SmartScreen puede mostrar
> "editor desconocido".

## Verificar la descarga (SHA-256)

\`\`\`
${hashes.join("\n")}
\`\`\`
`;
const bodyFile = join(ROOT, "target/dist/release-body.md");
writeFileSync(bodyFile, body);
console.log(`\n✓ Listo en ${join(ROOT, "target/dist")}`);
for (const h of hashes) console.log(`  ${h}`);

if (publish) {
  console.log(`\nPublicando v${version} en ${REPO}…`);
  const flags = prerelease ? ["--prerelease"] : ["--latest"];
  execFileSync("gh", ["release", "create", `v${version}`, ...files, "-R", REPO, "--title", `P-Rune Core ${version}`, "--notes-file", bodyFile, ...flags], { stdio: "inherit" });
}
