# Revisión de herramientas de nbedit

Revisión del código local del CLI, biblioteca y MCP. Las observaciones siguientes
son hallazgos de lectura; salvo los rangos de edición y las funciones nuevas,
no se han corregido en este cambio.

## Orden recomendado

1. Corregir pérdida de integridad: IDs en `create`/`copy`, límites de `split`,
   separadores en `merge` y referencias de bookmarks al renombrar o borrar.
2. Unificar resultados y validación en la biblioteca para que CLI y MCP tengan
   exactamente el mismo comportamiento, incluyendo las funciones estructurales.
3. Extender transacciones a todos los comandos mutadores, con previsualización
   del diff y detección de modificaciones concurrentes antes de guardar.
4. Mejorar selecciones por identidad, diagnósticos y observabilidad de ejecución.

## Inventario y propuestas

| Herramienta | Hallazgo o limitación actual | Mejora propuesta |
|---|---|---|
| `read` | `--json` con varias celdas imprime objetos consecutivos; `--only-outputs` no aplica el mismo límite de líneas que la salida normal. | Un array JSON válido o NDJSON explícito; aplicar límites uniformes y mostrar IDs y números de línea opcionales. |
| `info`, `ids` | La información útil para seleccionar celdas está repartida. | `info --json`, conteos de tamaño, tags, ID, bookmark y estado de outputs en un resumen común. |
| `search` | La búsqueda es textual. | Modo literal, contexto configurable, selección previa y filtros por tags/ID; resultados JSON comunes con celda, ID, línea y columna. |
| `query` | Busca regex sobre JSON serializado; una coincidencia de metadata global se imprime con índice `null`. | Consultas por JSON Pointer y valores tipados; mostrar ámbito global sin índice ficticio. |
| `create` | El CLI inserta `Cell::new` sin asignar ID; MCP sí llama a `ensure_cell_ids`. | Generar ID único para la nueva celda en una operación compartida, sin modificar IDs ajenos. |
| `edit` | El selector de líneas usaba mensajes de celdas; corregido. CLI y MCP todavía tienen rutas distintas para reemplazo completo. | Compartir la operación completa; aceptar ID/bookmark, precondición de contenido y diff del cambio. |
| `wrap` | Valida la sintaxis resultante, pero no prueba equivalencia semántica. Envolver cambia el ámbito de docstrings de módulo. | Comparar AST del cuerpo, detectar construcciones incompatibles y permitir política de envoltura idempotente para evitar anidación accidental. |
| `indent` | Agrega espacios; puede producir un estado intermedio inválido si no se exige validación. | `dedent`, sangría relativa/objetivo y diagnóstico de rangos que cortan un bloque sintáctico. |
| `apply` | Transacción de operaciones de fuente en un notebook. | Incluir creación, movimiento, metadata y borrado de celdas; vista de diff y precondiciones por hash. |
| `replace` | El preview usa `zip` entre líneas viejas y nuevas y puede omitir líneas agregadas/eliminadas. Usa `process::exit` desde la biblioteca. | Diff real, reemplazo literal, conteo esperado de coincidencias, validación final y retorno tipado. |
| `delete` | Elimina celdas sin revisar bookmarks que apuntan a sus IDs. | Informar y actualizar referencias internas; borrado por ID y preview de referencias afectadas. |
| `move` | Calcula destino y lo limita al número de celdas restantes; el mensaje puede mostrar una posición distinta de la inserción efectiva. | Definir `--before`/`--after` por ID y reportar el orden final exacto. |
| `copy` | Copia IDs originales sin resolver colisiones en el destino. | Mantener IDs solo si son únicos; generar nuevos para colisiones y devolver mapa origen/destino. |
| `duplicate` | Ya genera IDs nuevos; conserva outputs del original. | Política explícita para outputs y ejecución, con mapa de IDs y selección por identidad. |
| `merge` | Concatena fuentes sin separador: `x = 1` y `y = 2` pueden convertirse en `x = 1y = 2`. Conserva metadata solo de la primera celda. | Separador seguro, validación Python y política de combinación de metadata/attachments y bookmarks. |
| `split` | La ayuda dice «antes de la línea», pero el código corta después de N líneas; rechaza cortar antes de la última línea y normaliza saltos. | Alinear contrato e implementación, preservar terminadores y evitar dividir cadenas o bloques sin advertirlo en el resultado. |
| `clear`, `strip` | `strip --notebook-metadata` borra también kernelspec y bookmarks; el preview no detalla los campos. | Strip selectivo por JSON Pointer, conteos antes/después y conservar metadata operativa por política configurable. |
| `validate` | La validación estructural revisa tipo e IDs presentes/duplicados; no es validación completa de nbformat. | Validar versión, formato de IDs, metadata objeto, outputs, execution_count y campos obligatorios; diagnósticos estructurados y reglas por versión. |
| `repair` | Corrige algunos IDs y metadata, pero no ofrece plan detallado de reparaciones. | Preview por campo, reparaciones seleccionables y rechazo de cambios ambiguos. |
| `lint` | Presencia de nodos no garantiza que `logger.error` esté en el handler ni que `raise` relance la excepción. | Reglas de relación: cobertura por try, handler correcto, `exc_info=True`, bare raise dentro del handler; severidades y exclusiones por celda/tag. |
| `bookmark`, `rename-id`, `refs` | Renombrar no actualiza bookmarks; `refs` solo busca texto en otras fuentes. | Actualizar referencias internas de forma atómica; distinguir referencias estructurales de coincidencias textuales. |
| `diff` | Tabla LCS con memoria cuadrática; no detecta identidad/movimientos y termina el proceso desde biblioteca. | Diff por ID, movimientos, JSON y opción de outputs/metadata; algoritmo acotado y retorno tipado. |
| `export` | Exporta una sola celda. | Exportar selecciones con separadores `# %%`, mapa de origen e importación con precondiciones. |
| `render` | Usa `python3` por defecto, diferente de la resolución de `run`; cualquier fallo recomienda instalar nbconvert. | Resolución compartida, distinguir dependencias de fallos del documento y escribir salida atómicamente. |
| `kernels` | Descubrimiento y comprobación pueden repetirse entre comandos. | `doctor` con diagnóstico de driver/kernel/dependencias y caché invalidable de comprobaciones. |
| `run` | La ejecución puede durar mientras otra herramienta modifica el notebook. | Detectar conflictos antes de persistir outputs; selección por ID, informe por celda y modo de no persistir. |
| `session` | El RPC de ejecución es una respuesta bloqueante; no transporta progreso por celda. | Protocolo de eventos, cancelación e interrupción reales, reinicio y estado ocupado/última ejecución. |
| MCP | Repite validación y parte de la edición; carece de las nuevas operaciones estructurales y de herramientas dedicadas para algunos comandos CLI. | Operaciones de biblioteca que retornen datos sin imprimir, paridad CLI/MCP y contratos probados. Conservar el confinamiento canónico de rutas en toda nueva operación. |

## Criterios de aceptación transversales

- Un error incluye ámbito, celda/ID y línea/columna cuando corresponde.
- Un resultado JSON siempre es parseable como el formato anunciado.
- Un dry run no modifica el notebook ni crea o reemplaza backups.
- Un fallo de validación no persiste cambios parciales.
- Un guardado rechaza conflictos si el original cambió desde la lectura.
- Las pruebas cubren Unicode, CRLF, notebooks vacíos, IDs duplicados, cadenas
  multilínea, outputs grandes y equivalencia de contratos CLI/MCP.
