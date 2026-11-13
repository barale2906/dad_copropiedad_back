# ADR 0017 · Núcleo contable genérico

- **Estado:** aceptada
- **Fecha:** 2026-11-13
- **Plan:** `docs/planes/12-contabilidad-nucleo.md` (fases 2–7); análisis y decisiones en el plan 11

## Contexto
La plataforma venderá contabilidad a cualquier empresa (PH, gimnasios, tiendas) y las verticales
la usarán para sus documentos (cartera de expensas, facturas). La especificación funcional exige
partida doble exacta, inmutabilidad con reversos, periodos con reapertura auditada, normativa en
datos versionados y no en el código, y trazabilidad desde el documento origen hasta los estados
financieros. El usuario decidió que el contador elige toda variable contable con plantillas
editables (P1) y que se carga el PUC completo.

## Decisión
- **Módulo `contabilidad`** para cualquier entidad; rutas `/entidades/{entidad_id}/contabilidad`.
  El núcleo (`domain::contabilidad`, `application::casos_uso::contabilidad`,
  `puertos::contabilidad`) **no importa nada de la PH**: dos pruebas de fronteras leen los
  fuentes y fallan si aparece una ruta prohibida.
- **Partida doble como tipo**: solo se contabiliza un `AsientoBalanceado` (≥ 2 líneas, una sola
  cara por línea, débitos = créditos exacto, `Monto` con dos decimales, nunca flotantes) validado
  contra el plan de cuentas (cuenta activa y de movimiento, tercero y centro de costo cuando la
  cuenta los exige). La base de datos lo vuelve a exigir: trigger diferido de partida doble al
  confirmar, trigger de contabilización con el periodo `abierto` (`FOR SHARE`), la fecha dentro del
  periodo y las cuentas.
- **Inmutabilidad**: un comprobante contabilizado y sus líneas no se modifican ni se borran
  (triggers); se corrige con un reverso (`revierte_a`, motivo, un solo reverso). Los borradores se
  editan y se previsualizan.
- **Numeración** por tipo de comprobante y año en la tabla `consecutivos` existente (serie
  `comprobante:<código>`); se asigna al contabilizar y los huecos se permiten (§5.5).
- **Periodos** por mes (13 = cierre anual), creados al primer uso; estados con
  `EstadoPeriodo::transicionar`. Reabrir uno cerrado exige una solicitud con motivo que aprueba
  **otra cuenta** (segregación de funciones; también un `CHECK` en la BD).
- **Contabilizador**: los módulos no escriben asientos; entregan un `DocumentoOrigen` (evento,
  origen, fecha, importes por concepto) a `contabilizar_documento` dentro de **su** transacción.
  La regla vigente del evento en la fecha (`reglas_contabilizacion`, versionadas) arma el asiento.
  Idempotente por origen (índice único).
- **Terceros = personas** del tenant con columnas fiscales nuevas (régimen de IVA,
  responsabilidades del RUT, CIIU, dirección y municipio fiscal, correo de facturación,
  `organizacion_id`); no hay tabla `terceros`. Crear un tercero con un documento existente
  reutiliza la persona.
- **Normativa como datos**: `catalogo_puc` (plataforma; clases y grupos sembrados, el resto se
  carga con `PUT /plataforma/puc`) y **una sola tabla** `plantillas_contables` (JSONB tipado como
  `PlantillaContable`, ámbito plataforma o firma contable, versionada, inmutable al publicarse).
  Aplicar una plantilla copia a la entidad lo que falta (con vista previa que no cambia nada);
  la entidad puede exportar su configuración como plantilla nueva.
- **Cierre anual**: comprobante del sistema en el periodo 13 que lleva a cero las cuentas de
  resultado (clases 4 a 7) contra la cuenta de excedente o déficit de la configuración; exige los
  meses cerrados y un solo cierre vigente.
- **Reportes**: balance de prueba (por cuenta, tercero y centro), libro diario, mayor y
  auxiliares, estados financieros por rubros que elige el contador (signo natural del rubro: las
  correctoras restan) y notas versionadas. Exportación CSV con su huella SHA-256 en la auditoría
  (QA-017), sin guardar el archivo: lo contabilizado no cambia y el mismo filtro lo reproduce.
- **Código de error** `regla_contable` (422) para lo que incumple estas reglas.

## Consecuencias
- Los saldos se calculan desde las líneas con índices; si el volumen lo pide, se agregan saldos
  por periodo al cerrar (no se hizo: YAGNI).
- No hay XLSX ni PDF de libros todavía (CSV basta para el revisor fiscal y el contador); los PDF
  saldrán con `infra-pdf` cuando se pidan.
- Los saldos iniciales se cargan por API (JSON); la importación de planillas para ellos se agrega
  sobre `importaciones` cuando haga falta.
- Las verticales (plan 14) y la facturación (planes 13 y 15) se montan sobre el contabilizador y
  las reglas sin tocar el núcleo.
- Pendiente operativo: cargar el PUC completo y que el contador de la plataforma arme y publique
  la plantilla estándar PH.
