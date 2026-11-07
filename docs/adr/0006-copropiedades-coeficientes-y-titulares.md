# ADR 0006 · Copropiedades, coeficientes versionados, titulares e importación masiva

- **Estado:** aceptada
- **Fecha:** 2026-11-07

## Contexto
Cada propiedad horizontal (Ley 675 de 2001) tiene datos legales, una estructura (torres,
etapas, bloques), bienes privados con **coeficiente de copropiedad** y sus titulares. Los
coeficientes solo cambian por reforma del reglamento (escritura pública), deben sumar
exactamente 1 y las votaciones pesan por ellos (AGENTS.md §9.5). Las PH llegan con cientos de
bienes en hojas de cálculo.

## Decisión
- **Ficha de la PH en `copropiedades`** (NIT con dígito de verificación DIAN, tipo, forma,
  ubicación, datos de la escritura y del reglamento, zona horaria) con máquina de estados
  `en_configuracion → activa ⇄ suspendida → retirada`, que maneja la plataforma. Los cargos
  (administrador, contador, revisor fiscal, consejo) **no son columnas**: son asignaciones de rol
  con vigencia (ADR 0005) y se leen con la función `cargos_vigentes()`. La ficha advierte si
  falta revisor fiscal (art. 56) o consejo (art. 53) en PH comerciales o mixtas.
- **Nomenclatura**: cada bien tiene número y agrupación; su código único por PH se calcula con
  una plantilla de la PH (`{agrupacion}`, `{ruta}`, `{numero}`: `T3-506`, `3506`…) o lo escribe
  la PH. Cambiar la plantilla no recalcula códigos existentes.
- **Coeficientes versionados**: `cuadros_coeficientes` (`borrador → vigente → historico`) y
  `coeficientes_bien_privado`. Activar exige suma **exacta** = 1 con `Decimal`, todos los bienes
  activos y ninguna reunión instalada; se valida en el dominio y lo repiten triggers. Un índice
  único parcial deja un solo cuadro vigente. `bienes_privados.coeficiente` es una copia del
  vigente que **solo** escribe la activación (trigger), así las votaciones no cambian. El
  soporte documental de la reforma es **opcional**. La herramienta de revisión muestra la
  diferencia y sugiere dónde ajustar; el ajuste es siempre humano.
- **Parqueaderos y depósitos**: con folio propio son bienes privados con coeficiente y pueden
  vincularse como **anexos** de su apartamento (`bien_principal_id`); englobados con el
  apartamento no se registran (su coeficiente va en el del apartamento); comunes asignados son
  **usos exclusivos** (art. 22, sin coeficiente). La PH usa los modelos que necesite.
- **Titulares**: `titularidades` con vigencia, tipo (propietario, nudo propietario,
  usufructuario, fideicomiso, leasing) y porcentaje. Los vigentes suman 100 % por clase de
  derecho (dominio y goce), con exactamente un representante ante la PH; una transferencia
  termina el conjunto anterior y crea el nuevo en una transacción (trigger diferido). Cada
  titular con cuenta recibe el rol derivado `propietario` (origen `propiedad`), que vence con la
  titularidad. `personas.cuenta_id` vincula a la persona de la PH con su cuenta global.
- **Importación masiva** genérica: archivo CSV/XLSX → validación fila por fila → vista previa
  con errores → confirmar → aplicar en una transacción (`importaciones`). Lectura con `csv` y
  `calamine` (Rust puro, solo lectura) en el crate `infra-planillas` detrás del puerto
  `LectorPlanillas`. Las planillas solo se aceptan por este camino (no como adjuntos), se guardan
  como archivos `personal` y los errores nunca repiten el valor de la celda. Los coeficientes
  importados quedan en un cuadro **borrador**: activarlo sigue siendo una decisión aparte.
- **Módulos de contribución** (art. 29, conjuntos mixtos): solo se deja la columna
  `cuadros_coeficientes.tipo` prevista para el módulo de contabilidad; hoy todo cuadro es de
  copropiedad.

## Consecuencias
- Un bien privado nuevo no tiene coeficiente hasta que se activa un cuadro que lo incluya, y sin
  coeficiente no puede votar.
- Inactivar un bien no cambia el cuadro vigente: el siguiente cuadro ya no lo incluye.
- La precisión `NUMERIC(10,8)` (8 decimales en fracción, 6 en porcentaje) alcanza para los
  reglamentos conocidos.
- Horarios: el componente `domain::Horario` (franjas semanales, festivos, excepciones) se
  reutiliza en el plan 04 (bienes comunes).
