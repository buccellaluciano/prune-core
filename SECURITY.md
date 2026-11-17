# Seguridad

P-Rune Core modifica el sistema y, para muchos de sus cambios, corre como administrador, así
que cualquier problema de seguridad se toma en serio.

## Cómo reportar una vulnerabilidad

**No abras un issue público.** Reportala en privado desde la pestaña **Security → Report a
vulnerability** de este repositorio (reporte privado de vulnerabilidades de GitHub).

Incluí, si podés:

- qué parte está afectada (ajustes, historial, winget, DNS, limpieza, el PATH, el instalador…);
- los pasos para reproducirlo y qué puede lograr alguien que lo aproveche;
- la versión de P-Rune Core (`prune --version`) y de Windows.

Vas a recibir una respuesta lo antes posible. Cuando haya un arreglo publicado, se puede dar
crédito en las notas de la versión si lo querés.

## Versiones con soporte

Solo la última versión publicada recibe arreglos de seguridad.

## Qué está dentro del alcance

- Escalada de privilegios a través de P-Rune Core (por ejemplo, que un programa sin admin
  logre que Core escriba en el registro o ejecute algo como administrador).
- Problemas en la verificación de winget, en el historial, en la edición del PATH o en el
  instalador.

El motor es el mismo que usa [P-Rune](https://github.com/buccellaluciano/prune), la app de
escritorio: si el problema está ahí, es probable que afecte a las dos.
